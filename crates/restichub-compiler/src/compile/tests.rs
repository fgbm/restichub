use std::collections::BTreeMap;

use proptest::prelude::*;
use restichub_proto::agent::v1::{Job, RepositoryAccess, step};
use time::{Date, Month, OffsetDateTime};

use super::{Plan, compile};
use crate::arg;
use crate::error::CompileError;
use crate::job::ValidatedJob;
use crate::option::ResticOption;
use crate::preview::preview;
use crate::profile::{
    BackupOptions, CheckOptions, HookParameter, HookPhase, HookRef, Profile, ProfilePaths,
    Retention, SnapshotFilter,
};
use crate::target::{
    AllowListedHook, Architecture, HookParameterSpec, OperatingSystem, ResticVersion, TargetFacts,
};

const BAD_OPTION: &str = "--password-command=cmd /c calc";
const BAD_SHORT: &str = "-x";
const PROFILE_TAG: &str = "restichub-profile-test";

fn date(year: i32, month: u8, day: u8) -> OffsetDateTime {
    let month = Month::try_from(month).unwrap_or(Month::January);
    match Date::from_calendar_date(year, month, day) {
        Ok(value) => value.midnight().assume_utc(),
        Err(_) => panic!("invalid test date"),
    }
}

fn allow_list() -> BTreeMap<String, AllowListedHook> {
    let mut allow_list = BTreeMap::new();
    allow_list.insert(
        "pg_dump".to_owned(),
        AllowListedHook {
            parameters: BTreeMap::from([(
                "database".to_owned(),
                HookParameterSpec { secret: false },
            )]),
        },
    );
    allow_list.insert(
        "dump_secret".to_owned(),
        AllowListedHook {
            parameters: BTreeMap::from([(
                "password".to_owned(),
                HookParameterSpec { secret: true },
            )]),
        },
    );
    allow_list
}

fn target() -> TargetFacts {
    let mut labels = BTreeMap::new();
    labels.insert("env".to_owned(), "prod".to_owned());
    TargetFacts {
        agent_id: "agent-1".to_owned(),
        agent_name: "web-1".to_owned(),
        hostname: "host1".to_owned(),
        os: OperatingSystem::Linux,
        arch: Architecture::X86_64,
        labels,
        restic_version: ResticVersion::MIN_SUPPORTED,
        now: date(2026, 10, 13),
        allow_list: allow_list(),
    }
}

fn profile() -> Profile {
    Profile {
        repository_id: "repo-1".to_owned(),
        paths: ProfilePaths {
            all: vec!["/srv".to_owned()],
            by_os: BTreeMap::new(),
        },
        tags: vec!["daily".to_owned()],
        profile_tag: Some(PROFILE_TAG.to_owned()),
        ..Profile::default()
    }
}

fn repository() -> RepositoryAccess {
    RepositoryAccess {
        repository_id: "repo-1".to_owned(),
        location: "s3:s3.example.com/bucket".to_owned(),
        password: "topsecret".to_owned(),
        ..RepositoryAccess::default()
    }
}

fn plan(profile: &Profile, target: &TargetFacts) -> Plan {
    match compile(profile, target) {
        Ok(value) => value,
        Err(error) => panic!("{error:?}"),
    }
}

fn error(profile: &Profile, target: &TargetFacts) -> CompileError {
    match compile(profile, target) {
        Ok(value) => panic!("expected an error, got {value:?}"),
        Err(error) => error,
    }
}

fn validated(plan: &Plan, repository: &RepositoryAccess) -> ValidatedJob {
    let job = Job {
        job_id: "job-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        repositories: vec![repository.clone()],
        steps: plan
            .steps
            .iter()
            .map(|planned| planned.step.clone())
            .collect(),
        ..Job::default()
    };
    match ValidatedJob::new(job) {
        Ok(value) => value,
        Err(error) => panic!("{error:?}"),
    }
}

fn backup_paths(plan: &Plan) -> Vec<String> {
    for planned in &plan.steps {
        if let Some(step::Kind::Backup(backup)) = &planned.step.kind {
            return backup.paths.clone();
        }
    }
    panic!("no backup step");
}

fn hook(hook_id: &str, phase: HookPhase, parameters: &[(&str, &str)]) -> HookRef {
    HookRef {
        hook_id: hook_id.to_owned(),
        phase,
        parameters: parameters
            .iter()
            .map(|(name, value)| HookParameter {
                name: (*name).to_owned(),
                value: (*value).to_owned(),
            })
            .collect(),
    }
}

