//! The validation of the settings of a profile (FR-PF-01, FR-PF-05, FR-PF-09).
//!
//! A field error names the request field and a stable reason code. A value that a caller stores is
//! the value that these functions return.

use crate::error::ApiError;
use crate::parameters::{self, ParameterDef, ParameterDefRequest};
use crate::scripts::ScriptDef;
use crate::validation::{self, Reason};

use super::{
    BackupOptions, CheckOptions, CreateProfileRequest, MAX_DURATION_MINUTES, MAX_EXCLUDE_FILES,
    MAX_EXCLUDES, MAX_PATH_ENTRY, MAX_PATHS, MAX_PATTERN, MAX_RETENTION_LIST,
    MAX_STALL_TIMEOUT_MINUTES, MAX_TAGS, MIN_STALL_TIMEOUT_MINUTES, ProfilePaths,
    RESERVED_TAG_PREFIX, Retention, SnapshotFilter, UpdateProfileRequest, ValidatedProfile,
};

fn validate_limits(options: &BackupOptions) -> Result<(), ApiError> {
    if let Some(minutes) = options.stall_timeout_minutes {
        if minutes < MIN_STALL_TIMEOUT_MINUTES {
            return Err(ApiError::invalid_field(
                "options.stall_timeout_minutes",
                Reason::TooShort,
            ));
        }
        if minutes > MAX_STALL_TIMEOUT_MINUTES {
            return Err(ApiError::invalid_field(
                "options.stall_timeout_minutes",
                Reason::TooLong,
            ));
        }
    }
    if options
        .max_duration_minutes
        .is_some_and(|minutes| minutes > MAX_DURATION_MINUTES)
    {
        return Err(ApiError::invalid_field(
            "options.max_duration_minutes",
            Reason::TooLong,
        ));
    }
    Ok(())
}

/// Validates a create request and returns the normalized settings (FR-PF-05).
pub fn validate_create(body: CreateProfileRequest) -> Result<ValidatedProfile, ApiError> {
    reject_hooks(body.hooks.as_deref())?;
    validate_profile(
        Some(body.name),
        body.paths,
        body.excludes,
        body.exclude_files,
        body.tags,
        body.options,
        body.scripts,
        body.retention,
        body.check,
        body.parameters,
    )
}

/// Validates an update request. `current_name` is used when the body holds no name.
pub fn validate_update(
    body: UpdateProfileRequest,
    current_name: &str,
) -> Result<ValidatedProfile, ApiError> {
    reject_hooks(body.hooks.as_deref())?;
    let name = body.name.unwrap_or_else(|| current_name.to_owned());
    validate_profile(
        Some(name),
        body.paths,
        body.excludes,
        body.exclude_files,
        body.tags,
        body.options,
        body.scripts,
        body.retention,
        body.check,
        body.parameters,
    )
}

