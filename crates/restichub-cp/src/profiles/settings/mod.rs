//! The typed profile settings, the validation and the compiler mapping.
//!
//! The module holds the API types of the profile endpoints, the validation that runs before a
//! write (FR-PF-05) and the mapping to the compiler types (FR-PF-04). The handlers live in the
//! parent module.
//!
//! The profile holds no secret. The repository password and the backend credentials stay in the
//! `secrets` table (ADR-0017). A script variable of the kind `secret_ref` holds the identifier of a
//! secret, never the value (FR-HK-09); the job builder resolves it at dispatch time.

mod audit;
mod save;
mod validate;

pub use audit::{profile_value, stored_value};
pub use save::save_warnings;
pub(crate) use validate::{reject_hooks, validate_profile};
pub use validate::{validate_create, validate_update};

use std::collections::BTreeMap;

use restichub_compiler::{
    BackupOptions as CompilerBackupOptions, CheckOptions as CompilerCheckOptions,
    GroupBy as CompilerGroupBy, JobLimits as CompilerJobLimits, OperatingSystem,
    Profile as CompilerProfile, ProfilePaths as CompilerProfilePaths,
    Retention as CompilerRetention, SnapshotFilter as CompilerSnapshotFilter,
};
use restichub_db::profiles::{NewProfile, Profile, ProfileUpdate};
use restichub_db::{ProfileId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::parameters::{self, ParameterDef, ParameterDefRequest, ParameterDefView};
use crate::scripts::{EnvKind, ScriptDef};
use crate::validation::Reason;

/// Limit in characters of one exclude, exclude file or tag.
pub const MAX_PATTERN: usize = 4096;
/// Limit in characters of one path entry. A Windows path is longer than a Unix path.
pub const MAX_PATH_ENTRY: usize = 4096;
/// Limit of the path entries of a profile, across every operating system.
pub const MAX_PATHS: usize = 256;
/// Limit of the exclude patterns.
pub const MAX_EXCLUDES: usize = 256;
/// Limit of the exclude files.
pub const MAX_EXCLUDE_FILES: usize = 32;
/// Limit of the tags.
pub const MAX_TAGS: usize = 64;
/// Limit of the retention lists: the keep tags, the group-by keys and the snapshot filter.
pub const MAX_RETENTION_LIST: usize = 64;

/// The source paths for each operating system (FR-PF-01).
///
/// `all` applies to every operating system. `windows`, `linux` and `macos` add the paths of one
/// operating system.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfilePaths {
    #[serde(default)]
    pub all: Vec<String>,
    #[serde(default)]
    pub windows: Vec<String>,
    #[serde(default)]
    pub linux: Vec<String>,
    #[serde(default)]
    pub macos: Vec<String>,
}

/// The restic flags that a profile sets (FR-PF-01).
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BackupOptions {
    /// Use a file system snapshot. VSS on Windows only (FR-PF-01).
    #[serde(default)]
    pub use_fs_snapshot: bool,
    #[serde(default)]
    pub one_file_system: bool,
    #[serde(default)]
    pub exclude_caches: bool,
    /// Upload limit in kibibytes each second. Zero means no limit (FR-AG-15).
    #[serde(default)]
    pub limit_upload_kib: u64,
    /// Download limit in kibibytes each second. Zero means no limit (FR-AG-15).
    #[serde(default)]
    pub limit_download_kib: u64,
    /// Minutes without progress after which the agent stops a step. From 5 to 10080. A missing
    /// value gives the default of a new profile and keeps the stored value of an update (ADR-0039).
    #[serde(default)]
    pub stall_timeout_minutes: Option<u32>,
    /// Minutes after which the agent stops the job whatever its progress. Zero means no limit.
    /// At most 43200. A missing value behaves as the stall timeout does (ADR-0039).
    #[serde(default)]
    pub max_duration_minutes: Option<u32>,
}

/// The job limits that a new profile gets when the request names none (ADR-0039).
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
pub struct JobLimitDefaults {
    pub stall_timeout_minutes: u32,
    pub max_duration_minutes: u32,
}

impl JobLimitDefaults {
    #[must_use]
    pub fn current() -> Self {
        Self {
            stall_timeout_minutes: default_stall_timeout_minutes(),
            max_duration_minutes: default_max_duration_minutes(),
        }
    }
}

