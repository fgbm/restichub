//! Converts a profile to typed steps (FR-PF-04, FR-PF-05).

use restichub_proto::agent::v1::{
    BackupStep, CheckStep, ForgetGroupBy, ForgetStep, Job, PruneStep, RepositoryAccess,
    SnapshotsFilter, Step, step,
};

use crate::error::{CompileError, FieldPath};
use crate::option::ResticOption;
use crate::profile::{CheckOptions, GroupBy, HookPhase, JobLimits, Profile, Retention};
use crate::target::{OperatingSystem, ResticVersion, TargetFacts};
use crate::template::{self, TemplateError};

/// The prefix of the tags that only the control plane sets (FR-RP-08).
const RESERVED_TAG_PREFIX: &str = "restichub-";

/// The steps of one target agent, in execution order (FR-PF-04).
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub steps: Vec<PlannedStep>,
    pub limits: JobLimits,
}

/// One step and whether only the maintenance agent runs it (FR-RP-08).
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedStep {
    pub step: Step,
    /// True for `forget`, `prune` and `check` (FR-RP-08).
    pub maintenance: bool,
}

/// Converts a profile to a plan for one target agent (FR-PF-04).
///
/// The step order is the `before` scripts, the `backup` step, the remaining
/// scripts, then the maintenance steps `forget`, `prune` and `check`.
///
/// # Errors
///
/// Returns a [`CompileError`] for an unsupported restic version, an option
/// that the operating system or the restic version does not allow, an invalid value or a template that does not expand.
pub fn compile(profile: &Profile, target: &TargetFacts) -> Result<Plan, CompileError> {
    if target.restic_version < ResticVersion::MIN_SUPPORTED {
        return Err(CompileError::UnsupportedRestic {
            installed: target.restic_version,
            required: ResticVersion::MIN_SUPPORTED,
        });
    }
    validate_platform(profile, target)?;
    validate_scripts(profile, target)?;

    let steps = build_steps(profile, target)?;
    check_contract(profile, &steps)?;

    Ok(Plan {
        steps,
        limits: profile.limits,
    })
}

fn validate_platform(profile: &Profile, target: &TargetFacts) -> Result<(), CompileError> {
    if profile.options.use_fs_snapshot && target.os != OperatingSystem::Windows {
        return Err(CompileError::UnsupportedOs {
            field: FieldPath::new("options.use_fs_snapshot"),
            option: ResticOption::UseFsSnapshot,
            os: target.os,
        });
    }

    for (option, field) in emitted_options(profile) {
        let required = option.minimum_version();
        if target.restic_version < required {
            return Err(CompileError::IncompatibleOption {
                field,
                option,
                required,
                installed: target.restic_version,
            });
        }
    }

    Ok(())
}

fn emitted_options(profile: &Profile) -> Vec<(ResticOption, FieldPath)> {
    let mut options = Vec::new();
    let backup = &profile.options;
    let flags = [
        (backup.use_fs_snapshot, ResticOption::UseFsSnapshot),
        (backup.one_file_system, ResticOption::OneFileSystem),
        (backup.exclude_caches, ResticOption::ExcludeCaches),
        (backup.limit_upload_kib > 0, ResticOption::LimitUpload),
        (backup.limit_download_kib > 0, ResticOption::LimitDownload),
    ];
    for (present, option) in flags {
        if present {
            options.push((
                option,
                FieldPath::new(format!("options.{}", option_name(option))),
            ));
        }
    }
    // The control plane always sends --retry-lock (FR-SC-05).
    options.push((ResticOption::RetryLock, FieldPath::new("options")));

    if let Some(retention) = &profile.retention {
        let counts = [
            (retention.keep_last, ResticOption::KeepLast, "keep_last"),
            (
                retention.keep_hourly,
                ResticOption::KeepHourly,
                "keep_hourly",
            ),
            (retention.keep_daily, ResticOption::KeepDaily, "keep_daily"),
            (
                retention.keep_weekly,
                ResticOption::KeepWeekly,
                "keep_weekly",
            ),
            (
                retention.keep_monthly,
                ResticOption::KeepMonthly,
                "keep_monthly",
            ),
            (
                retention.keep_yearly,
                ResticOption::KeepYearly,
                "keep_yearly",
            ),
        ];
        for (count, option, name) in counts {
            if count > 0 {
                options.push((option, FieldPath::new(format!("retention.{name}"))));
            }
        }
        if !retention.keep_within.is_empty() {
            options.push((
                ResticOption::KeepWithin,
                FieldPath::new("retention.keep_within"),
            ));
        }
        if !retention.keep_tags.is_empty() {
            options.push((
                ResticOption::KeepTags,
                FieldPath::new("retention.keep_tags"),
            ));
        }
    }

    if let Some(check) = &profile.check
        && !check.read_data_subset.is_empty()
    {
        options.push((
            ResticOption::ReadDataSubset,
            FieldPath::new("check.read_data_subset"),
        ));
    }

    options
}