fn reject_hooks(hooks: Option<&[serde_json::Value]>) -> Result<(), ApiError> {
    match hooks {
        Some([_, ..]) => Err(ApiError::invalid_field("hooks", Reason::Removed)),
        _ => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_profile(
    name: Option<String>,
    paths: ProfilePaths,
    excludes: Vec<String>,
    exclude_files: Vec<String>,
    tags: Vec<String>,
    options: BackupOptions,
    scripts: Vec<ScriptDef>,
    retention: Option<Retention>,
    check: Option<CheckOptions>,
    parameters: Vec<ParameterDefRequest>,
) -> Result<ValidatedProfile, ApiError> {
    let name = crate::validation::validate_record_name("name", &name.unwrap_or_default())?;
    let paths = validate_paths(paths)?;
    let excludes = validate_patterns("excludes", excludes)?;
    ensure_count("excludes", excludes.len(), MAX_EXCLUDES)?;
    let exclude_files = validate_patterns("exclude_files", exclude_files)?;
    ensure_count("exclude_files", exclude_files.len(), MAX_EXCLUDE_FILES)?;
    let tags = validate_tags("tags", tags)?;
    ensure_count("tags", tags.len(), MAX_TAGS)?;
    validate_limits(&options)?;
    let parameters = crate::parameters::validate_definitions("parameters", parameters)?;
    let scripts = crate::scripts::validate_scripts(scripts, "scripts")?;
    crate::parameters::check_script_parameters(&parameters, &scripts)?;
    let retention = match retention {
        Some(retention) => Some(validate_retention(retention)?),
        None => None,
    };
    let check = match check {
        Some(check) => Some(validate_check(check)?),
        None => None,
    };
    check_templates(&parameters, &paths, &excludes, &exclude_files, &tags)?;
    Ok(ValidatedProfile {
        name,
        paths,
        excludes,
        exclude_files,
        tags,
        options,
        scripts,
        retention,
        check,
        parameters,
    })
}

/// Refuses a template of the profile that a parameter can never fill (FR-PF-05, FR-PF-10).
///
/// A secret parameter cannot expand in a path, an exclude, an exclude file or a tag (FR-PF-12). An
/// optional text parameter without a default is the empty string, and an empty path or tag is
/// invalid, so the profile does not save (owner decision, ADR-0047).
fn check_templates(
    parameters: &[ParameterDef],
    paths: &ProfilePaths,
    excludes: &[String],
    exclude_files: &[String],
    tags: &[String],
) -> Result<(), ApiError> {
    let mut fields: Vec<(String, &String)> = Vec::new();
    let lists: [(&str, &[String]); 7] = [
        ("paths.all", &paths.all),
        ("paths.windows", &paths.windows),
        ("paths.linux", &paths.linux),
        ("paths.macos", &paths.macos),
        ("excludes", excludes),
        ("exclude_files", exclude_files),
        ("tags", tags),
    ];
    for (field, values) in lists {
        for (index, value) in values.iter().enumerate() {
            fields.push((format!("{field}[{index}]"), value));
        }
    }
    parameters::check_templates(
        parameters,
        fields.iter().map(|(field, value)| (field.as_str(), *value)),
    )
}

fn validate_paths(paths: ProfilePaths) -> Result<ProfilePaths, ApiError> {
    let all = validate_path_entries("paths.all", paths.all)?;
    let windows = validate_path_entries("paths.windows", paths.windows)?;
    let linux = validate_path_entries("paths.linux", paths.linux)?;
    let macos = validate_path_entries("paths.macos", paths.macos)?;
    let total = all.len() + windows.len() + linux.len() + macos.len();
    if total > MAX_PATHS {
        return Err(ApiError::invalid_field("paths", Reason::TooLong));
    }
    Ok(ProfilePaths {
        all,
        windows,
        linux,
        macos,
    })
}

/// A path entry is absolute: it starts with `/` or a drive letter (FR-PF-01). A template variable
/// is allowed inside the path (FR-PF-03). A root path (`/` or `C:\`) is allowed and one trailing
/// separator is dropped, so `/srv/` becomes `/srv`.
fn validate_path_entries(
    field: &'static str,
    values: Vec<String>,
) -> Result<Vec<String>, ApiError> {
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        result.push(validate_path_entry(field, &value)?);
    }
    Ok(result)
}

fn validate_path_entry(field: &'static str, value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value == "/" {
        return Ok("/".to_owned());
    }
    let bytes = value.as_bytes();
    if bytes.len() == 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
    {
        return Ok(format!("{}:\\", char::from(bytes[0])));
    }
    let trimmed = value
        .strip_suffix('/')
        .or_else(|| value.strip_suffix('\\'))
        .unwrap_or(value);
    let validated = validation::validate_local_path(field, trimmed)?;
    if validated.chars().count() > MAX_PATH_ENTRY {
        return Err(ApiError::invalid_field(field, Reason::TooLong));
    }
    Ok(validated)
}

/// An exclude, an exclude file or a tag holds no control character (FR-PF-01).
fn validate_patterns(field: &'static str, values: Vec<String>) -> Result<Vec<String>, ApiError> {
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            return Err(ApiError::invalid_field(field, Reason::Empty));
        }
        if value.chars().count() > MAX_PATTERN {
            return Err(ApiError::invalid_field(field, Reason::TooLong));
        }
        if restichub_proto::validate::has_forbidden(value) || value.chars().any(char::is_control) {
            return Err(ApiError::invalid_field(field, Reason::InvalidCharacter));
        }
        result.push(value.to_owned());
    }
    Ok(result)
}