/// The stall timeout of a profile that does not set one (ADR-0039).
pub const DEFAULT_STALL_TIMEOUT_MINUTES: u32 = 60;
/// The maximum duration of a profile that does not set one (ADR-0039).
pub const DEFAULT_MAX_DURATION_MINUTES: u32 = 48 * 60;
const STALL_TIMEOUT_VARIABLE: &str = "RESTICHUB_JOB_STALL_TIMEOUT_SECS";
const MAX_DURATION_VARIABLE: &str = "RESTICHUB_JOB_MAX_DURATION_SECS";
const MIN_STALL_TIMEOUT_MINUTES: u32 = 5;
const MAX_STALL_TIMEOUT_MINUTES: u32 = 10_080;
const MAX_DURATION_MINUTES: u32 = 43_200;

/// The stall timeout of a new profile: the environment variable or the built-in default.
fn default_stall_timeout_minutes() -> u32 {
    minutes_from_variable(
        STALL_TIMEOUT_VARIABLE,
        DEFAULT_STALL_TIMEOUT_MINUTES,
        MIN_STALL_TIMEOUT_MINUTES..=MAX_STALL_TIMEOUT_MINUTES,
    )
}

/// The maximum duration of a new profile: the environment variable or the built-in default.
fn default_max_duration_minutes() -> u32 {
    minutes_from_variable(
        MAX_DURATION_VARIABLE,
        DEFAULT_MAX_DURATION_MINUTES,
        0..=MAX_DURATION_MINUTES,
    )
}

/// Reads seconds from `name`. A missing or invalid value, or one outside `range`, gives `fallback`.
fn minutes_from_variable(name: &str, fallback: u32, range: std::ops::RangeInclusive<u32>) -> u32 {
    minutes_from_seconds(std::env::var(name).ok().as_deref(), fallback, range)
}

fn minutes_from_seconds(
    value: Option<&str>,
    fallback: u32,
    range: std::ops::RangeInclusive<u32>,
) -> u32 {
    value
        .and_then(|value| value.trim().parse::<u32>().ok())
        .map(|seconds| seconds.div_ceil(60))
        .filter(|minutes| range.contains(minutes))
        .unwrap_or(fallback)
}

impl BackupOptions {
    fn stall_minutes(&self) -> u32 {
        self.stall_timeout_minutes
            .unwrap_or_else(default_stall_timeout_minutes)
    }

    fn max_minutes(&self) -> u32 {
        self.max_duration_minutes
            .unwrap_or_else(default_max_duration_minutes)
    }
}

/// Position of a script in the job (FR-HK-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HookPhase {
    Before,
    After,
    OnSuccess,
    OnError,
    Finally,
}

/// Grouping key of the retention rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    Host,
    Paths,
    Tags,
}

/// Snapshot filter of the retention rule.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotFilter {
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
}

/// How long the snapshots stay (FR-PF-01).
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Retention {
    #[serde(default)]
    pub keep_last: u32,
    #[serde(default)]
    pub keep_hourly: u32,
    #[serde(default)]
    pub keep_daily: u32,
    #[serde(default)]
    pub keep_weekly: u32,
    #[serde(default)]
    pub keep_monthly: u32,
    #[serde(default)]
    pub keep_yearly: u32,
    /// Keep the snapshots younger than a duration, for example `30d`.
    #[serde(default)]
    pub keep_within: String,
    #[serde(default)]
    pub keep_tags: Vec<String>,
    #[serde(default)]
    pub group_by: Vec<GroupBy>,
    #[serde(default)]
    pub filter: SnapshotFilter,
}

/// Settings of the verification step (FR-PF-01).
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckOptions {
    /// Portion to check, for example `10%` or `1/7`.
    #[serde(default)]
    pub read_data_subset: String,
}