fn option_name(option: ResticOption) -> &'static str {
    match option {
        ResticOption::UseFsSnapshot => "use_fs_snapshot",
        ResticOption::OneFileSystem => "one_file_system",
        ResticOption::ExcludeCaches => "exclude_caches",
        ResticOption::LimitUpload => "limit_upload_kib",
        ResticOption::LimitDownload => "limit_download_kib",
        _ => "option",
    }
}

fn validate_scripts(profile: &Profile, target: &TargetFacts) -> Result<(), CompileError> {
    for (index, script) in profile.scripts.iter().enumerate() {
        crate::script::check_script(index, script, target)?;
    }
    Ok(())
}

fn build_steps(profile: &Profile, target: &TargetFacts) -> Result<Vec<PlannedStep>, CompileError> {
    let mut steps = Vec::new();

    push_scripts(&mut steps, profile, HookPhase::Before);

    steps.push(PlannedStep {
        step: backup_step(profile, target)?,
        maintenance: false,
    });

    // The agent derives the order from the phase, not from the position (FR-HK-05). The listed
    // order is the order of a run, so the preview reads like the job.
    for phase in [
        HookPhase::After,
        HookPhase::OnSuccess,
        HookPhase::OnError,
        HookPhase::Finally,
    ] {
        push_scripts(&mut steps, profile, phase);
    }

    if let Some(retention) = &profile.retention {
        steps.push(PlannedStep {
            step: forget_step(profile, retention, target)?,
            maintenance: true,
        });
        steps.push(PlannedStep {
            step: prune_step(profile),
            maintenance: true,
        });
    }

    if let Some(check) = &profile.check {
        steps.push(PlannedStep {
            step: check_step(profile, check, target)?,
            maintenance: true,
        });
    }

    Ok(steps)
}

/// Adds the scripts of one phase, in the order of the profile (FR-HK-09).
fn push_scripts(steps: &mut Vec<PlannedStep>, profile: &Profile, phase: HookPhase) {
    for (index, script) in profile.scripts.iter().enumerate() {
        if script.phase == phase {
            steps.push(PlannedStep {
                step: crate::script::script_step(index, script),
                maintenance: false,
            });
        }
    }
}

fn backup_step(profile: &Profile, target: &TargetFacts) -> Result<Step, CompileError> {
    let mut paths = Vec::new();
    for (index, path) in profile.paths.all.iter().enumerate() {
        paths.push(expand_field(path, target, &format!("paths.all[{index}]"))?);
    }
    if let Some(extra) = profile.paths.by_os.get(&target.os) {
        for (index, path) in extra.iter().enumerate() {
            paths.push(expand_field(
                path,
                target,
                &format!("paths.by_os[{index}]"),
            )?);
        }
    }
    if paths.is_empty() {
        return Err(CompileError::NoPaths { os: target.os });
    }

    let mut excludes = Vec::new();
    for (index, exclude) in profile.excludes.iter().enumerate() {
        excludes.push(expand_field(
            exclude,
            target,
            &format!("excludes[{index}]"),
        )?);
    }
    let mut exclude_files = Vec::new();
    for (index, file) in profile.exclude_files.iter().enumerate() {
        exclude_files.push(expand_field(
            file,
            target,
            &format!("exclude_files[{index}]"),
        )?);
    }
    let mut tags = Vec::new();
    for (index, tag) in profile.tags.iter().enumerate() {
        tags.push(expand_tag(tag, target, &format!("tags[{index}]"))?);
    }
    tags.extend(profile.profile_tag.iter().cloned());

    let options = &profile.options;
    Ok(Step {
        step_id: "backup".to_owned(),
        timeout_seconds: 0,
        kind: Some(step::Kind::Backup(BackupStep {
            repository_id: profile.repository_id.clone(),
            paths,
            excludes,
            exclude_files,
            tags,
            use_fs_snapshot: options.use_fs_snapshot,
            one_file_system: options.one_file_system,
            exclude_caches: options.exclude_caches,
            limit_upload_kib: options.limit_upload_kib,
            limit_download_kib: options.limit_download_kib,
            retry_lock: true,
            priority: 0,
        })),
    })
}