/// A tag holds no comma and no surrounding space: restic reads these forms differently (AC-PR-08).
fn validate_tags(field: &'static str, values: Vec<String>) -> Result<Vec<String>, ApiError> {
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(ApiError::invalid_field(field, Reason::Empty));
        }
        if trimmed.chars().count() > MAX_PATTERN || trimmed.contains(',') {
            return Err(ApiError::invalid_field(field, Reason::InvalidFormat));
        }
        if restichub_proto::validate::has_forbidden(trimmed) {
            return Err(ApiError::invalid_field(field, Reason::InvalidCharacter));
        }
        // The maintenance job isolates the profiles with the tags of this prefix (FR-RP-08).
        if trimmed
            .to_ascii_lowercase()
            .starts_with(RESERVED_TAG_PREFIX)
        {
            return Err(ApiError::invalid_field(field, Reason::Reserved));
        }
        result.push(trimmed.to_owned());
    }
    Ok(result)
}

fn validate_retention(retention: Retention) -> Result<Retention, ApiError> {
    // A parameter cannot expand here, so the refusal names the rule and not the format (ADR-0034).
    parameters::check_maintenance(&retention.keep_within, "retention.keep_within")?;
    for (index, tag) in retention.keep_tags.iter().enumerate() {
        parameters::check_maintenance(tag, &format!("retention.keep_tags[{index}]"))?;
    }
    for (index, host) in retention.filter.hosts.iter().enumerate() {
        parameters::check_maintenance(host, &format!("retention.filter.hosts[{index}]"))?;
    }
    for (index, tag) in retention.filter.tags.iter().enumerate() {
        parameters::check_maintenance(tag, &format!("retention.filter.tags[{index}]"))?;
    }
    for (index, path) in retention.filter.paths.iter().enumerate() {
        parameters::check_maintenance(path, &format!("retention.filter.paths[{index}]"))?;
    }
    if !retention.keep_within.is_empty() && !is_duration(&retention.keep_within) {
        return Err(ApiError::invalid_field(
            "retention.keep_within",
            Reason::InvalidFormat,
        ));
    }
    let keep_tags = validate_tags("retention.keep_tags", retention.keep_tags)?;
    ensure_count("retention.keep_tags", keep_tags.len(), MAX_RETENTION_LIST)?;
    // The maintenance step of a profile covers every agent of its plans, so a host name that
    // depends on one agent has no single meaning (ADR-0034).
    if retention.filter.hosts.iter().any(|host| host.contains('{')) {
        return Err(ApiError::invalid_field(
            "retention.filter.hosts",
            Reason::InvalidFormat,
        ));
    }
    let filter = SnapshotFilter {
        hosts: validate_patterns("retention.filter.hosts", retention.filter.hosts)?,
        tags: validate_tags("retention.filter.tags", retention.filter.tags)?,
        paths: validate_patterns("retention.filter.paths", retention.filter.paths)?,
    };
    ensure_count(
        "retention.filter.hosts",
        filter.hosts.len(),
        MAX_RETENTION_LIST,
    )?;
    ensure_count(
        "retention.filter.tags",
        filter.tags.len(),
        MAX_RETENTION_LIST,
    )?;
    ensure_count(
        "retention.filter.paths",
        filter.paths.len(),
        MAX_RETENTION_LIST,
    )?;
    Ok(Retention {
        keep_tags,
        filter,
        ..retention
    })
}

/// Refuses a list that is longer than the limit (FR-PF-05).
fn ensure_count(field: &'static str, length: usize, maximum: usize) -> Result<(), ApiError> {
    if length > maximum {
        return Err(ApiError::invalid_field(field, Reason::TooLong));
    }
    Ok(())
}

fn validate_check(check: CheckOptions) -> Result<CheckOptions, ApiError> {
    parameters::check_maintenance(&check.read_data_subset, "check.read_data_subset")?;
    if !check.read_data_subset.is_empty() && !is_read_data_subset(&check.read_data_subset) {
        return Err(ApiError::invalid_field(
            "check.read_data_subset",
            Reason::InvalidFormat,
        ));
    }
    Ok(check)
}

/// A restic duration, for example `30d`, `2w` or `1y2m3d4h`.
fn is_duration(value: &str) -> bool {
    let mut digits = false;
    for character in value.chars() {
        if character.is_ascii_digit() {
            digits = true;
        } else if !matches!(character, 'y' | 'm' | 'd' | 'h' | 'w') {
            return false;
        }
    }
    digits
}

/// The `read-data-subset` form: `N%` or `N/M` (FR-PF-01).
fn is_read_data_subset(value: &str) -> bool {
    if let Some(percent) = value.strip_suffix('%') {
        return percent.parse::<u32>().is_ok();
    }
    match value.split_once('/') {
        Some((part, total)) => part.parse::<u32>().is_ok() && total.parse::<u32>().is_ok(),
        None => false,
    }
}