/// The fields that arrive at the create endpoint (FR-PF-01).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProfileRequest {
    /// Free text of 1 to 100 characters. Unique in the tenant, ignoring the letter case. Holds no control or format character.
    #[schema(max_length = 100)]
    pub name: String,
    #[serde(default)]
    pub paths: ProfilePaths,
    #[serde(default)]
    pub excludes: Vec<String>,
    #[serde(default)]
    pub exclude_files: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub options: BackupOptions,
    /// Scripts from the control plane, in the order they run in their phase (FR-HK-09). Only an
    /// Admin can set them.
    #[serde(default)]
    pub scripts: Vec<ScriptDef>,
    /// Absent when the profile keeps every snapshot.
    #[serde(default)]
    pub retention: Option<Retention>,
    /// Absent when the profile runs no verification step.
    #[serde(default)]
    pub check: Option<CheckOptions>,
    /// What the profile offers to its hosts (FR-PF-09). At most 64.
    #[serde(default)]
    #[schema(max_items = 64)]
    pub parameters: Vec<ParameterDefRequest>,
    /// Accepted only as an empty list, so an old client keeps working; use `scripts` instead
    /// (ADR-0040).
    #[serde(default)]
    #[schema(ignore)]
    pub hooks: Option<Vec<serde_json::Value>>,
}

/// The body of a profile update. The body replaces the settings; an absent name keeps the name.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProfileRequest {
    /// Free text of 1 to 100 characters. Unique in the tenant, ignoring the letter case. Holds no control or format character.
    #[schema(max_length = 100)]
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub paths: ProfilePaths,
    #[serde(default)]
    pub excludes: Vec<String>,
    #[serde(default)]
    pub exclude_files: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub options: BackupOptions,
    /// Replaces the whole list. Only an Admin can change it (FR-HK-12).
    #[serde(default)]
    pub scripts: Vec<ScriptDef>,
    #[serde(default)]
    pub retention: Option<Retention>,
    #[serde(default)]
    pub check: Option<CheckOptions>,
    /// Replaces the whole list (FR-PF-09).
    #[serde(default)]
    #[schema(max_items = 64)]
    pub parameters: Vec<ParameterDefRequest>,
    /// Accepted only as an empty list, so an old client keeps working; use `scripts` instead
    /// (ADR-0040).
    #[serde(default)]
    #[schema(ignore)]
    pub hooks: Option<Vec<serde_json::Value>>,
}

/// One stored profile (FR-PF-01). It holds no secret.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProfileView {
    pub id: String,
    pub name: String,
    pub paths: ProfilePaths,
    pub excludes: Vec<String>,
    pub exclude_files: Vec<String>,
    pub tags: Vec<String>,
    pub options: BackupOptions,
    pub scripts: Vec<ScriptDef>,
    pub retention: Option<Retention>,
    pub check: Option<CheckOptions>,
    /// What the profile declares (FR-PF-09). For a Viewer, a text default is empty with the flag
    /// `hidden` (FR-PF-12).
    pub parameters: Vec<ParameterDefView>,
    /// In RFC 3339.
    pub created_at: String,
    /// In RFC 3339.
    pub updated_at: String,
    /// Null for an API token actor.
    pub created_by: Option<String>,
}

