//! The restic command line builder (ADR-0021, FR-PR-10).
//!
//! This module is the one place that turns a typed step into restic arguments.
//! The agent uses it. Every positional value goes after `--` and every option
//! value uses the attached form `--flag=value`. A value from a step can never
//! become an option. Secrets go to the environment, never to the arguments
//! (FR-AG-06).

use restichub_proto::agent::v1::{
    OverwriteMode, RepositoryAccess, StatsMode, Step, repository_access::BackendCredentials, step,
};

use zeroize::Zeroize as _;

use crate::job::ValidatedJob;

/// Text that replaces a secret in the preview.
pub const MASK: &str = "***";

/// How long restic waits for a held lock when a step sets `retry_lock` (FR-SC-05). restic needs a
/// value: a bare `--retry-lock` swallows the next argument.
const RETRY_LOCK: &str = "--retry-lock=5m";

/// Environment variables that the preview shows without a mask. Every other
/// value is masked, so a new secret variable is masked by default (AC-PF-01).
const SAFE_ENV: &[&str] = &["AWS_DEFAULT_REGION"];

/// The restic commands whose output the agent parses as JSON (ADR-0011).
const JSON_COMMANDS: &[&str] = &["backup", "check", "snapshots", "forget", "stats", "ls"];

/// A restic command line. `args` holds no secret; `env` holds the secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

impl CommandLine {
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    #[must_use]
    pub fn env(&self) -> &[(String, String)] {
        &self.env
    }

    /// Adds `--json` after the restic command when that command has JSON output (FR-AG-08,
    /// ADR-0011). The agent runs this form; the preview shows the plain form.
    #[must_use]
    pub fn with_json(mut self) -> Self {
        if self
            .args
            .first()
            .is_some_and(|command| JSON_COMMANDS.contains(&command.as_str()))
        {
            self.args.insert(1, "--json".to_owned());
        }
        self
    }
}

impl Drop for CommandLine {
    fn drop(&mut self) {
        for (_, value) in &mut self.env {
            value.zeroize();
        }
    }
}

/// A step that the builder cannot turn into a restic command.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArgError {
    #[error("the step has no known kind")]
    UnknownStep,
    #[error("a hook step is not a restic command")]
    NotRestic,
    #[error("the step refers to a repository that the job does not hold")]
    UnknownRepository,
}

