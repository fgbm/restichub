//! Argument validation of a job (FR-PR-10, FR-PR-01).
//!
//! The control plane calls [`validate_job`](crate::validate::validate_job) before it signs a job. The agent
//! calls it before it runs a job. Validation keeps a job inside the typed
//! contract: the job has steps, every step has a known kind, every step refers
//! to a repository of the job, every required field is present, and every
//! string field has no control or format character. A field with a format, for
//! example a snapshot identifier or a size, must match that format. The SFTP
//! private key is the one exception: a PEM key has line breaks, so that field
//! rejects a NUL only.
//!
//! A tag rejects an empty value, a comma and surrounding space, because restic
//! splits a tag on commas and reads an empty tag as "untagged". The repository
//! location must be a form that restic opens without an external program
//! (ADR-0021): `s3:`, `rest:` or an absolute local path.
//!
//! The agent builds the restic command line from the typed fields (ADR-0021).
//! Validation rejects a flag-like value only where the field has a format. A
//! free value, for example a path, an exclude pattern or a tag, may contain a
//! flag-like string: the command builder places a positional value after `--`
//! or attaches an option value with `--flag=value`, so the value never becomes
//! an option.

use std::collections::HashSet;

use crate::agent::v1::{
    HookStep, Job, RejectReason, RepositoryAccess, SnapshotsFilter, Step,
    repository_access::BackendCredentials, step,
};
use crate::signing::Rejection;

/// Validates a job against the typed contract (FR-PR-10).
///
/// # Errors
///
/// Returns a [`Rejection`] with [`RejectReason::UnsupportedStep`] for an
/// unknown step kind, [`RejectReason::InvalidHookParameters`] for a duplicate
/// hook parameter name, and [`RejectReason::Malformed`] for any other rule.
pub fn validate_job(job: &Job) -> Result<(), Rejection> {
    if job.steps.is_empty() {
        return Err(reject(job, RejectReason::Malformed));
    }

    for repository in &job.repositories {
        check_repository(job, repository)?;
    }

    let repository_ids: Vec<&str> = job
        .repositories
        .iter()
        .map(|repository| repository.repository_id.as_str())
        .collect();

    for step in &job.steps {
        check_step(job, &repository_ids, step)?;
    }

    Ok(())
}

fn check_step(job: &Job, repository_ids: &[&str], step: &Step) -> Result<(), Rejection> {
    let Some(kind) = step.kind.as_ref() else {
        return Err(reject(job, RejectReason::UnsupportedStep));
    };

    match kind {
        step::Kind::Backup(backup) => {
            check_repository_id(job, repository_ids, &backup.repository_id)?;
            if backup.paths.is_empty() {
                return Err(reject(job, RejectReason::Malformed));
            }
            check_free(job, backup.paths.iter())?;
            check_free(job, backup.excludes.iter())?;
            check_free(job, backup.exclude_files.iter())?;
            check_tags(job, backup.tags.iter())?;
        }
        step::Kind::Check(check) => {
            check_repository_id(job, repository_ids, &check.repository_id)?;
            check_format(job, &check.read_data_subset, is_read_data_subset)?;
        }
        step::Kind::Snapshots(snapshots) => {
            check_repository_id(job, repository_ids, &snapshots.repository_id)?;
            check_free(job, snapshots.hosts.iter())?;
            check_tags(job, snapshots.tags.iter())?;
            check_free(job, snapshots.paths.iter())?;
        }
        step::Kind::Ls(ls) => {
            check_repository_id(job, repository_ids, &ls.repository_id)?;
            check_required_format(job, &ls.snapshot_id, is_snapshot_id)?;
            check_free(job, std::iter::once(&ls.path))?;
        }
        step::Kind::Restore(restore) => {
            check_repository_id(job, repository_ids, &restore.repository_id)?;
            check_required_format(job, &restore.snapshot_id, is_snapshot_id)?;
            check_required_free(job, &restore.target)?;
            check_free(job, restore.includes.iter())?;
            check_free(job, restore.excludes.iter())?;
        }
        step::Kind::Unlock(unlock) => {
            check_repository_id(job, repository_ids, &unlock.repository_id)?;
        }
        step::Kind::UnlockAll(unlock_all) => {
            check_repository_id(job, repository_ids, &unlock_all.repository_id)?;
        }
        step::Kind::Init(init) => {
            check_repository_id(job, repository_ids, &init.repository_id)?;
        }
        step::Kind::TestAccess(test_access) => {
            check_repository_id(job, repository_ids, &test_access.repository_id)?;
        }
        step::Kind::Forget(forget) => {
            check_repository_id(job, repository_ids, &forget.repository_id)?;
            check_format(job, &forget.keep_within, is_keep_within)?;
            check_tags(job, forget.keep_tags.iter())?;
            if let Some(filter) = &forget.filter {
                check_filter(job, filter)?;
            }
        }
        step::Kind::Prune(prune) => {
            check_repository_id(job, repository_ids, &prune.repository_id)?;
            check_format(job, &prune.max_unused, is_size_option)?;
            check_format(job, &prune.max_repack_size, is_size_option)?;
        }
        step::Kind::Stats(stats) => {
            check_repository_id(job, repository_ids, &stats.repository_id)?;
            for snapshot_id in &stats.snapshot_ids {
                check_format(job, snapshot_id, is_snapshot_id)?;
            }
        }
        step::Kind::Hook(hook) => {
            check_hook(job, hook)?;
        }
    }

    Ok(())
}