#[test]
fn backup_step_is_built_with_retry_lock() {
    let plan = plan(&profile(), &target());
    assert_eq!(plan.steps.len(), 1);
    let planned = &plan.steps[0];
    assert!(!planned.maintenance);
    match &planned.step.kind {
        Some(step::Kind::Backup(backup)) => {
            assert!(backup.retry_lock);
            assert_eq!(backup.repository_id, "repo-1");
            assert_eq!(backup.paths, vec!["/srv".to_owned()]);
        }
        other => panic!("unexpected step {other:?}"),
    }
}

#[test]
fn retention_and_check_add_maintenance_steps() {
    let mut profile = profile();
    profile.retention = Some(Retention::default());
    profile.check = Some(CheckOptions {
        read_data_subset: "10%".to_owned(),
    });
    let plan = plan(&profile, &target());
    assert_eq!(plan.steps.len(), 4);
    for planned in &plan.steps {
        match &planned.step.kind {
            Some(step::Kind::Backup(_)) | Some(step::Kind::Hook(_)) => {
                assert!(!planned.maintenance);
            }
            Some(step::Kind::Forget(_))
            | Some(step::Kind::Prune(_))
            | Some(step::Kind::Check(_)) => assert!(planned.maintenance),
            other => panic!("unexpected step {other:?}"),
        }
    }
}

fn forget_filter(plan: &Plan) -> restichub_proto::agent::v1::SnapshotsFilter {
    plan.steps
        .iter()
        .find_map(|planned| match &planned.step.kind {
            Some(step::Kind::Forget(forget)) => forget.filter.clone(),
            _ => None,
        })
        .expect("a forget step with a filter")
}

#[test]
fn fr_rp_08_the_forget_carries_the_profile_tag_and_the_filter_tags_as_one_and_list() {
    let mut profile = profile();
    profile.retention = Some(Retention {
        keep_last: 1,
        filter: SnapshotFilter {
            tags: vec!["daily".to_owned()],
            ..SnapshotFilter::default()
        },
        ..Retention::default()
    });
    let plan = plan(&profile, &target());
    let filter = forget_filter(&plan);
    assert_eq!(filter.all_tags, [PROFILE_TAG, "daily"]);
    assert!(filter.tags.is_empty());

    let preview = preview(&plan, &repository(), &allow_list()).expect("the preview");
    let forget = preview
        .iter()
        .find(|step| matches!(step.step.kind, Some(step::Kind::Forget(_))))
        .expect("a forget step");
    assert!(
        forget
            .command
            .contains(&format!("--tag={PROFILE_TAG},daily")),
        "{}",
        forget.command
    );
}

#[test]
fn fr_rp_08_the_forget_without_filter_tags_filters_on_the_profile_tag() {
    let mut profile = profile();
    profile.retention = Some(Retention {
        keep_last: 1,
        ..Retention::default()
    });
    assert_eq!(
        forget_filter(&plan(&profile, &target())).all_tags,
        [PROFILE_TAG]
    );

    profile.profile_tag = None;
    assert!(
        matches!(
            error(&profile, &target()),
            CompileError::StepsRejected { .. }
        ),
        "a forget without any tag is not a valid step"
    );
}

#[test]
fn fr_rp_08_a_tag_that_expands_to_the_reserved_prefix_fails_to_compile() {
    let mut facts = target();
    facts.agent_name = "ResticHub-profile-other".to_owned();
    let template = "{{agent.name}}";

    let mut tagged = profile();
    tagged.tags = vec![template.to_owned()];
    assert!(matches!(
        error(&tagged, &facts),
        CompileError::InvalidValue { .. }
    ));

    let mut filtered = profile();
    filtered.retention = Some(Retention {
        keep_last: 1,
        filter: SnapshotFilter {
            tags: vec![template.to_owned()],
            ..SnapshotFilter::default()
        },
        ..Retention::default()
    });
    assert!(matches!(
        error(&filtered, &facts),
        CompileError::InvalidValue { .. }
    ));

    let mut keep = profile();
    keep.retention = Some(Retention {
        keep_last: 1,
        keep_tags: vec![template.to_owned()],
        ..Retention::default()
    });
    assert!(matches!(
        error(&keep, &facts),
        CompileError::InvalidValue { .. }
    ));
}

#[test]
fn fr_rp_08_the_backup_carries_the_profile_tag() {
    let plan = plan(&profile(), &target());
    let Some(step::Kind::Backup(backup)) = &plan.steps[0].step.kind else {
        panic!("a backup step");
    };
    assert_eq!(backup.tags, ["daily", PROFILE_TAG]);
}