fn forget_step(
    profile: &Profile,
    retention: &Retention,
    target: &TargetFacts,
) -> Result<Step, CompileError> {
    let keep_within = expand_field(&retention.keep_within, target, "retention.keep_within")?;

    let mut keep_tags = Vec::new();
    for (index, tag) in retention.keep_tags.iter().enumerate() {
        keep_tags.push(expand_tag(
            tag,
            target,
            &format!("retention.keep_tags[{index}]"),
        )?);
    }

    let mut filter = SnapshotsFilter::default();
    for (index, host) in retention.filter.hosts.iter().enumerate() {
        filter.hosts.push(expand_field(
            host,
            target,
            &format!("retention.filter.hosts[{index}]"),
        )?);
    }
    // The snapshot needs the profile tag and every tag of the filter (FR-RP-08, ADR-0034).
    filter.all_tags.extend(profile.profile_tag.iter().cloned());
    for (index, tag) in retention.filter.tags.iter().enumerate() {
        filter.all_tags.push(expand_tag(
            tag,
            target,
            &format!("retention.filter.tags[{index}]"),
        )?);
    }
    for (index, path) in retention.filter.paths.iter().enumerate() {
        filter.paths.push(expand_field(
            path,
            target,
            &format!("retention.filter.paths[{index}]"),
        )?);
    }

    Ok(Step {
        step_id: "forget".to_owned(),
        timeout_seconds: 0,
        kind: Some(step::Kind::Forget(ForgetStep {
            repository_id: profile.repository_id.clone(),
            keep_last: retention.keep_last,
            keep_hourly: retention.keep_hourly,
            keep_daily: retention.keep_daily,
            keep_weekly: retention.keep_weekly,
            keep_monthly: retention.keep_monthly,
            keep_yearly: retention.keep_yearly,
            keep_within,
            keep_tags,
            group_by: retention
                .group_by
                .iter()
                .copied()
                .map(group_by_flag)
                .collect(),
            filter: Some(filter),
            retry_lock: true,
            priority: 0,
        })),
    })
}

fn prune_step(profile: &Profile) -> Step {
    Step {
        step_id: "prune".to_owned(),
        timeout_seconds: 0,
        kind: Some(step::Kind::Prune(PruneStep {
            repository_id: profile.repository_id.clone(),
            max_unused: String::new(),
            max_repack_size: String::new(),
            retry_lock: true,
            priority: 0,
        })),
    }
}

fn check_step(
    profile: &Profile,
    check: &CheckOptions,
    target: &TargetFacts,
) -> Result<Step, CompileError> {
    Ok(Step {
        step_id: "check".to_owned(),
        timeout_seconds: 0,
        kind: Some(step::Kind::Check(CheckStep {
            repository_id: profile.repository_id.clone(),
            read_data_subset: expand_field(
                &check.read_data_subset,
                target,
                "check.read_data_subset",
            )?,
            retry_lock: true,
            priority: 0,
        })),
    })
}

fn group_by_flag(group: GroupBy) -> i32 {
    match group {
        GroupBy::Host => ForgetGroupBy::Host as i32,
        GroupBy::Paths => ForgetGroupBy::Paths as i32,
        GroupBy::Tags => ForgetGroupBy::Tags as i32,
    }
}

fn expand_field(value: &str, target: &TargetFacts, field: &str) -> Result<String, CompileError> {
    let expanded = template::expand(value, target).map_err(|error| template_error(error, field))?;
    if restichub_proto::validate::has_forbidden(&expanded) {
        return Err(CompileError::InvalidValue {
            field: FieldPath::new(field),
        });
    }
    Ok(expanded)
}

/// Expands a tag and rejects the forms that restic reads differently: an empty
/// tag, a comma and surrounding space (AC-PR-08). A template may expand to the
/// reserved prefix, so the check runs after the expansion (FR-RP-08).
fn expand_tag(value: &str, target: &TargetFacts, field: &str) -> Result<String, CompileError> {
    let expanded = expand_field(value, target, field)?;
    if expanded.is_empty()
        || expanded.contains(',')
        || expanded.trim() != expanded
        || expanded
            .to_ascii_lowercase()
            .starts_with(RESERVED_TAG_PREFIX)
    {
        return Err(CompileError::InvalidValue {
            field: FieldPath::new(field),
        });
    }
    Ok(expanded)
}

fn template_error(error: TemplateError, field: &str) -> CompileError {
    let field = FieldPath::new(field);
    match error {
        TemplateError::UnknownVariable { name } => CompileError::UnknownVariable { field, name },
        TemplateError::MissingLabel { key } => CompileError::MissingLabel { field, key },
        TemplateError::InvalidDate { format } => CompileError::InvalidDate { field, format },
        TemplateError::Unterminated => CompileError::UnterminatedTemplate { field },
    }
}

fn check_contract(profile: &Profile, steps: &[PlannedStep]) -> Result<(), CompileError> {
    // The profile holds the repository identifier, not its location. The
    // control plane validates the real location when it assembles the job, so
    // this placeholder only satisfies the shared contract.
    let job = Job {
        job_id: "compile".to_owned(),
        tenant_id: "compile".to_owned(),
        agent_id: "compile".to_owned(),
        repositories: vec![RepositoryAccess {
            repository_id: profile.repository_id.clone(),
            location: "/".to_owned(),
            ..RepositoryAccess::default()
        }],
        steps: steps.iter().map(|planned| planned.step.clone()).collect(),
        ..Job::default()
    };
    crate::job::ValidatedJob::new(job)
        .map(|_| ())
        .map_err(|rejection| CompileError::StepsRejected {
            reason: rejection.reason,
        })
}

#[cfg(test)]
mod tests;
