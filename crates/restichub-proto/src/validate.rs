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
    HookPhase, HookStep, Job, OverwriteMode, RejectReason, RepositoryAccess, ScriptFailurePolicy,
    ScriptInterpreter, ScriptStep, SnapshotsFilter, Step, hook_parameter::Typed,
    repository_access::BackendCredentials, script_env, step,
};
use crate::signing::Rejection;

/// The most parameters that one hook step carries (FR-HK-02).
pub const MAX_HOOK_PARAMETERS: usize = 32;

/// The most bytes in the body of a script (FR-HK-09).
pub const MAX_SCRIPT_BODY_BYTES: usize = 64 * 1024;

pub const MAX_SCRIPT_NAME_CHARS: usize = 128;

/// The most environment variables that one script step carries.
pub const MAX_SCRIPT_ENV: usize = 64;

/// The most bytes in the value of a script variable.
pub const MAX_SCRIPT_ENV_VALUE_BYTES: usize = 8 * 1024;

/// The most nodes that one `ls` step returns. The result of a step must fit in a protocol message
/// (AC-RS-02).
pub const MAX_LS_LIMIT: u64 = 5_000;

/// Validates a job against the typed contract (FR-PR-10).
///
/// # Errors
///
/// Returns a [`Rejection`] with [`RejectReason::UnsupportedStep`] for an
/// unknown step kind, [`RejectReason::InvalidHookParameters`] for a duplicate
/// hook parameter name, [`RejectReason::InvalidScript`] for a script step that breaks a rule, and
/// [`RejectReason::Malformed`] for any other rule.
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

    // `unlock --remove-all` removes the locks of a live backup, so it never shares a job with
    // another step (FR-SC-06).
    if job.steps.len() > 1
        && job
            .steps
            .iter()
            .any(|step| matches!(step.kind, Some(step::Kind::UnlockAll(_))))
    {
        return Err(reject(job, RejectReason::Malformed));
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
            check_required_format(job, &ls.snapshot_id, is_concrete_snapshot_id)?;
            check_free(job, std::iter::once(&ls.path))?;
            if ls.limit > MAX_LS_LIMIT {
                return Err(reject(job, RejectReason::Malformed));
            }
        }
        step::Kind::Restore(restore) => {
            check_repository_id(job, repository_ids, &restore.repository_id)?;
            check_required_format(job, &restore.snapshot_id, is_concrete_snapshot_id)?;
            check_required_free(job, &restore.target)?;
            if !is_restore_target(&restore.target) {
                return Err(reject(job, RejectReason::Malformed));
            }
            check_patterns(job, restore.includes.iter())?;
            check_patterns(job, restore.excludes.iter())?;
            // restic stops with "exclude and include patterns are mutually exclusive".
            if !restore.includes.is_empty() && !restore.excludes.is_empty() {
                return Err(reject(job, RejectReason::Malformed));
            }
            // An unset policy would leave the choice to the restic default.
            if !matches!(
                OverwriteMode::try_from(restore.overwrite),
                Ok(mode) if mode != OverwriteMode::Unspecified
            ) {
                return Err(reject(job, RejectReason::Malformed));
            }
            // restic stops with "--dry-run and --verify are mutually exclusive".
            if restore.dry_run && restore.verify {
                return Err(reject(job, RejectReason::Malformed));
            }
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
            // A forget without a tag filter reaches every profile of the repository (FR-RP-08).
            let filter = forget
                .filter
                .as_ref()
                .filter(|filter| !filter.tags.is_empty() || !filter.all_tags.is_empty())
                .ok_or_else(|| reject(job, RejectReason::Malformed))?;
            check_filter(job, filter)?;
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
        step::Kind::Script(script) => {
            check_script(job, script)?;
        }
    }

    Ok(())
}

fn check_filter(job: &Job, filter: &SnapshotsFilter) -> Result<(), Rejection> {
    check_free(job, filter.hosts.iter())?;
    check_tags(job, filter.tags.iter())?;
    check_tags(job, filter.all_tags.iter())?;
    check_free(job, filter.paths.iter())
}