#[test]
fn before_hook_runs_before_the_backup() {
    let mut profile = profile();
    profile.hooks = vec![hook("pg_dump", HookPhase::Before, &[("database", "app")])];
    let plan = plan(&profile, &target());
    assert_eq!(plan.steps.len(), 2);
    assert!(matches!(plan.steps[0].step.kind, Some(step::Kind::Hook(_))));
    assert!(matches!(
        plan.steps[1].step.kind,
        Some(step::Kind::Backup(_))
    ));
}

#[test]
fn ac_pf_02_vss_on_linux_agent_is_rejected() {
    let mut profile = profile();
    profile.options = BackupOptions {
        use_fs_snapshot: true,
        ..BackupOptions::default()
    };
    let error = error(&profile, &target());
    assert!(matches!(
        error,
        CompileError::UnsupportedOs {
            option: ResticOption::UseFsSnapshot,
            os: OperatingSystem::Linux,
            ..
        }
    ));
}

#[test]
fn ac_pf_02_vss_on_windows_agent_is_accepted() {
    let mut profile = profile();
    profile.options = BackupOptions {
        use_fs_snapshot: true,
        ..BackupOptions::default()
    };
    let mut target = target();
    target.os = OperatingSystem::Windows;
    assert!(compile(&profile, &target).is_ok());
}

fn backup_args(profile: &Profile, target: &TargetFacts) -> Vec<String> {
    let plan = plan(profile, target);
    let validated = validated(&plan, &repository());
    let backup = plan
        .steps
        .iter()
        .find(|planned| matches!(planned.step.kind, Some(step::Kind::Backup(_))))
        .expect("a backup step");
    arg::build_command(&validated, &backup.step)
        .expect("a command")
        .args()
        .to_vec()
}

#[test]
fn ac_ag_14_exclude_cloud_files_reaches_only_a_windows_backup() {
    let mut profile = profile();
    profile.options = BackupOptions {
        exclude_cloud_files: true,
        ..BackupOptions::default()
    };
    let mut windows = target();
    windows.os = OperatingSystem::Windows;
    assert!(backup_args(&profile, &windows).contains(&"--exclude-cloud-files".to_owned()));
    assert!(!backup_args(&profile, &target()).contains(&"--exclude-cloud-files".to_owned()));

    profile.options.exclude_cloud_files = false;
    assert!(!backup_args(&profile, &windows).contains(&"--exclude-cloud-files".to_owned()));
}

#[test]
fn ac_ag_12_the_plan_carries_the_job_limits() {
    let mut profile = profile();
    profile.limits = crate::profile::JobLimits {
        stall_timeout_seconds: 1800,
        max_duration_seconds: 86_400,
    };
    assert_eq!(plan(&profile, &target()).limits, profile.limits);
}

#[test]
fn ac_pf_02_missing_hook_is_rejected() {
    let mut profile = profile();
    profile.hooks = vec![hook("missing", HookPhase::Before, &[])];
    let error = error(&profile, &target());
    assert!(matches!(
        error,
        CompileError::UnknownHook { hook_id, .. } if hook_id == "missing"
    ));
}

#[test]
fn ac_pf_02_unknown_hook_parameter_is_rejected() {
    let mut profile = profile();
    profile.hooks = vec![hook("pg_dump", HookPhase::Before, &[("other", "app")])];
    let error = error(&profile, &target());
    assert!(matches!(
        error,
        CompileError::UnknownHookParameter { name, .. } if name == "other"
    ));
}

#[test]
fn duplicate_hook_parameter_is_rejected() {
    let mut profile = profile();
    profile.hooks = vec![hook(
        "pg_dump",
        HookPhase::Before,
        &[("database", "app"), ("database", "other")],
    )];
    let error = error(&profile, &target());
    assert!(matches!(
        error,
        CompileError::DuplicateHookParameter { name, .. } if name == "database"
    ));
}

#[test]
fn ac_pf_03_variables_give_each_agent_its_own_values() {
    let mut profile = profile();
    profile.paths.all = vec!["/srv/{{hostname}}/{{os}}/{{agent.label.env}}".to_owned()];

    let linux = target();
    let mut windows = target();
    windows.os = OperatingSystem::Windows;
    windows.hostname = "host2".to_owned();
    windows.labels.insert("env".to_owned(), "dev".to_owned());
    let mut macos = target();
    macos.os = OperatingSystem::Macos;
    macos.hostname = "host3".to_owned();

    assert_eq!(
        backup_paths(&plan(&profile, &linux)),
        vec!["/srv/host1/linux/prod".to_owned()]
    );
    assert_eq!(
        backup_paths(&plan(&profile, &windows)),
        vec!["/srv/host2/windows/dev".to_owned()]
    );
    assert_eq!(
        backup_paths(&plan(&profile, &macos)),
        vec!["/srv/host3/macos/prod".to_owned()]
    );
}