/// Builds the restic arguments and environment of one step (ADR-0021).
///
/// The step must belong to the validated job. Validation already checked the
/// repository location, so the builder cannot receive an unsafe location.
///
/// # Errors
///
/// Returns [`ArgError::UnknownStep`] for a step without a kind,
/// [`ArgError::NotRestic`] for a hook step and [`ArgError::UnknownRepository`]
/// for a step whose repository the job does not hold.
pub fn build_command(validated: &ValidatedJob, step: &Step) -> Result<CommandLine, ArgError> {
    let kind = step.kind.as_ref().ok_or(ArgError::UnknownStep)?;
    let Some(repository_id) = repository_id(kind) else {
        return Err(ArgError::NotRestic);
    };
    let repository = validated
        .repository(repository_id)
        .ok_or(ArgError::UnknownRepository)?;
    let mut args = Vec::new();
    match kind {
        step::Kind::Backup(backup) => {
            args.push("backup".to_owned());
            for exclude in &backup.excludes {
                args.push(attached("--exclude", exclude));
            }
            for file in &backup.exclude_files {
                args.push(attached("--exclude-file", file));
            }
            for tag in &backup.tags {
                args.push(attached("--tag", tag));
            }
            push_flag(&mut args, backup.use_fs_snapshot, "--use-fs-snapshot");
            push_flag(&mut args, backup.one_file_system, "--one-file-system");
            push_flag(&mut args, backup.exclude_caches, "--exclude-caches");
            push_rate(&mut args, "--limit-upload", backup.limit_upload_kib);
            push_rate(&mut args, "--limit-download", backup.limit_download_kib);
            push_flag(&mut args, backup.retry_lock, RETRY_LOCK);
            positional(&mut args, &backup.paths);
        }
        step::Kind::Check(check) => {
            args.push("check".to_owned());
            push_value(&mut args, "--read-data-subset", &check.read_data_subset);
            push_flag(&mut args, check.retry_lock, RETRY_LOCK);
        }
        step::Kind::Snapshots(snapshots) => {
            args.push("snapshots".to_owned());
            for host in &snapshots.hosts {
                args.push(attached("--host", host));
            }
            for tag in &snapshots.tags {
                args.push(attached("--tag", tag));
            }
            for path in &snapshots.paths {
                args.push(attached("--path", path));
            }
            if snapshots.latest > 0 {
                args.push(format!("--latest={}", snapshots.latest));
            }
        }
        step::Kind::Ls(ls) => {
            args.push("ls".to_owned());
            push_flag(&mut args, ls.recursive, "--recursive");
            let mut values = vec![ls.snapshot_id.clone()];
            if !ls.path.is_empty() {
                values.push(ls.path.clone());
            }
            positional(&mut args, &values);
        }
        step::Kind::Restore(restore) => {
            args.push("restore".to_owned());
            for include in &restore.includes {
                args.push(attached("--include", include));
            }
            for exclude in &restore.excludes {
                args.push(attached("--exclude", exclude));
            }
            push_value(&mut args, "--target", &restore.target);
            push_flag(&mut args, restore.dry_run, "--dry-run");
            if let Ok(mode) = OverwriteMode::try_from(restore.overwrite)
                && let Some(value) = overwrite_value(mode)
            {
                args.push(attached("--overwrite", value));
            }
            push_flag(&mut args, restore.verify, "--verify");
            positional(&mut args, std::slice::from_ref(&restore.snapshot_id));
        }
        step::Kind::Unlock(_) => {
            args.push("unlock".to_owned());
        }
        step::Kind::UnlockAll(_) => {
            args.push("unlock".to_owned());
            args.push("--remove-all".to_owned());
        }
        step::Kind::Init(_) => {
            args.push("init".to_owned());
        }
        step::Kind::TestAccess(_) => {
            args.push("cat".to_owned());
            args.push("config".to_owned());
        }
        step::Kind::Forget(forget) => {
            args.push("forget".to_owned());
            push_count(&mut args, "--keep-last", forget.keep_last);
            push_count(&mut args, "--keep-hourly", forget.keep_hourly);
            push_count(&mut args, "--keep-daily", forget.keep_daily);
            push_count(&mut args, "--keep-weekly", forget.keep_weekly);
            push_count(&mut args, "--keep-monthly", forget.keep_monthly);
            push_count(&mut args, "--keep-yearly", forget.keep_yearly);
            push_value(&mut args, "--keep-within", &forget.keep_within);
            for tag in &forget.keep_tags {
                args.push(attached("--keep-tag", tag));
            }
            if let Some(group_by) = group_by_value(&forget.group_by) {
                args.push(attached("--group-by", &group_by));
            }
            if let Some(filter) = &forget.filter {
                for host in &filter.hosts {
                    args.push(attached("--host", host));
                }
                for tag in &filter.tags {
                    args.push(attached("--tag", tag));
                }
                if !filter.all_tags.is_empty() {
                    args.push(attached("--tag", &filter.all_tags.join(",")));
                }
                for path in &filter.paths {
                    args.push(attached("--path", path));
                }
            }
            push_flag(&mut args, forget.retry_lock, RETRY_LOCK);
        }
        step::Kind::Prune(prune) => {
            args.push("prune".to_owned());
            push_value(&mut args, "--max-unused", &prune.max_unused);
            push_value(&mut args, "--max-repack-size", &prune.max_repack_size);
            push_flag(&mut args, prune.retry_lock, RETRY_LOCK);
        }
        step::Kind::Stats(stats) => {
            args.push("stats".to_owned());
            for snapshot_id in &stats.snapshot_ids {
                args.push(attached("--snapshot", snapshot_id));
            }
            if let Ok(mode) = StatsMode::try_from(stats.mode)
                && let Some(value) = stats_value(mode)
            {
                args.push(attached("--mode", value));
            }
        }
        step::Kind::Hook(_) => return Err(ArgError::NotRestic),
    }

    Ok(CommandLine {
        program: "restic".to_owned(),
        args,
        env: repository_env(repository),
    })
}

/// Masks every environment value except an explicit safe list, and masks the
/// userinfo of `RESTIC_REPOSITORY` (AC-PF-01).
pub fn mask_secrets(line: &mut CommandLine) {
    for (name, value) in &mut line.env {
        if name == "RESTIC_REPOSITORY" {
            *value = mask_userinfo(value);
        } else if !is_safe_env(name) {
            *value = MASK.to_owned();
        }
    }
}