impl ProfileView {
    /// A string value may hold a credential that was typed into the script, so it goes with the body
    /// (FR-HK-12). A text parameter default is a value too, so a Viewer reads none of them
    /// (FR-PF-12).
    pub fn hide_script_bodies(&mut self) {
        for script in &mut self.scripts {
            script.body.clear();
            for variable in &mut script.env {
                if variable.kind == EnvKind::String {
                    variable.value.clear();
                }
            }
        }
        for definition in &mut self.parameters {
            if definition.kind == restichub_db::ParameterKind::Text
                && definition.default_value.is_some()
            {
                definition.default_value = Some(String::new());
                definition.hidden = true;
            }
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProfileListResponse {
    pub items: Vec<ProfileView>,
    /// Null when the table holds no later profile.
    pub next_cursor: Option<String>,
}

/// A warning that the save-time compile reported for one target (FR-PF-05).
#[derive(Debug, Serialize, ToSchema)]
pub struct ProfileWarning {
    /// Where the warning applies, for example `paths.all`.
    pub field: String,
    /// Stable snake_case code.
    pub reason: String,
    /// The operating system of the synthetic target, when the warning is specific to it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
}

/// The result of a profile create or update: the stored profile and the save-time warnings
/// (FR-PF-05).
#[derive(Debug, Serialize, ToSchema)]
pub struct ProfileResponse {
    pub profile: ProfileView,
    pub warnings: Vec<ProfileWarning>,
    /// The plans of the profile that the updated settings no longer compile for (FR-SC-11).
    pub plan_warnings: Vec<PlanWarning>,
}

/// A plan that the updated profile no longer compiles for (FR-SC-11).
///
/// The update does not refuse the change. The plan keeps its schedule and fails at dispatch time,
/// when the queue recompiles the profile for the facts of the agent, in a later part of epic #7.
#[derive(Debug, Serialize, ToSchema)]
pub struct PlanWarning {
    pub plan_id: String,
    pub name: String,
    /// The compiler field that the profile fails on, for example `paths`.
    pub field: String,
    /// Stable snake_case code.
    pub reason: String,
}

impl TryFrom<&Profile> for ProfileView {
    type Error = ApiError;

    fn try_from(profile: &Profile) -> Result<Self, ApiError> {
        Ok(Self {
            id: profile.id.to_string(),
            name: profile.name.clone(),
            paths: paths_from_row(profile),
            excludes: profile.excludes.clone(),
            exclude_files: profile.exclude_files.clone(),
            tags: profile.tags.clone(),
            options: BackupOptions {
                use_fs_snapshot: profile.use_fs_snapshot,
                one_file_system: profile.one_file_system,
                exclude_caches: profile.exclude_caches,
                limit_upload_kib: u64::try_from(profile.limit_upload_kib).unwrap_or(0),
                limit_download_kib: u64::try_from(profile.limit_download_kib).unwrap_or(0),
                stall_timeout_minutes: Some(
                    u32::try_from(profile.stall_timeout_minutes)
                        .unwrap_or(DEFAULT_STALL_TIMEOUT_MINUTES),
                ),
                max_duration_minutes: Some(
                    u32::try_from(profile.max_duration_minutes).unwrap_or(0),
                ),
            },
            scripts: crate::scripts::from_row(&profile.scripts)?,
            retention: retention_from_row(profile)?,
            check: check_from_row(profile)?,
            parameters: parameters::from_row(&profile.parameters)?
                .iter()
                .map(|definition| ParameterDefView::of(definition, true))
                .collect(),
            created_at: format_time(profile.created_at),
            updated_at: format_time(profile.updated_at),
            created_by: profile.created_by.map(|id| id.to_string()),
        })
    }
}

fn paths_from_row(profile: &Profile) -> ProfilePaths {
    ProfilePaths {
        all: profile.paths_all.clone(),
        windows: profile.paths_windows.clone(),
        linux: profile.paths_linux.clone(),
        macos: profile.paths_macos.clone(),
    }
}

fn retention_from_row(profile: &Profile) -> Result<Option<Retention>, ApiError> {
    match &profile.retention {
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|error| {
                ApiError::internal(format!("the stored retention is not valid: {error}"))
            }),
        None => Ok(None),
    }
}

fn check_from_row(profile: &Profile) -> Result<Option<CheckOptions>, ApiError> {
    match &profile.check_options {
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|error| {
                ApiError::internal(format!("the stored check options are not valid: {error}"))
            }),
        None => Ok(None),
    }
}

/// The settings of a create or an update after validation.
#[derive(Debug, Clone)]
pub struct ValidatedProfile {
    pub name: String,
    pub paths: ProfilePaths,
    pub excludes: Vec<String>,
    pub exclude_files: Vec<String>,
    pub tags: Vec<String>,
    pub options: BackupOptions,
    pub scripts: Vec<ScriptDef>,
    pub retention: Option<Retention>,
    pub check: Option<CheckOptions>,
    pub parameters: Vec<ParameterDef>,
}

impl ValidatedProfile {
    pub fn new_profile<'a>(&'a self, id: ProfileId, created_by: Option<UserId>) -> NewProfile<'a> {
        NewProfile {
            id,
            name: &self.name,
            paths_all: &self.paths.all,
            paths_windows: &self.paths.windows,
            paths_linux: &self.paths.linux,
            paths_macos: &self.paths.macos,
            excludes: &self.excludes,
            exclude_files: &self.exclude_files,
            tags: &self.tags,
            use_fs_snapshot: self.options.use_fs_snapshot,
            one_file_system: self.options.one_file_system,
            exclude_caches: self.options.exclude_caches,
            limit_upload_kib: i64::try_from(self.options.limit_upload_kib).unwrap_or(i64::MAX),
            limit_download_kib: i64::try_from(self.options.limit_download_kib).unwrap_or(i64::MAX),
            stall_timeout_minutes: i32::try_from(self.options.stall_minutes()).unwrap_or(i32::MAX),
            max_duration_minutes: i32::try_from(self.options.max_minutes()).unwrap_or(i32::MAX),
            retention: self.retention_value(),
            check_options: self.check_value(),
            scripts: self.scripts_value(),
            parameters: self.parameters_value(),
            created_by,
        }
    }

    pub fn update<'a>(&'a self) -> ProfileUpdate<'a> {
        ProfileUpdate {
            name: &self.name,
            paths_all: &self.paths.all,
            paths_windows: &self.paths.windows,
            paths_linux: &self.paths.linux,
            paths_macos: &self.paths.macos,
            excludes: &self.excludes,
            exclude_files: &self.exclude_files,
            tags: &self.tags,
            use_fs_snapshot: self.options.use_fs_snapshot,
            one_file_system: self.options.one_file_system,
            exclude_caches: self.options.exclude_caches,
            limit_upload_kib: i64::try_from(self.options.limit_upload_kib).unwrap_or(i64::MAX),
            limit_download_kib: i64::try_from(self.options.limit_download_kib).unwrap_or(i64::MAX),
            stall_timeout_minutes: self
                .options
                .stall_timeout_minutes
                .map(|minutes| i32::try_from(minutes).unwrap_or(i32::MAX)),
            max_duration_minutes: self
                .options
                .max_duration_minutes
                .map(|minutes| i32::try_from(minutes).unwrap_or(i32::MAX)),
            retention: self.retention_value(),
            check_options: self.check_value(),
            scripts: self.scripts_value(),
            parameters: self.parameters_value(),
        }
    }

    fn retention_value(&self) -> Option<Value> {
        self.retention
            .as_ref()
            .and_then(|retention| serde_json::to_value(retention).ok())
    }

    fn check_value(&self) -> Option<Value> {
        self.check
            .as_ref()
            .and_then(|check| serde_json::to_value(check).ok())
    }

    fn scripts_value(&self) -> Value {
        serde_json::to_value(&self.scripts).unwrap_or_else(|_| json!([]))
    }

    fn parameters_value(&self) -> Value {
        parameters::to_value(&self.parameters)
    }

    /// The profile as the compiler input (FR-PF-04).
    pub fn to_compiler_profile(&self) -> CompilerProfile {
        let mut by_os = BTreeMap::new();
        if !self.paths.windows.is_empty() {
            by_os.insert(OperatingSystem::Windows, self.paths.windows.clone());
        }
        if !self.paths.linux.is_empty() {
            by_os.insert(OperatingSystem::Linux, self.paths.linux.clone());
        }
        if !self.paths.macos.is_empty() {
            by_os.insert(OperatingSystem::Macos, self.paths.macos.clone());
        }
        CompilerProfile {
            paths: CompilerProfilePaths {
                all: self.paths.all.clone(),
                by_os,
            },
            excludes: self.excludes.clone(),
            exclude_files: self.exclude_files.clone(),
            tags: self.tags.clone(),
            // An unsaved profile has no identifier, so the preview shows a tag that no stored
            // profile can hold. The reserved prefix keeps it out of the tags of a user.
            profile_tag: Some(UNSAVED_PROFILE_TAG.to_owned()),
            options: CompilerBackupOptions {
                use_fs_snapshot: self.options.use_fs_snapshot,
                one_file_system: self.options.one_file_system,
                exclude_caches: self.options.exclude_caches,
                limit_upload_kib: self.options.limit_upload_kib,
                limit_download_kib: self.options.limit_download_kib,
            },
            limits: CompilerJobLimits {
                stall_timeout_seconds: self.options.stall_minutes().saturating_mul(60),
                max_duration_seconds: self.options.max_minutes().saturating_mul(60),
            },
            scripts: self
                .scripts
                .iter()
                .map(crate::scripts::compiler_script)
                .collect(),
            retention: self.retention.as_ref().map(compiler_retention),
            check: self.check.as_ref().map(|check| CompilerCheckOptions {
                read_data_subset: check.read_data_subset.clone(),
            }),
        }
    }
}