fn check_hook(job: &Job, hook: &HookStep) -> Result<(), Rejection> {
    if hook.hook_id.is_empty() || has_forbidden(&hook.hook_id) {
        return Err(reject(job, RejectReason::Malformed));
    }

    if hook.parameters.len() > MAX_HOOK_PARAMETERS {
        return Err(reject(job, RejectReason::InvalidHookParameters));
    }
    let mut names = HashSet::new();
    for parameter in &hook.parameters {
        if has_forbidden(&parameter.name) {
            return Err(reject(job, RejectReason::Malformed));
        }
        // Field 2 held an untyped value. A value must have a type (FR-HK-03).
        if !parameter.value.is_empty() || parameter.typed.is_none() {
            return Err(reject(job, RejectReason::InvalidHookParameters));
        }
        // A secret may hold line breaks, such as a key. Only NUL cannot travel in an environment
        // variable.
        let invalid = match &parameter.typed {
            Some(Typed::StringValue(value)) => has_forbidden(value),
            Some(Typed::SecretValue(value)) => has_nul(value),
            Some(Typed::IntegerValue(_) | Typed::BooleanValue(_)) | None => false,
        };
        if invalid {
            return Err(reject(job, RejectReason::Malformed));
        }
        if !names.insert(parameter.name.as_str()) {
            return Err(reject(job, RejectReason::InvalidHookParameters));
        }
    }

    Ok(())
}

/// True for a name that a script variable may have. The agent runs the interpreter with these
/// variables, so a name that changes how the loader or a shell starts is not allowed (FR-HK-09).
#[must_use]
pub fn is_script_env_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') || name.len() > 64 {
        return false;
    }
    if !characters.all(|character| character.is_ascii_alphanumeric() || character == '_') {
        return false;
    }
    let upper = name.to_ascii_uppercase();
    const RESERVED: [&str; 14] = [
        "PATH",
        "BASH_ENV",
        "ENV",
        "IFS",
        "SHELLOPTS",
        "BASHOPTS",
        "PS4",
        "PROMPT_COMMAND",
        "PSMODULEPATH",
        "SYSTEMROOT",
        "COMSPEC",
        "TEMP",
        "TMP",
        "HOME",
    ];
    const RESERVED_PREFIXES: [&str; 5] = ["LD_", "DYLD_", "RESTICHUB_", "RESTIC_", "PYTHON"];
    !RESERVED.contains(&upper.as_str())
        && !RESERVED_PREFIXES
            .iter()
            .any(|prefix| upper.starts_with(prefix))
}