fn check_filter(job: &Job, filter: &SnapshotsFilter) -> Result<(), Rejection> {
    check_free(job, filter.hosts.iter())?;
    check_tags(job, filter.tags.iter())?;
    check_free(job, filter.paths.iter())
}

fn check_hook(job: &Job, hook: &HookStep) -> Result<(), Rejection> {
    if hook.hook_id.is_empty() || has_forbidden(&hook.hook_id) {
        return Err(reject(job, RejectReason::Malformed));
    }

    let mut names = HashSet::new();
    for parameter in &hook.parameters {
        if has_forbidden(&parameter.name) || has_forbidden(&parameter.value) {
            return Err(reject(job, RejectReason::Malformed));
        }
        if !names.insert(parameter.name.as_str()) {
            return Err(reject(job, RejectReason::InvalidHookParameters));
        }
    }

    Ok(())
}

fn check_repository(job: &Job, repository: &RepositoryAccess) -> Result<(), Rejection> {
    if repository.repository_id.is_empty() {
        return Err(reject(job, RejectReason::Malformed));
    }
    if !is_safe_repository_location(&repository.location) {
        return Err(reject(job, RejectReason::Malformed));
    }

    let mut fields = vec![&repository.repository_id, &repository.location];
    fields.push(&repository.password);
    match &repository.backend_credentials {
        Some(BackendCredentials::S3(credentials)) => {
            fields.push(&credentials.access_key_id);
            fields.push(&credentials.secret_access_key);
            fields.push(&credentials.session_token);
            fields.push(&credentials.region);
        }
        Some(BackendCredentials::Rest(credentials)) => {
            fields.push(&credentials.username);
            fields.push(&credentials.password);
        }
        Some(BackendCredentials::Sftp(credentials)) => {
            // A PEM private key has line breaks. This field rejects a NUL only.
            if has_nul(&credentials.private_key) {
                return Err(reject(job, RejectReason::Malformed));
            }
            fields.push(&credentials.password);
            fields.push(&credentials.known_hosts_entry);
        }
        Some(BackendCredentials::Local(_)) | None => {}
    }

    for field in fields {
        if has_forbidden(field) {
            return Err(reject(job, RejectReason::Malformed));
        }
    }

    Ok(())
}

fn check_repository_id(
    job: &Job,
    repository_ids: &[&str],
    repository_id: &str,
) -> Result<(), Rejection> {
    if repository_id.is_empty() || !repository_ids.contains(&repository_id) {
        Err(reject(job, RejectReason::Malformed))
    } else {
        Ok(())
    }
}

/// Rejects a control character; an empty value is allowed.
fn check_free<'a>(
    job: &Job,
    values: impl IntoIterator<Item = &'a String>,
) -> Result<(), Rejection> {
    for value in values {
        if has_forbidden(value) {
            return Err(reject(job, RejectReason::Malformed));
        }
    }
    Ok(())
}