/// The tag that marks the snapshots of one profile (FR-PF-04, FR-RP-08).
///
/// The job builder adds it to every backup of the profile. The `forget` step of the maintenance job
/// filters on it, so the retention of one profile never removes the snapshots of another profile
/// that shares the repository, the host or the paths.
pub fn profile_tag(id: restichub_db::ProfileId) -> String {
    format!("{RESERVED_TAG_PREFIX}profile-{id}")
}

/// The prefix of the tags that only the control plane sets. A user tag may not start with it.
pub const RESERVED_TAG_PREFIX: &str = "restichub-";

/// The tag that the preview of an unsaved profile shows (FR-PF-04, FR-RP-08).
///
/// The reserved prefix keeps the value out of the tags of a user, and no stored profile can hold
/// it, because [`profile_tag`] always ends with an identifier.
pub const UNSAVED_PROFILE_TAG: &str = "restichub-profile-new";

/// The stored profile as the compiler input (FR-PF-04).
pub fn compiler_profile_from_row(profile: &Profile) -> Result<CompilerProfile, ApiError> {
    let validated = ValidatedProfile {
        name: profile.name.clone(),
        paths: paths_from_row(profile),
        excludes: profile.excludes.clone(),
        exclude_files: profile.exclude_files.clone(),
        tags: profile.tags.clone(),
        options: BackupOptions {
            use_fs_snapshot: profile.use_fs_snapshot,
            one_file_system: profile.one_file_system,
            exclude_caches: profile.exclude_caches,
            limit_upload_kib: u64::try_from(profile.limit_upload_kib).unwrap_or(0),
            limit_download_kib: u64::try_from(profile.limit_download_kib).unwrap_or(0),
            stall_timeout_minutes: Some(
                u32::try_from(profile.stall_timeout_minutes)
                    .unwrap_or(DEFAULT_STALL_TIMEOUT_MINUTES),
            ),
            max_duration_minutes: Some(u32::try_from(profile.max_duration_minutes).unwrap_or(0)),
        },
        scripts: crate::scripts::from_row(&profile.scripts)?,
        retention: retention_from_row(profile)?,
        check: check_from_row(profile)?,
        parameters: parameters::from_row(&profile.parameters)?,
    };
    let mut compiler_profile = validated.to_compiler_profile();
    compiler_profile.profile_tag = Some(profile_tag(profile.id));
    Ok(compiler_profile)
}