fn check_script(job: &Job, script: &ScriptStep) -> Result<(), Rejection> {
    let invalid = || reject(job, RejectReason::InvalidScript);
    if script.name.is_empty()
        || script.name.chars().count() > MAX_SCRIPT_NAME_CHARS
        || has_forbidden(&script.name)
    {
        return Err(invalid());
    }
    if script.body.trim().is_empty()
        || script.body.len() > MAX_SCRIPT_BODY_BYTES
        || has_nul(&script.body)
    {
        return Err(invalid());
    }
    let interpreter =
        ScriptInterpreter::try_from(script.interpreter).unwrap_or(ScriptInterpreter::Unspecified);
    let phase = HookPhase::try_from(script.phase).unwrap_or(HookPhase::Unspecified);
    let policy = ScriptFailurePolicy::try_from(script.failure_policy)
        .unwrap_or(ScriptFailurePolicy::Unspecified);
    if interpreter == ScriptInterpreter::Unspecified
        || phase == HookPhase::Unspecified
        || policy == ScriptFailurePolicy::Unspecified
    {
        return Err(invalid());
    }
    if script.env.len() > MAX_SCRIPT_ENV {
        return Err(invalid());
    }
    let mut names = HashSet::new();
    for variable in &script.env {
        if !is_script_env_name(&variable.name) || !names.insert(variable.name.to_ascii_uppercase())
        {
            return Err(invalid());
        }
        let valid = match &variable.value {
            Some(script_env::Value::StringValue(value)) => {
                value.len() <= MAX_SCRIPT_ENV_VALUE_BYTES && !has_forbidden(value)
            }
            Some(script_env::Value::SecretValue(value)) => {
                value.len() <= MAX_SCRIPT_ENV_VALUE_BYTES && !has_nul(value)
            }
            None => false,
        };
        if !valid {
            return Err(invalid());
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

/// Rejects an empty pattern: `--include=` would select nothing or everything, by restic version.
fn check_patterns<'a>(
    job: &Job,
    values: impl IntoIterator<Item = &'a String>,
) -> Result<(), Rejection> {
    for value in values {
        if value.is_empty() || has_forbidden(value) {
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

/// Returns true for a path that starts at a root: `/`, or a drive letter and a separator.
pub fn is_absolute_path(value: &str) -> bool {
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

/// Returns true for a snapshot identifier that names one snapshot. `latest` moves between two
/// reads, so a restore and a listing never accept it.
fn is_concrete_snapshot_id(value: &str) -> bool {
    value != "latest" && is_snapshot_id(value)
}

/// Returns true for an absolute target path without a `.` or `..` component, in either syntax.
fn is_restore_target(value: &str) -> bool {
    use crate::restore_path::{PathFlavor, components};
    components(PathFlavor::Posix, value).is_some()
        || components(PathFlavor::Windows, value).is_some()
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

    fn hook_job(parameter: HookParameter) -> Job {
        job_with(vec![step(step::Kind::Hook(HookStep {
            hook_id: string("pg_dump"),
            parameters: vec![parameter],
            ..HookStep::default()
        }))])
    }

    #[test]
    fn fr_hk_03_rejects_an_untyped_hook_parameter() {
        let untyped = hook_job(HookParameter {
            name: string("database"),
            value: string("app"),
            typed: None,
        });
        assert_eq!(
            validate_job(&untyped),
            Err(reject(&untyped, RejectReason::InvalidHookParameters))
        );
        let both = hook_job(HookParameter {
            name: string("database"),
            value: string("app"),
            typed: Some(Typed::StringValue(string("app"))),
        });
        assert_eq!(
            validate_job(&both),
            Err(reject(&both, RejectReason::InvalidHookParameters))
        );
        let none = hook_job(HookParameter {
            name: string("database"),
            ..HookParameter::default()
        });
        assert_eq!(
            validate_job(&none),
            Err(reject(&none, RejectReason::InvalidHookParameters))
        );
    }

    #[test]
    fn fr_hk_03_accepts_every_hook_value_type_and_rejects_a_control_character() {
        for typed in [
            Typed::StringValue(string("app")),
            Typed::IntegerValue(-7),
            Typed::BooleanValue(true),
            Typed::SecretValue(string("s3cret")),
        ] {
            let job = hook_job(HookParameter {
                name: string("p"),
                typed: Some(typed),
                ..HookParameter::default()
            });
            assert_eq!(validate_job(&job), Ok(()));
        }
        for typed in [
            Typed::StringValue(string("a\nb")),
            Typed::SecretValue(string("a\u{0}b")),
        ] {
            let job = hook_job(HookParameter {
                name: string("p"),
                typed: Some(typed),
                ..HookParameter::default()
            });
            assert!(malformed(&job));
        }
    }

    #[test]
    fn fr_hk_02_rejects_too_many_hook_parameters() {
        let parameters = (0..=MAX_HOOK_PARAMETERS)
            .map(|index| HookParameter {
                name: format!("p{index}"),
                typed: Some(Typed::BooleanValue(true)),
                ..HookParameter::default()
            })
            .collect();
        let job = job_with(vec![step(step::Kind::Hook(HookStep {
            hook_id: string("pg_dump"),
            parameters,
            ..HookStep::default()
        }))]);
        assert_eq!(
            validate_job(&job),
            Err(reject(&job, RejectReason::InvalidHookParameters))
        );
    }

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

    fn script(body: &str) -> ScriptStep {
        ScriptStep {
            name: string("dump"),
            interpreter: ScriptInterpreter::Bash as i32,
            phase: HookPhase::Before as i32,
            body: string(body),
            failure_policy: ScriptFailurePolicy::AbortBackup as i32,
            env: Vec::new(),
        }
    }

    fn script_job(script: ScriptStep) -> Job {
        job_with(vec![step(step::Kind::Script(script))])
    }

    #[test]
    fn fr_hk_09_accepts_a_valid_script_and_rejects_every_broken_rule() {
        assert_eq!(validate_job(&script_job(script("echo ok\n"))), Ok(()));
        let invalid = |script: ScriptStep| {
            let job = script_job(script);
            assert_eq!(
                validate_job(&job),
                Err(reject(&job, RejectReason::InvalidScript))
            );
        };
        invalid(script("  \n"));
        invalid(script(&"x".repeat(MAX_SCRIPT_BODY_BYTES + 1)));
        invalid(script("echo\0"));
        invalid(ScriptStep {
            name: String::new(),
            ..script("echo")
        });
        invalid(ScriptStep {
            name: "a\nb".to_owned(),
            ..script("echo")
        });
        invalid(ScriptStep {
            interpreter: 0,
            ..script("echo")
        });
        invalid(ScriptStep {
            phase: 0,
            ..script("echo")
        });
        invalid(ScriptStep {
            failure_policy: 0,
            ..script("echo")
        });
        let variable = |name: &str, value: Option<script_env::Value>| ScriptStep {
            env: vec![crate::agent::v1::ScriptEnv {
                name: name.to_owned(),
                value,
            }],
            ..script("echo")
        };
        let plain = || Some(script_env::Value::StringValue(string("x")));
        for name in ["", "1A", "A-B", "PATH", "path", "LD_PRELOAD", "BASH_ENV"] {
            invalid(variable(name, plain()));
        }
        invalid(variable("PGHOST", None));
        invalid(variable(
            "PGHOST",
            Some(script_env::Value::StringValue(string("a\nb"))),
        ));
        let mut twice = variable("PGHOST", plain());
        twice.env.push(twice.env[0].clone());
        invalid(twice);
        let accepted = variable(
            "PGPASSWORD",
            Some(script_env::Value::SecretValue(string("line1\nline2"))),
        );
        assert_eq!(validate_job(&script_job(accepted)), Ok(()));
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

    fn tagged() -> SnapshotsFilter {
        SnapshotsFilter {
            tags: vec![string("restichub-profile-a")],
            ..SnapshotsFilter::default()
        }
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
                snapshot_id: string("0123abcd"),
                path: string("/srv"),
                ..LsStep::default()
            })),
            step(step::Kind::Restore(RestoreStep {
                repository_id: string("repo-1"),
                snapshot_id: string("0123abcd"),
                target: string("/restore"),
                overwrite: OverwriteMode::Never as i32,
                ..RestoreStep::default()
            })),
            step(step::Kind::Unlock(UnlockStep {
                repository_id: string("repo-1"),
            })),
            step(step::Kind::Forget(ForgetStep {
                repository_id: string("repo-1"),
                filter: Some(tagged()),
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
                    typed: Some(Typed::StringValue(string("app"))),
                    ..HookParameter::default()
                }],
                ..HookStep::default()
            })),
        ];

        assert_eq!(validate_job(&job_with(steps)), Ok(()));
    }

    #[test]
    fn fr_rp_08_rejects_a_forget_without_a_tag_filter() {
        let filters = [
            None,
            Some(SnapshotsFilter::default()),
            Some(SnapshotsFilter {
                hosts: vec![string("host-a")],
                paths: vec![string("/srv")],
                ..SnapshotsFilter::default()
            }),
        ];
        for filter in filters {
            let job = job_with(vec![step(step::Kind::Forget(ForgetStep {
                repository_id: string("repo-1"),
                filter,
                ..ForgetStep::default()
            }))]);
            assert!(malformed(&job), "{:?}", job.steps);
        }
        for filter in [
            tagged(),
            SnapshotsFilter {
                all_tags: vec![string("restichub-profile-a"), string("daily")],
                ..SnapshotsFilter::default()
            },
        ] {
            let job = job_with(vec![step(step::Kind::Forget(ForgetStep {
                repository_id: string("repo-1"),
                filter: Some(filter),
                ..ForgetStep::default()
            }))]);
            assert_eq!(validate_job(&job), Ok(()));
        }
    }

    #[test]
    fn fr_sc_06_unlock_all_runs_only_alone() {
        let unlock_all = || {
            step(step::Kind::UnlockAll(UnlockAllStep {
                repository_id: string("repo-1"),
            }))
        };
        let unlock = step(step::Kind::Unlock(UnlockStep {
            repository_id: string("repo-1"),
        }));
        assert_eq!(validate_job(&job_with(vec![unlock_all()])), Ok(()));
        assert!(malformed(&job_with(vec![unlock.clone(), unlock_all()])));
        assert!(malformed(&job_with(vec![unlock_all(), unlock])));
        assert!(malformed(&job_with(vec![unlock_all(), unlock_all()])));
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
                filter: Some(tagged()),
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
                filter: Some(tagged()),
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
                snapshot_id: string("0123abcd"),
                path: string(BAD_OPTION),
                ..LsStep::default()
            })),
            step(step::Kind::Hook(HookStep {
                hook_id: string("pg_dump"),
                parameters: vec![HookParameter {
                    name: string("database"),
                    typed: Some(Typed::StringValue(string(BAD_OPTION))),
                    ..HookParameter::default()
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
                snapshot_id: string("0123abcd"),
                path: string(newline),
                ..LsStep::default()
            }))]),
            job_with(vec![step(step::Kind::Hook(HookStep {
                hook_id: string("pg_dump"),
                parameters: vec![HookParameter {
                    name: string("database"),
                    typed: Some(Typed::StringValue(string(newline))),
                    ..HookParameter::default()
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
                filter: Some(tagged()),
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
                    typed: Some(Typed::StringValue(string("app"))),
                    ..HookParameter::default()
                },
                HookParameter {
                    name: string("database"),
                    typed: Some(Typed::StringValue(string("other"))),
                    ..HookParameter::default()
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
                    filter: Some(tagged()),
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
                job_with(vec![step(step::Kind::Forget(ForgetStep {
                    repository_id: string("repo-1"),
                    filter: Some(SnapshotsFilter {
                        all_tags: vec![string(bad)],
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
    #[test]
    fn ac_rs_03_rejects_a_restore_or_ls_step_outside_its_contract() {
        let restore = |change: fn(&mut RestoreStep)| {
            let mut restore = RestoreStep {
                repository_id: string("repo-1"),
                snapshot_id: string("0123abcd"),
                target: string("/restore"),
                overwrite: OverwriteMode::Never as i32,
                ..RestoreStep::default()
            };
            change(&mut restore);
            job_with(vec![step(step::Kind::Restore(restore))])
        };
        assert_eq!(validate_job(&restore(|_| {})), Ok(()));
        assert!(malformed(&restore(|r| r.target = string("restore"))));
        assert!(malformed(&restore(|r| r.target = string("../restore"))));
        assert!(malformed(&restore(|r| r.includes = vec![string("")])));
        assert!(malformed(&restore(|r| r.excludes = vec![string("")])));
        assert!(malformed(&restore(|r| r.overwrite = 99)));
        assert!(malformed(&restore(|r| {
            r.overwrite = OverwriteMode::Unspecified as i32;
        })));
        assert!(malformed(&restore(|r| r.snapshot_id = string("latest"))));
        assert!(malformed(&restore(|r| {
            r.dry_run = true;
            r.verify = true;
        })));
        assert_eq!(
            validate_job(&restore(|r| r.dry_run = true)),
            Ok(()),
            "a dry run alone is valid"
        );
        assert!(malformed(&restore(|r| r.target = string("/srv/../etc"))));
        assert!(malformed(&restore(
            |r| r.target = string("C:\\srv\\..\\etc")
        )));
        assert!(malformed(&restore(|r| {
            r.includes = vec![string("/srv")];
            r.excludes = vec![string("*.log")];
        })));
        assert_eq!(
            validate_job(&restore(|r| r.target = string("C:\\restore"))),
            Ok(())
        );
        let ls = |path: &str, limit: u64| {
            job_with(vec![step(step::Kind::Ls(LsStep {
                repository_id: string("repo-1"),
                snapshot_id: string("0123abcd"),
                path: string(path),
                limit,
                ..LsStep::default()
            }))])
        };
        assert_eq!(validate_job(&ls("", 0)), Ok(()));
        let latest = job_with(vec![step(step::Kind::Ls(LsStep {
            repository_id: string("repo-1"),
            snapshot_id: string("latest"),
            ..LsStep::default()
        }))]);
        assert!(malformed(&latest), "a listing names one snapshot");
        assert_eq!(validate_job(&ls("/C/Users", MAX_LS_LIMIT)), Ok(()));
        assert!(malformed(&ls("/etc", MAX_LS_LIMIT + 1)));
    }
}