#[test]
fn ac_pr_08_injected_values_stay_out_of_option_positions() {
    let mut profile = profile();
    profile.paths.all = vec![BAD_OPTION.to_owned(), BAD_SHORT.to_owned()];
    profile.excludes = vec![BAD_OPTION.to_owned()];
    profile.tags = vec![BAD_SHORT.to_owned()];
    let plan = plan(&profile, &target());
    let validated = validated(&plan, &repository());
    let step = match &plan.steps[0].step.kind {
        Some(step::Kind::Backup(backup)) => step::Kind::Backup(backup.clone()),
        other => panic!("unexpected step {other:?}"),
    };
    let step = restichub_proto::agent::v1::Step {
        kind: Some(step),
        ..restichub_proto::agent::v1::Step::default()
    };
    let line = match arg::build_command(&validated, &step) {
        Ok(value) => value,
        Err(error) => panic!("{error:?}"),
    };

    let mut positional = false;
    for (index, argument) in line.args().iter().enumerate() {
        if argument == "--" {
            positional = true;
            continue;
        }
        if positional {
            continue;
        }
        assert!(index == 0 || argument.starts_with("--"), "{argument}");
    }
    assert!(line.args().contains(&BAD_OPTION.to_owned()));
    assert!(line.args().contains(&BAD_SHORT.to_owned()));
    assert!(line.args().contains(&format!("--exclude={BAD_OPTION}")));
    assert!(line.args().contains(&format!("--tag={BAD_SHORT}")));
}

#[test]
fn ac_pr_08_rejects_empty_or_comma_tag() {
    for bad in ["", ",", "a,b", " daily", "daily ", "a\tb"] {
        let mut tagged = profile();
        tagged.tags = vec![bad.to_owned()];
        assert!(
            matches!(error(&tagged, &target()), CompileError::InvalidValue { .. }),
            "{bad:?}"
        );
    }

    for bad in ["", ",", " keep"] {
        let mut keep = profile();
        keep.retention = Some(Retention {
            keep_tags: vec![bad.to_owned()],
            ..Retention::default()
        });
        assert!(
            matches!(error(&keep, &target()), CompileError::InvalidValue { .. }),
            "{bad:?}"
        );

        let mut filter = profile();
        filter.retention = Some(Retention {
            filter: SnapshotFilter {
                tags: vec![bad.to_owned()],
                ..SnapshotFilter::default()
            },
            ..Retention::default()
        });
        assert!(
            matches!(error(&filter, &target()), CompileError::InvalidValue { .. }),
            "{bad:?}"
        );
    }
}