fn compiler_retention(retention: &Retention) -> CompilerRetention {
    CompilerRetention {
        keep_last: retention.keep_last,
        keep_hourly: retention.keep_hourly,
        keep_daily: retention.keep_daily,
        keep_weekly: retention.keep_weekly,
        keep_monthly: retention.keep_monthly,
        keep_yearly: retention.keep_yearly,
        keep_within: retention.keep_within.clone(),
        keep_tags: retention.keep_tags.clone(),
        group_by: retention
            .group_by
            .iter()
            .map(|group| match group {
                GroupBy::Host => CompilerGroupBy::Host,
                GroupBy::Paths => CompilerGroupBy::Paths,
                GroupBy::Tags => CompilerGroupBy::Tags,
            })
            .collect(),
        filter: CompilerSnapshotFilter {
            hosts: retention.filter.hosts.clone(),
            tags: retention.filter.tags.clone(),
            paths: retention.filter.paths.clone(),
        },
    }
}

/// The identifier of a profile from a path parameter.
pub fn parse_profile_id(value: &str) -> Result<ProfileId, ApiError> {
    Uuid::parse_str(value)
        .map(ProfileId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause("id", Reason::InvalidFormat, error.to_string())
        })
}

fn format_time(value: time::OffsetDateTime) -> String {
    use time::format_description::well_known::Rfc3339;
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod limit_variable_tests {
    use super::minutes_from_seconds;

    #[test]
    fn fr_pf_01_variable_gives_minutes_in_range() {
        assert_eq!(minutes_from_seconds(None, 60, 5..=100), 60);
        assert_eq!(minutes_from_seconds(Some("1800"), 60, 5..=100), 30);
        assert_eq!(minutes_from_seconds(Some("61"), 60, 5..=100), 60);
        assert_eq!(minutes_from_seconds(Some("junk"), 60, 5..=100), 60);
        assert_eq!(minutes_from_seconds(Some("0"), 2880, 0..=43_200), 0);
    }
}