/// Rejects an empty tag, a tag with a comma and a tag with surrounding space.
/// restic splits a tag on commas and reads an empty tag as "untagged", so these
/// forms would change the set of tags.
fn check_tags<'a>(
    job: &Job,
    values: impl IntoIterator<Item = &'a String>,
) -> Result<(), Rejection> {
    for value in values {
        if value.is_empty() || value.contains(',') || value.trim() != value || has_forbidden(value)
        {
            return Err(reject(job, RejectReason::Malformed));
        }
    }
    Ok(())
}

/// Rejects a value that does not match `is_valid`; an empty value is allowed.
fn check_format(job: &Job, value: &str, is_valid: fn(&str) -> bool) -> Result<(), Rejection> {
    if has_forbidden(value) {
        return Err(reject(job, RejectReason::Malformed));
    }
    if !value.is_empty() && !is_valid(value) {
        return Err(reject(job, RejectReason::Malformed));
    }
    Ok(())
}

fn check_required_format(
    job: &Job,
    value: &str,
    is_valid: fn(&str) -> bool,
) -> Result<(), Rejection> {
    if value.is_empty() || has_forbidden(value) || !is_valid(value) {
        return Err(reject(job, RejectReason::Malformed));
    }
    Ok(())
}

fn check_required_free(job: &Job, value: &str) -> Result<(), Rejection> {
    if value.is_empty() || has_forbidden(value) {
        return Err(reject(job, RejectReason::Malformed));
    }
    Ok(())
}

/// Returns true when `value` has a character that a value must not have: a
/// control character (Cc), a format character (Cf, for example a bidirectional
/// override), a line or paragraph separator (Zl, Zp), or DEL (FR-PR-10).
pub fn has_forbidden(value: &str) -> bool {
    value.chars().any(is_forbidden)
}

fn is_forbidden(character: char) -> bool {
    character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') || is_format(character)
}