/// Returns true for an environment variable that the preview shows in clear.
#[must_use]
pub fn is_safe_env(name: &str) -> bool {
    SAFE_ENV.contains(&name)
}

/// Hides the credentials in the authority of a URL. `rest:https://u:p@h/`
/// becomes `rest:https://***@h/`. A value without userinfo is unchanged.
fn mask_userinfo(value: &str) -> String {
    let Some(scheme_end) = value.find("://") else {
        return value.to_owned();
    };
    let after = &value[scheme_end + 3..];
    let Some(at) = after.find('@') else {
        return value.to_owned();
    };
    let slash = after.find('/').unwrap_or(usize::MAX);
    if at >= slash {
        return value.to_owned();
    }
    let mut masked = String::with_capacity(value.len());
    masked.push_str(&value[..scheme_end + 3]);
    masked.push_str(MASK);
    masked.push_str(&after[at..]);
    masked
}

/// The repository identifier of a step that runs restic, or `None` for a hook.
fn repository_id(kind: &step::Kind) -> Option<&str> {
    match kind {
        step::Kind::Backup(backup) => Some(&backup.repository_id),
        step::Kind::Check(check) => Some(&check.repository_id),
        step::Kind::Snapshots(snapshots) => Some(&snapshots.repository_id),
        step::Kind::Ls(ls) => Some(&ls.repository_id),
        step::Kind::Restore(restore) => Some(&restore.repository_id),
        step::Kind::Unlock(unlock) => Some(&unlock.repository_id),
        step::Kind::UnlockAll(unlock_all) => Some(&unlock_all.repository_id),
        step::Kind::Init(init) => Some(&init.repository_id),
        step::Kind::TestAccess(test_access) => Some(&test_access.repository_id),
        step::Kind::Forget(forget) => Some(&forget.repository_id),
        step::Kind::Prune(prune) => Some(&prune.repository_id),
        step::Kind::Stats(stats) => Some(&stats.repository_id),
        step::Kind::Hook(_) => None,
    }
}

fn repository_env(repository: &RepositoryAccess) -> Vec<(String, String)> {
    let mut env = vec![("RESTIC_REPOSITORY".to_owned(), repository.location.clone())];
    push_env(&mut env, "RESTIC_PASSWORD", &repository.password);
    match &repository.backend_credentials {
        Some(BackendCredentials::S3(credentials)) => {
            push_env(&mut env, "AWS_ACCESS_KEY_ID", &credentials.access_key_id);
            push_env(
                &mut env,
                "AWS_SECRET_ACCESS_KEY",
                &credentials.secret_access_key,
            );
            push_env(&mut env, "AWS_SESSION_TOKEN", &credentials.session_token);
            push_env(&mut env, "AWS_DEFAULT_REGION", &credentials.region);
        }
        Some(BackendCredentials::Rest(credentials)) => {
            push_env(&mut env, "RESTIC_REST_USERNAME", &credentials.username);
            push_env(&mut env, "RESTIC_REST_PASSWORD", &credentials.password);
        }
        Some(BackendCredentials::Sftp(_)) | Some(BackendCredentials::Local(_)) | None => {}
    }
    env
}

fn push_env(env: &mut Vec<(String, String)>, name: &str, value: &str) {
    if !value.is_empty() {
        env.push((name.to_owned(), value.to_owned()));
    }
}

fn attached(flag: &str, value: &str) -> String {
    format!("{flag}={value}")
}

fn push_flag(args: &mut Vec<String>, present: bool, flag: &str) {
    if present {
        args.push(flag.to_owned());
    }
}

fn push_value(args: &mut Vec<String>, flag: &str, value: &str) {
    if !value.is_empty() {
        args.push(attached(flag, value));
    }
}

fn push_count(args: &mut Vec<String>, flag: &str, value: u32) {
    if value > 0 {
        args.push(format!("{flag}={value}"));
    }
}

fn push_rate(args: &mut Vec<String>, flag: &str, value: u64) {
    if value > 0 {
        args.push(format!("{flag}={value}"));
    }
}

fn positional(args: &mut Vec<String>, values: &[String]) {
    if values.is_empty() {
        return;
    }
    args.push("--".to_owned());
    args.extend(values.iter().cloned());
}

