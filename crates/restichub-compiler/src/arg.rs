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

const MASK: &str = "***";

/// A restic command line. `args` holds no secret; `env` holds the secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// A step that the builder cannot turn into a restic command.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ArgError {
    #[error("the step has no known kind")]
    UnknownStep,
    #[error("a hook step is not a restic command")]
    NotRestic,
}

/// Builds the restic arguments and environment of one step (ADR-0021).
///
/// # Errors
///
/// Returns [`ArgError::UnknownStep`] for a step without a kind and
/// [`ArgError::NotRestic`] for a hook step.
pub fn build_command(step: &Step, repository: &RepositoryAccess) -> Result<CommandLine, ArgError> {
    let kind = step.kind.as_ref().ok_or(ArgError::UnknownStep)?;
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
            push_flag(&mut args, backup.retry_lock, "--retry-lock");
            positional(&mut args, &backup.paths);
        }
        step::Kind::Check(check) => {
            args.push("check".to_owned());
            push_value(&mut args, "--read-data-subset", &check.read_data_subset);
            push_flag(&mut args, check.retry_lock, "--retry-lock");
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
                for path in &filter.paths {
                    args.push(attached("--path", path));
                }
            }
            push_flag(&mut args, forget.retry_lock, "--retry-lock");
        }
        step::Kind::Prune(prune) => {
            args.push("prune".to_owned());
            push_value(&mut args, "--max-unused", &prune.max_unused);
            push_value(&mut args, "--max-repack-size", &prune.max_repack_size);
            push_flag(&mut args, prune.retry_lock, "--retry-lock");
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

/// Replaces the value of every secret environment variable with a mask (AC-PF-01).
pub fn mask_secrets(line: &mut CommandLine) {
    for (name, value) in &mut line.env {
        if is_secret(name) {
            *value = MASK.to_owned();
        }
    }
}

/// Returns true for an environment variable that holds a secret.
#[must_use]
pub fn is_secret(name: &str) -> bool {
    matches!(
        name,
        "RESTIC_PASSWORD" | "AWS_SECRET_ACCESS_KEY" | "AWS_SESSION_TOKEN" | "RESTIC_REST_PASSWORD"
    )
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
    use restichub_proto::agent::v1::{BackupStep, HookStep, RepositoryAccess, Step, step};

    use super::{ArgError, MASK, build_command, is_secret, mask_secrets};

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

    fn line(step: &Step) -> super::CommandLine {
        match build_command(step, &repository()) {
            Ok(value) => value,
            Err(error) => panic!("{error:?}"),
        }
    }

    #[test]
    fn ac_pr_08_flag_like_positional_goes_after_dash_dash() {
        let step = backup(&[BAD_OPTION, BAD_SHORT], &[], &[]);
        let line = line(&step);
        let marker = line.args.iter().position(|value| value == "--");
        assert_eq!(marker, Some(1));
        assert_eq!(line.args.get(2).map(String::as_str), Some(BAD_OPTION));
        assert_eq!(line.args.get(3).map(String::as_str), Some(BAD_SHORT));
    }

    #[test]
    fn ac_pr_08_flag_like_option_value_uses_attached_form() {
        let step = backup(&["/srv"], &[BAD_OPTION], &[BAD_SHORT]);
        let line = line(&step);
        assert!(line.args.contains(&format!("--exclude={BAD_OPTION}")));
        assert!(line.args.contains(&format!("--tag={BAD_SHORT}")));
        assert!(!line.args.iter().any(|value| value == BAD_SHORT));
    }

    #[test]
    fn secrets_go_to_environment_not_arguments() {
        let line = line(&backup(&["/srv"], &[], &[]));
        assert!(!line.args.iter().any(|value| value.contains("topsecret")));
        assert!(
            line.env
                .iter()
                .any(|(name, value)| name == "RESTIC_PASSWORD" && value == "topsecret")
        );
    }

    #[test]
    fn ac_pf_01_mask_secrets_masks_the_password() {
        let mut line = line(&backup(&["/srv"], &[], &[]));
        mask_secrets(&mut line);
        assert!(
            line.env
                .iter()
                .all(|(name, value)| !is_secret(name) || value == MASK)
        );
    }

    #[test]
    fn hook_step_is_not_a_restic_command() {
        let step = Step {
            step_id: "hook-0".to_owned(),
            kind: Some(step::Kind::Hook(HookStep::default())),
            ..Step::default()
        };
        assert_eq!(
            build_command(&step, &repository()),
            Err(ArgError::NotRestic)
        );
    }

    #[test]
    fn step_without_kind_is_rejected() {
        assert_eq!(
            build_command(&Step::default(), &repository()),
            Err(ArgError::UnknownStep)
        );
    }
}
