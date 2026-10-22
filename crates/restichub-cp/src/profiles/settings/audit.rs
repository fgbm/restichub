//! The audit values of a profile (FR-PF-01, FR-PF-12).
//!
//! A diff names the settings and never holds a text parameter value or a secret.

use serde_json::{Value, json};

use crate::parameters::{self, ParameterDef};

use restichub_db::profiles::Profile;

use super::ValidatedProfile;

/// The settings of a validated profile for an audit diff. It holds no secret (FR-PF-01).
///
/// `previous` is the declaration set that the row held before the write.
pub fn profile_value(profile: &ValidatedProfile, previous: Option<&[ParameterDef]>) -> Value {
    json!({
        "name": profile.name,
        "paths": {
            "all": profile.paths.all,
            "windows": profile.paths.windows,
            "linux": profile.paths.linux,
            "macos": profile.paths.macos,
        },
        "excludes": profile.excludes,
        "exclude_files": profile.exclude_files,
        "tags": profile.tags,
        "options": {
            "use_fs_snapshot": profile.options.use_fs_snapshot,
            "one_file_system": profile.options.one_file_system,
            "exclude_caches": profile.options.exclude_caches,
            "limit_upload_kib": profile.options.limit_upload_kib,
            "limit_download_kib": profile.options.limit_download_kib,
            "priority": profile.options.priority,
            "stall_timeout_minutes": profile.options.stall_timeout_minutes,
            "max_duration_minutes": profile.options.max_duration_minutes,
        },
        "scripts": crate::scripts::audit_value(&profile.scripts),
        "retention": profile.retention,
        "check": profile.check,
        "parameters": parameters::audit_definitions(&profile.parameters, previous),
    })
}

/// The settings of a stored row for an audit diff.
pub fn stored_value(profile: &Profile) -> Value {
    json!({
        "name": profile.name,
        "paths": {
            "all": profile.paths_all,
            "windows": profile.paths_windows,
            "linux": profile.paths_linux,
            "macos": profile.paths_macos,
        },
        "excludes": profile.excludes,
        "exclude_files": profile.exclude_files,
        "tags": profile.tags,
        "options": {
            "use_fs_snapshot": profile.use_fs_snapshot,
            "one_file_system": profile.one_file_system,
            "exclude_caches": profile.exclude_caches,
            "limit_upload_kib": profile.limit_upload_kib,
            "limit_download_kib": profile.limit_download_kib,
            "priority": profile.priority,
            "stall_timeout_minutes": profile.stall_timeout_minutes,
            "max_duration_minutes": profile.max_duration_minutes,
        },
        "scripts": stored_scripts_value(&profile.scripts),
        "retention": profile.retention,
        "check": profile.check_options,
        "parameters": parameters::stored_value(&profile.parameters),
    })
}

/// The stored scripts as the audit diff shows them. A row that does not parse shows as null.
fn stored_scripts_value(scripts: &Value) -> Value {
    crate::scripts::from_row(scripts)
        .map_or(Value::Null, |scripts| crate::scripts::audit_value(&scripts))
}