fn group_by_value(group_by: &[i32]) -> Option<String> {
    let names: Vec<&str> = group_by
        .iter()
        .filter_map(|value| restichub_proto::agent::v1::ForgetGroupBy::try_from(*value).ok())
        .filter_map(|value| match value {
            restichub_proto::agent::v1::ForgetGroupBy::Host => Some("host"),
            restichub_proto::agent::v1::ForgetGroupBy::Paths => Some("paths"),
            restichub_proto::agent::v1::ForgetGroupBy::Tags => Some("tags"),
            restichub_proto::agent::v1::ForgetGroupBy::Unspecified => None,
        })
        .collect();
    if names.is_empty() {
        None
    } else {
        Some(names.join(","))
    }
}

fn stats_value(mode: StatsMode) -> Option<&'static str> {
    match mode {
        StatsMode::RestoreSize => Some("restore-size"),
        StatsMode::FilesByContents => Some("files-by-contents"),
        StatsMode::BlobsPerFile => Some("blobs-per-file"),
        StatsMode::RawData => Some("raw-data"),
        StatsMode::Unspecified => None,
    }
}

fn overwrite_value(mode: OverwriteMode) -> Option<&'static str> {
    match mode {
        OverwriteMode::Always => Some("always"),
        OverwriteMode::IfChanged => Some("if-changed"),
        OverwriteMode::IfNewer => Some("if-newer"),
        OverwriteMode::Never => Some("never"),
        OverwriteMode::Unspecified => None,
    }
}

#[cfg(test)]
mod tests {
    use restichub_proto::agent::v1::{
        BackupStep, HookStep, Job, RepositoryAccess, Step, UnlockStep, step,
    };

    use super::{ArgError, MASK, build_command, is_safe_env, mask_secrets};
    use crate::job::ValidatedJob;

    const BAD_OPTION: &str = "--password-command=cmd /c calc";
    const BAD_SHORT: &str = "-x";

    fn repository() -> RepositoryAccess {
        RepositoryAccess {
            repository_id: "repo-1".to_owned(),
            location: "s3:s3.example.com/bucket".to_owned(),
            password: "topsecret".to_owned(),
            ..RepositoryAccess::default()
        }
    }

    fn validated(step: Step) -> ValidatedJob {
        let job = Job {
            job_id: "job-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            agent_id: "agent-1".to_owned(),
            repositories: vec![repository()],
            steps: vec![step],
            ..Job::default()
        };
        match ValidatedJob::new(job) {
            Ok(value) => value,
            Err(error) => panic!("{error:?}"),
        }
    }

    fn backup(paths: &[&str], excludes: &[&str], tags: &[&str]) -> Step {
        Step {
            step_id: "backup".to_owned(),
            kind: Some(step::Kind::Backup(BackupStep {
                repository_id: "repo-1".to_owned(),
                paths: paths.iter().map(|value| (*value).to_owned()).collect(),
                excludes: excludes.iter().map(|value| (*value).to_owned()).collect(),
                tags: tags.iter().map(|value| (*value).to_owned()).collect(),
                ..BackupStep::default()
            })),
            ..Step::default()
        }
    }

    fn unlock() -> Step {
        Step {
            step_id: "unlock".to_owned(),
            kind: Some(step::Kind::Unlock(UnlockStep {
                repository_id: "repo-1".to_owned(),
            })),
            ..Step::default()
        }
    }

    fn line(step: &Step) -> super::CommandLine {
        let validated = validated(step.clone());
        match build_command(&validated, step) {
            Ok(value) => value,
            Err(error) => panic!("{error:?}"),
        }
    }

    #[test]
    fn ac_pr_08_flag_like_positional_goes_after_dash_dash() {
        let step = backup(&[BAD_OPTION, BAD_SHORT], &[], &[]);
        let line = line(&step);
        let marker = line.args().iter().position(|value| value == "--");
        assert_eq!(marker, Some(1));
        assert_eq!(line.args().get(2).map(String::as_str), Some(BAD_OPTION));
        assert_eq!(line.args().get(3).map(String::as_str), Some(BAD_SHORT));
    }

    #[test]
    fn ac_pr_08_flag_like_option_value_uses_attached_form() {
        let step = backup(&["/srv"], &[BAD_OPTION], &[BAD_SHORT]);
        let line = line(&step);
        assert!(line.args().contains(&format!("--exclude={BAD_OPTION}")));
        assert!(line.args().contains(&format!("--tag={BAD_SHORT}")));
        assert!(!line.args().iter().any(|value| value == BAD_SHORT));
    }