#[test]
fn ac_pr_08_rejects_format_characters() {
    for bad in [
        "a\u{202E}",
        "a\u{200B}",
        "a\u{2028}",
        "a\u{2029}",
        "a\u{001B}",
        "a\u{007F}",
    ] {
        let mut profile = profile();
        profile.paths.all = vec![bad.to_owned()];
        assert!(
            matches!(
                error(&profile, &target()),
                CompileError::InvalidValue { .. }
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn ac_pr_08_newline_and_nul_values_are_rejected() {
    for bad in ["bad\nvalue", "bad\rvalue", "bad\0value"] {
        let mut profile = profile();
        profile.paths.all = vec![bad.to_owned()];
        assert!(
            matches!(
                error(&profile, &target()),
                CompileError::InvalidValue { .. }
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn unterminated_template_is_rejected() {
    let mut profile = profile();
    profile.paths.all = vec!["{{hostname".to_owned()];
    assert!(matches!(
        error(&profile, &target()),
        CompileError::UnterminatedTemplate { .. }
    ));
}

#[test]
fn missing_label_is_rejected() {
    let mut profile = profile();
    profile.paths.all = vec!["{{agent.label.nope}}".to_owned()];
    assert!(matches!(
        error(&profile, &target()),
        CompileError::MissingLabel { .. }
    ));
}

#[test]
fn empty_paths_are_rejected() {
    let mut profile = profile();
    profile.paths.all = Vec::new();
    assert!(matches!(
        error(&profile, &target()),
        CompileError::NoPaths { .. }
    ));
}

#[test]
fn ac_pf_01_preview_masks_secrets() {
    let plan = plan(&profile(), &target());
    let previews = match preview(&plan, &repository(), &allow_list()) {
        Ok(value) => value,
        Err(error) => panic!("{error:?}"),
    };
    let backup = &previews[0];
    assert!(backup.command.starts_with("restic backup"));
    assert!(!backup.command.contains("topsecret"));
    assert!(
        backup
            .environment
            .iter()
            .any(|(name, value)| name == "RESTIC_PASSWORD" && value == "***")
    );
}

#[test]
fn ac_pf_01_preview_masks_secret_hook_parameter() {
    let mut profile = profile();
    profile.hooks = vec![hook(
        "dump_secret",
        HookPhase::Before,
        &[("password", "s3cr3t")],
    )];
    let plan = plan(&profile, &target());
    let previews = match preview(&plan, &repository(), &allow_list()) {
        Ok(value) => value,
        Err(error) => panic!("{error:?}"),
    };
    assert!(previews[0].command.contains("password=***"));
    assert!(!previews[0].command.contains("s3cr3t"));
}

#[test]
fn ac_pf_01_preview_quotes_hook_parameter_value() {
    let mut profile = profile();
    profile.hooks = vec![hook("pg_dump", HookPhase::Before, &[("database", "my db")])];
    let plan = plan(&profile, &target());
    let previews = match preview(&plan, &repository(), &allow_list()) {
        Ok(value) => value,
        Err(error) => panic!("{error:?}"),
    };
    assert!(previews[0].command.contains("database='my db'"));
}

fn attack() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just(",".to_owned()),
        Just("a,b".to_owned()),
        Just("-x".to_owned()),
        Just("--".to_owned()),
        Just("--password-command=cmd /c calc".to_owned()),
        Just("\u{202E}".to_owned()),
        Just("\u{FF0D}".to_owned()),
        Just("{{os}}".to_owned()),
        Just("%n".to_owned()),
        any::<String>(),
    ]
}

proptest! {
    #[test]
    fn injection_values_never_become_options(
        values in prop::collection::vec(attack(), 0..6),
        hook_values in prop::collection::vec(attack(), 0..3),
    ) {
        let profile = Profile {
            repository_id: "repo-1".to_owned(),
            paths: ProfilePaths {
                all: values.clone(),
                by_os: BTreeMap::new(),
            },
            excludes: values.clone(),
            exclude_files: values.clone(),
            tags: values.clone(),
            profile_tag: Some(PROFILE_TAG.to_owned()),
            retention: Some(Retention {
                keep_last: 1,
                keep_tags: values.clone(),
                filter: SnapshotFilter {
                    tags: values.clone(),
                    ..SnapshotFilter::default()
                },
                ..Retention::default()
            }),
            check: Some(CheckOptions {
                read_data_subset: "10%".to_owned(),
            }),
            hooks: vec![HookRef {
                hook_id: "pg_dump".to_owned(),
                phase: HookPhase::Before,
                parameters: hook_values
                    .iter()
                    .map(|value| HookParameter {
                        name: "database".to_owned(),
                        value: value.clone(),
                    })
                    .collect(),
            }],
            ..Profile::default()
        };

        let Ok(plan) = compile(&profile, &target()) else {
            return Ok(());
        };
        let validated = match ValidatedJob::new(Job {
            job_id: "job-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            agent_id: "agent-1".to_owned(),
            repositories: vec![repository()],
            steps: plan.steps.iter().map(|planned| planned.step.clone()).collect(),
            ..Job::default()
        }) {
            Ok(value) => value,
            Err(error) => panic!("{error:?}"),
        };

        for planned in &plan.steps {
            match &planned.step.kind {
                Some(step::Kind::Backup(backup)) => {
                    for tag in &backup.tags {
                        prop_assert!(!tag.is_empty() && !tag.contains(',') && tag.trim() == tag);
                    }
                }
                Some(step::Kind::Forget(forget)) => {
                    for tag in &forget.keep_tags {
                        prop_assert!(!tag.is_empty() && !tag.contains(',') && tag.trim() == tag);
                    }
                    if let Some(filter) = &forget.filter {
                        for tag in filter.tags.iter().chain(&filter.all_tags) {
                            prop_assert!(!tag.is_empty() && !tag.contains(',') && tag.trim() == tag);
                        }
                    }
                }
                _ => {}
            }

            let Ok(line) = arg::build_command(&validated, &planned.step) else {
                continue;
            };
            let mut positional = false;
            for (index, argument) in line.args().iter().enumerate() {
                prop_assert!(!restichub_proto::validate::has_forbidden(argument));
                if argument == "--" {
                    positional = true;
                    continue;
                }
                if positional {
                    continue;
                }
                if index == 0 {
                    prop_assert!(!argument.starts_with('-'));
                    continue;
                }
                prop_assert!(argument.starts_with("--"), "value {argument:?} before --");
            }
        }
    }
}