fn is_format(character: char) -> bool {
    matches!(
        character,
        '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// Returns true when restic can open the location without an external program
/// (ADR-0021). The allowed forms are `s3:` with an optional HTTP endpoint,
/// `rest:` with an HTTP URL, and an absolute local path.
pub fn is_safe_repository_location(value: &str) -> bool {
    if value.is_empty() || value.starts_with('-') || has_forbidden(value) {
        return false;
    }
    if let Some(rest) = value.strip_prefix("s3:") {
        return is_s3_location(rest);
    }
    if let Some(rest) = value.strip_prefix("rest:") {
        return rest.starts_with("https://") || rest.starts_with("http://");
    }
    is_absolute_path(value)
}

fn is_s3_location(rest: &str) -> bool {
    if rest.is_empty() || rest.starts_with('-') {
        return false;
    }
    match rest.find("://") {
        Some(_) => rest.starts_with("https://") || rest.starts_with("http://"),
        None => true,
    }
}

fn is_absolute_path(value: &str) -> bool {
    if value.starts_with('/') {
        return true;
    }
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// A PEM private key has line breaks, so its field rejects a NUL only.
fn has_nul(value: &str) -> bool {
    value.bytes().any(|byte| byte == 0)
}

/// Returns true for a snapshot identifier: `latest`, or 8 to 64 lower-case
/// hexadecimal characters.
fn is_snapshot_id(value: &str) -> bool {
    if value == "latest" {
        return true;
    }
    matches!(value.len(), 8..=64) && value.bytes().all(is_lower_hex)
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

/// Returns true for a keep-within duration: one or more digit groups, each
/// followed by a unit of `y`, `m`, `w`, `d` or `h`.
fn is_keep_within(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let digits_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == digits_start || index >= bytes.len() {
            return false;
        }
        match bytes[index] {
            b'y' | b'Y' | b'm' | b'M' | b'w' | b'W' | b'd' | b'D' | b'h' | b'H' => index += 1,
            _ => return false,
        }
    }
    !value.is_empty()
}

/// Returns true for a size option: `unlimited`, a number with an optional
/// fraction and a `%`, or a number with an optional binary unit.
fn is_size_option(value: &str) -> bool {
    if value == "unlimited" {
        return true;
    }
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index == 0 {
        return false;
    }
    if index == bytes.len() {
        return true;
    }
    match bytes[index] {
        b'%' => index == bytes.len() - 1,
        b'.' => {
            index += 1;
            let fraction_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            fraction_start != index && index == bytes.len() - 1 && bytes[index] == b'%'
        }
        b'k' | b'K' | b'm' | b'M' | b'g' | b'G' | b't' | b'T' => index == bytes.len() - 1,
        _ => false,
    }
}

/// Returns true for a read-data subset: a number pair `a/b`, a percentage, or
/// a number with an optional binary unit.
fn is_read_data_subset(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index == 0 {
        return false;
    }
    if index == bytes.len() {
        return true;
    }
    match bytes[index] {
        b'/' => {
            index += 1;
            let digits_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            digits_start != index && index == bytes.len()
        }
        b'%' => index == bytes.len() - 1,
        b'.' => {
            index += 1;
            let fraction_start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            fraction_start != index && index == bytes.len() - 1 && bytes[index] == b'%'
        }
        b'k' | b'K' | b'm' | b'M' | b'g' | b'G' | b't' | b'T' => index == bytes.len() - 1,
        _ => false,
    }
}

fn reject(job: &Job, reason: RejectReason) -> Rejection {
    Rejection {
        reason,
        job_id: Some(job.job_id.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::v1::{
        BackupStep, CheckStep, ForgetStep, HookParameter, HookStep, LsStep, PruneStep, RestoreStep,
        SftpCredentials, SnapshotsStep, StatsStep, UnlockAllStep, UnlockStep,
    };

    const BAD_OPTION: &str = "--password-command=cmd /c calc";
    const BAD_SHORT: &str = "-x";

    fn repository() -> RepositoryAccess {
        RepositoryAccess {
            repository_id: "repo-1".to_owned(),
            location: "s3:s3.example.com/bucket".to_owned(),
            ..RepositoryAccess::default()
        }
    }

    fn job_with(steps: Vec<Step>) -> Job {
        Job {
            job_id: "job-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            agent_id: "agent-1".to_owned(),
            repositories: vec![repository()],
            steps,
            ..Job::default()
        }
    }

    fn job_with_sftp(credentials: SftpCredentials) -> Job {
        let mut job = job_with(vec![step(step::Kind::Unlock(UnlockStep {
            repository_id: string("repo-1"),
        }))]);
        job.repositories[0].backend_credentials = Some(BackendCredentials::Sftp(credentials));
        job
    }

    fn step(kind: step::Kind) -> Step {
        Step {
            step_id: "step-1".to_owned(),
            kind: Some(kind),
            ..Step::default()
        }
    }

    fn string(value: &str) -> String {
        value.to_owned()
    }

    fn malformed(job: &Job) -> bool {
        validate_job(job) == Err(reject(job, RejectReason::Malformed))
    }

    #[test]
    fn ac_pr_08_accepts_valid_job() {
        let steps = vec![
            step(step::Kind::Backup(BackupStep {
                repository_id: string("repo-1"),
                paths: vec![string("/srv"), string("/etc")],
                excludes: vec![string("*.tmp")],
                tags: vec![string("daily")],
                ..BackupStep::default()
            })),
            step(step::Kind::Check(CheckStep {
                repository_id: string("repo-1"),
                read_data_subset: string("10%"),
                ..CheckStep::default()
            })),
            step(step::Kind::Snapshots(SnapshotsStep {
                repository_id: string("repo-1"),
                ..SnapshotsStep::default()
            })),
            step(step::Kind::Ls(LsStep {
                repository_id: string("repo-1"),
                snapshot_id: string("latest"),
                path: string("/srv"),
                ..LsStep::default()
            })),
            step(step::Kind::Restore(RestoreStep {
                repository_id: string("repo-1"),
                snapshot_id: string("0123abcd"),
                target: string("/restore"),
                ..RestoreStep::default()
            })),
            step(step::Kind::Unlock(UnlockStep {
                repository_id: string("repo-1"),
            })),
            step(step::Kind::UnlockAll(UnlockAllStep {
                repository_id: string("repo-1"),
            })),
            step(step::Kind::Forget(ForgetStep {
                repository_id: string("repo-1"),
                keep_within: string("30d"),
                ..ForgetStep::default()
            })),
            step(step::Kind::Prune(PruneStep {
                repository_id: string("repo-1"),
                max_unused: string("5%"),
                max_repack_size: string("50M"),
                ..PruneStep::default()
            })),
            step(step::Kind::Stats(StatsStep {
                repository_id: string("repo-1"),
                snapshot_ids: vec![string("0123abcd")],
                ..StatsStep::default()
            })),
            step(step::Kind::Hook(HookStep {
                hook_id: string("pg_dump"),
                parameters: vec![HookParameter {
                    name: string("database"),
                    value: string("app"),
                }],
                ..HookStep::default()
            })),
        ];

        assert_eq!(validate_job(&job_with(steps)), Ok(()));
    }

    #[test]
    fn ac_pr_08_rejects_flag_like_value_in_format_fields() {
        let jobs = vec![
            job_with(vec![step(step::Kind::Check(CheckStep {
                repository_id: string("repo-1"),
                read_data_subset: string(BAD_OPTION),
                ..CheckStep::default()
            }))]),
            job_with(vec![step(step::Kind::Ls(LsStep {
                repository_id: string("repo-1"),
                snapshot_id: string(BAD_OPTION),
                ..LsStep::default()
            }))]),
            job_with(vec![step(step::Kind::Forget(ForgetStep {
                repository_id: string("repo-1"),
                keep_within: string(BAD_OPTION),
                ..ForgetStep::default()
            }))]),
            job_with(vec![step(step::Kind::Prune(PruneStep {
                repository_id: string("repo-1"),
                max_unused: string(BAD_OPTION),
                ..PruneStep::default()
            }))]),
            job_with(vec![step(step::Kind::Prune(PruneStep {
                repository_id: string("repo-1"),
                max_repack_size: string(BAD_OPTION),
                ..PruneStep::default()
            }))]),
        ];

        for job in &jobs {
            assert!(malformed(job), "{:?}", job.steps);
        }
    }

    #[test]
    fn ac_pr_08_rejects_short_flag_in_format_fields() {
        let jobs = vec![
            job_with(vec![step(step::Kind::Check(CheckStep {
                repository_id: string("repo-1"),
                read_data_subset: string(BAD_SHORT),
                ..CheckStep::default()
            }))]),
            job_with(vec![step(step::Kind::Ls(LsStep {
                repository_id: string("repo-1"),
                snapshot_id: string(BAD_SHORT),
                ..LsStep::default()
            }))]),
            job_with(vec![step(step::Kind::Forget(ForgetStep {
                repository_id: string("repo-1"),
                keep_within: string(BAD_SHORT),
                ..ForgetStep::default()
            }))]),
            job_with(vec![step(step::Kind::Prune(PruneStep {
                repository_id: string("repo-1"),
                max_unused: string(BAD_SHORT),
                ..PruneStep::default()
            }))]),
        ];

        for job in &jobs {
            assert!(malformed(job), "{:?}", job.steps);
        }
    }

    #[test]
    fn ac_pr_08_allows_flag_like_value_in_free_fields() {
        let steps = vec![
            step(step::Kind::Backup(BackupStep {
                repository_id: string("repo-1"),
                paths: vec![string(BAD_OPTION), string(BAD_SHORT)],
                excludes: vec![string(BAD_OPTION)],
                tags: vec![string(BAD_SHORT)],
                ..BackupStep::default()
            })),
            step(step::Kind::Ls(LsStep {
                repository_id: string("repo-1"),
                snapshot_id: string("latest"),
                path: string(BAD_OPTION),
                ..LsStep::default()
            })),
            step(step::Kind::Hook(HookStep {
                hook_id: string("pg_dump"),
                parameters: vec![HookParameter {
                    name: string("database"),
                    value: string(BAD_OPTION),
                }],
                ..HookStep::default()
            })),
        ];

        assert_eq!(validate_job(&job_with(steps)), Ok(()));
    }

    #[test]
    fn ac_pr_08_rejects_newline_in_any_field() {
        let newline = "value\nwith-newline";
        let jobs = vec![
            job_with(vec![step(step::Kind::Backup(BackupStep {
                repository_id: string("repo-1"),
                paths: vec![string(newline)],
                ..BackupStep::default()
            }))]),
            job_with(vec![step(step::Kind::Backup(BackupStep {
                repository_id: string("repo-1"),
                paths: vec![string("/srv")],
                excludes: vec![string(newline)],
                ..BackupStep::default()
            }))]),
            job_with(vec![step(step::Kind::Backup(BackupStep {
                repository_id: string("repo-1"),
                paths: vec![string("/srv")],
                tags: vec![string(newline)],
                ..BackupStep::default()
            }))]),
            job_with(vec![step(step::Kind::Ls(LsStep {
                repository_id: string("repo-1"),
                snapshot_id: string("latest"),
                path: string(newline),
                ..LsStep::default()
            }))]),
            job_with(vec![step(step::Kind::Hook(HookStep {
                hook_id: string("pg_dump"),
                parameters: vec![HookParameter {
                    name: string("database"),
                    value: string(newline),
                }],
                ..HookStep::default()
            }))]),
        ];

        for job in &jobs {
            assert!(malformed(job), "{:?}", job.steps);
        }

        let mut with_newline_repository = job_with(vec![step(step::Kind::Unlock(UnlockStep {
            repository_id: string("repo-1"),
        }))]);
        with_newline_repository.repositories[0].location = string(newline);
        assert!(malformed(&with_newline_repository));

        let with_newline_known_hosts = job_with_sftp(SftpCredentials {
            known_hosts_entry: string(newline),
            ..SftpCredentials::default()
        });
        assert!(malformed(&with_newline_known_hosts));
    }

    #[test]
    fn ac_pr_08_allows_line_breaks_in_sftp_private_key() {
        let key = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\nBBBB\r\n\
                   -----END OPENSSH PRIVATE KEY-----";
        let job = job_with_sftp(SftpCredentials {
            private_key: string(key),
            ..SftpCredentials::default()
        });

        assert_eq!(validate_job(&job), Ok(()));
    }

    #[test]
    fn ac_pr_08_rejects_nul_in_sftp_private_key() {
        let job = job_with_sftp(SftpCredentials {
            private_key: string("key\0value"),
            ..SftpCredentials::default()
        });

        assert!(malformed(&job));
    }

    #[test]
    fn ac_pr_08_accepts_empty_optional_strings() {
        let steps = vec![
            step(step::Kind::Check(CheckStep {
                repository_id: string("repo-1"),
                read_data_subset: String::new(),
                ..CheckStep::default()
            })),
            step(step::Kind::Forget(ForgetStep {
                repository_id: string("repo-1"),
                keep_within: String::new(),
                ..ForgetStep::default()
            })),
        ];

        assert_eq!(validate_job(&job_with(steps)), Ok(()));
    }

    #[test]
    fn ac_pr_08_rejects_unknown_step_kind() {
        let unknown = Step {
            step_id: string("step-1"),
            ..Step::default()
        };
        let job = job_with(vec![unknown]);

        assert_eq!(
            validate_job(&job),
            Err(reject(&job, RejectReason::UnsupportedStep))
        );
    }

    #[test]
    fn ac_pr_08_rejects_unknown_repository() {
        let job = job_with(vec![step(step::Kind::Backup(BackupStep {
            repository_id: string("repo-9"),
            paths: vec![string("/srv")],
            ..BackupStep::default()
        }))]);

        assert!(malformed(&job));
    }

    #[test]
    fn ac_pr_08_rejects_missing_required_fields() {
        let jobs = vec![
            job_with(Vec::new()),
            job_with(vec![step(step::Kind::Backup(BackupStep {
                paths: vec![string("/srv")],
                ..BackupStep::default()
            }))]),
            job_with(vec![step(step::Kind::Backup(BackupStep {
                repository_id: string("repo-1"),
                ..BackupStep::default()
            }))]),
            job_with(vec![step(step::Kind::Ls(LsStep {
                repository_id: string("repo-1"),
                ..LsStep::default()
            }))]),
            job_with(vec![step(step::Kind::Restore(RestoreStep {
                repository_id: string("repo-1"),
                target: string("/restore"),
                ..RestoreStep::default()
            }))]),
            job_with(vec![step(step::Kind::Restore(RestoreStep {
                repository_id: string("repo-1"),
                snapshot_id: string("latest"),
                ..RestoreStep::default()
            }))]),
            job_with(vec![step(step::Kind::Hook(HookStep::default()))]),
        ];

        for job in &jobs {
            assert!(malformed(job), "{:?}", job.steps);
        }
    }

    #[test]
    fn ac_pr_08_rejects_duplicate_hook_parameters() {
        let job = job_with(vec![step(step::Kind::Hook(HookStep {
            hook_id: string("pg_dump"),
            parameters: vec![
                HookParameter {
                    name: string("database"),
                    value: string("app"),
                },
                HookParameter {
                    name: string("database"),
                    value: string("other"),
                },
            ],
            ..HookStep::default()
        }))]);

        assert_eq!(
            validate_job(&job),
            Err(reject(&job, RejectReason::InvalidHookParameters))
        );
    }

    #[test]
    fn ac_pr_08_rejects_empty_or_comma_tag() {
        let bad_tags = ["", ",", "a,b", " daily", "daily ", "a\tb"];
        for bad in bad_tags {
            let jobs = [
                job_with(vec![step(step::Kind::Backup(BackupStep {
                    repository_id: string("repo-1"),
                    paths: vec![string("/srv")],
                    tags: vec![string(bad)],
                    ..BackupStep::default()
                }))]),
                job_with(vec![step(step::Kind::Snapshots(SnapshotsStep {
                    repository_id: string("repo-1"),
                    tags: vec![string(bad)],
                    ..SnapshotsStep::default()
                }))]),
                job_with(vec![step(step::Kind::Forget(ForgetStep {
                    repository_id: string("repo-1"),
                    keep_tags: vec![string(bad)],
                    ..ForgetStep::default()
                }))]),
                job_with(vec![step(step::Kind::Forget(ForgetStep {
                    repository_id: string("repo-1"),
                    filter: Some(SnapshotsFilter {
                        tags: vec![string(bad)],
                        ..SnapshotsFilter::default()
                    }),
                    ..ForgetStep::default()
                }))]),
            ];
            for job in &jobs {
                assert!(malformed(job), "{bad:?}");
            }
        }
    }

    #[test]
    fn ac_pr_08_accepts_a_plain_tag() {
        let job = job_with(vec![step(step::Kind::Backup(BackupStep {
            repository_id: string("repo-1"),
            paths: vec![string("/srv")],
            tags: vec![string("daily"), string("-x"), string("env=prod")],
            ..BackupStep::default()
        }))]);
        assert_eq!(validate_job(&job), Ok(()));
    }

    #[test]
    fn ac_pr_08_rejects_program_running_location() {
        let bad = [
            "rclone:remote:path",
            "sftp:host:/srv",
            "-x",
            "-o",
            "local:/srv/repo",
            "b2:bucket:path",
            "azure:container:/",
            "gs:bucket:/",
            "ftp://host/repo",
            "s3:ftp://host/bucket",
            "rest:ftp://host/",
            "relative/path",
            "C:relative",
        ];
        for location in bad {
            let mut job = job_with(vec![step(step::Kind::Unlock(UnlockStep {
                repository_id: string("repo-1"),
            }))]);
            job.repositories[0].location = string(location);
            assert!(malformed(&job), "{location}");
        }
    }

    #[test]
    fn ac_pr_08_accepts_a_native_location() {
        let good = [
            "s3:s3.example.com/bucket",
            "s3:https://minio.example.com/bucket",
            "s3:http://minio.local:9000/bucket",
            "rest:https://rest.example.com/",
            "rest:http://rest.local:8000/",
            "/srv/backup",
            "C:\\backup",
            "c:/backup",
        ];
        for location in good {
            let mut job = job_with(vec![step(step::Kind::Unlock(UnlockStep {
                repository_id: string("repo-1"),
            }))]);
            job.repositories[0].location = string(location);
            assert_eq!(validate_job(&job), Ok(()), "{location}");
        }
    }

    #[test]
    fn ac_pr_08_rejects_format_and_separator_characters() {
        let bad = [
            "value\u{202E}", // right-to-left override (Cf)
            "value\u{200B}", // zero width space (Cf)
            "value\u{2028}", // line separator (Zl)
            "value\u{2029}", // paragraph separator (Zp)
            "value\u{001B}", // ESC (Cc)
            "value\u{007F}", // DEL (Cc)
            "value\u{FEFF}", // byte order mark (Cf)
        ];
        for value in bad {
            let job = job_with(vec![step(step::Kind::Backup(BackupStep {
                repository_id: string("repo-1"),
                paths: vec![string(value)],
                ..BackupStep::default()
            }))]);
            assert!(malformed(&job), "{value:?}");
        }
    }
}