    #[test]
    fn secrets_go_to_environment_not_arguments() {
        let line = line(&backup(&["/srv"], &[], &[]));
        assert!(!line.args().iter().any(|value| value.contains("topsecret")));
        assert!(
            line.env()
                .iter()
                .any(|(name, value)| name == "RESTIC_PASSWORD" && value == "topsecret")
        );
    }

    #[test]
    fn ac_pf_01_mask_secrets_masks_every_unsafe_value() {
        let mut line = line(&backup(&["/srv"], &[], &[]));
        mask_secrets(&mut line);
        for (name, value) in line.env() {
            if name == "RESTIC_REPOSITORY" {
                assert!(!value.contains("topsecret"));
            } else {
                assert!(is_safe_env(name) || value == MASK, "{name}");
            }
        }
    }

    #[test]
    fn ac_pf_01_mask_secrets_masks_repository_userinfo() {
        let mut repository = repository();
        repository.location = "rest:https://user:pass@rest.example.com/".to_owned();
        let job = Job {
            job_id: "job-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            agent_id: "agent-1".to_owned(),
            repositories: vec![repository],
            steps: vec![unlock()],
            ..Job::default()
        };
        let validated = match ValidatedJob::new(job) {
            Ok(value) => value,
            Err(error) => panic!("{error:?}"),
        };
        let mut line = match build_command(&validated, &unlock()) {
            Ok(value) => value,
            Err(error) => panic!("{error:?}"),
        };
        mask_secrets(&mut line);
        assert!(line.env().iter().any(|(name, value)| {
            name == "RESTIC_REPOSITORY" && value == "rest:https://***@rest.example.com/"
        }));
    }

    #[test]
    fn ac_pr_08_rejects_program_running_location() {
        for location in ["rclone:remote:path", "sftp:host:/srv", "-x", "local:/srv"] {
            let mut repository = repository();
            repository.location = location.to_owned();
            let job = Job {
                job_id: "job-1".to_owned(),
                tenant_id: "tenant-1".to_owned(),
                agent_id: "agent-1".to_owned(),
                repositories: vec![repository],
                steps: vec![unlock()],
                ..Job::default()
            };
            assert!(ValidatedJob::new(job).is_err(), "{location}");
        }
    }

    #[test]
    fn adr_0011_json_goes_right_after_the_command_and_before_the_separator() {
        let line = line(&backup(&["/srv"], &["*.tmp"], &[])).with_json();
        assert_eq!(
            line.args(),
            ["backup", "--json", "--exclude=*.tmp", "--", "/srv"]
        );
    }

    #[test]
    fn adr_0011_a_command_without_json_output_is_unchanged() {
        let line = line(&unlock());
        assert_eq!(line.clone().with_json().args(), line.args());
    }

    #[test]
    fn fr_sc_05_retry_lock_has_an_attached_duration() {
        let mut step = backup(&["/srv"], &[], &[]);
        if let Some(step::Kind::Backup(backup)) = &mut step.kind {
            backup.retry_lock = true;
        }
        let line = line(&step);
        assert!(line.args().contains(&"--retry-lock=5m".to_owned()));
        assert!(!line.args().contains(&"--retry-lock".to_owned()));
    }

    #[test]
    fn hook_step_is_not_a_restic_command() {
        let step = Step {
            step_id: "hook-0".to_owned(),
            kind: Some(step::Kind::Hook(HookStep {
                hook_id: "pg_dump".to_owned(),
                ..HookStep::default()
            })),
            ..Step::default()
        };
        let validated = validated(step.clone());
        assert_eq!(build_command(&validated, &step), Err(ArgError::NotRestic));
    }

    #[test]
    fn step_without_kind_is_rejected() {
        let validated = validated(unlock());
        assert_eq!(
            build_command(&validated, &Step::default()),
            Err(ArgError::UnknownStep)
        );
    }

    #[test]
    fn step_of_an_unknown_repository_is_rejected() {
        let validated = validated(unlock());
        let step = backup(&["/srv"], &[], &[]);
        let mut step = step;
        if let Some(step::Kind::Backup(backup)) = &mut step.kind {
            backup.repository_id = "repo-9".to_owned();
        }
        assert_eq!(
            build_command(&validated, &step),
            Err(ArgError::UnknownRepository)
        );
    }
}
