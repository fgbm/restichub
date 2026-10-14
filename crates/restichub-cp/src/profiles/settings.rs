//! The typed profile settings, the validation and the compiler mapping.
//!
//! The module holds the API types of the profile endpoints, the validation that runs before a
//! write (FR-PF-05) and the mapping to the compiler types (FR-PF-04). The handlers live in the
//! parent module.
//!
//! The profile holds no secret. The repository password and the backend credentials stay in the
//! `secrets` table (ADR-0017), so a profile value is safe in an audit diff.

use std::collections::BTreeMap;

use restichub_compiler::{
    BackupOptions as CompilerBackupOptions, CheckOptions as CompilerCheckOptions,
    GroupBy as CompilerGroupBy, HookParameter as CompilerHookParameter,
    HookPhase as CompilerHookPhase, HookRef as CompilerHookRef, OperatingSystem,
    Profile as CompilerProfile, ProfilePaths as CompilerProfilePaths,
    Retention as CompilerRetention, SnapshotFilter as CompilerSnapshotFilter,
};
use restichub_db::profiles::{NewProfile, Profile, ProfileUpdate};
use restichub_db::{ProfileId, RepositoryId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::validation::{self, Reason};

/// Limit in characters of a profile name. The derived value stays within the column.
pub const MAX_PROFILE_NAME: usize = 48;
/// Limit in characters of a hook identifier or a hook parameter name.
pub const MAX_HOOK_ID: usize = 64;
/// Limit in characters of one exclude, exclude file or tag.
pub const MAX_PATTERN: usize = 4096;
/// Limit in characters of one path entry. A Windows path is longer than a Unix path.
pub const MAX_PATH_ENTRY: usize = 4096;

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
}

/// Position of a hook in the job (FR-HK-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HookPhase {
    Before,
    After,
    OnSuccess,
    OnError,
    Finally,
}

/// One named hook parameter (FR-HK-03).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HookParameter {
    pub name: String,
    pub value: String,
}

/// A hook of the profile, by allow-list identifier (FR-HK-02).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HookRef {
    pub hook_id: String,
    pub phase: HookPhase,
    #[serde(default)]
    pub parameters: Vec<HookParameter>,
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
    /// Holds 1 to 48 characters from `a-z`, `0-9`, `.`, `_` and `-`.
    pub name: String,
    /// The identifier of the repository that the profile uses.
    pub repository_id: String,
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
    #[serde(default)]
    pub hooks: Vec<HookRef>,
    /// Absent when the profile keeps every snapshot.
    #[serde(default)]
    pub retention: Option<Retention>,
    /// Absent when the profile runs no verification step.
    #[serde(default)]
    pub check: Option<CheckOptions>,
}

/// The body of a profile update. The body replaces the settings; an absent name keeps the name.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProfileRequest {
    #[serde(default)]
    pub name: Option<String>,
    pub repository_id: String,
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
    #[serde(default)]
    pub hooks: Vec<HookRef>,
    #[serde(default)]
    pub retention: Option<Retention>,
    #[serde(default)]
    pub check: Option<CheckOptions>,
}

/// One stored profile (FR-PF-01). It holds no secret.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProfileView {
    pub id: String,
    pub name: String,
    pub repository_id: String,
    pub paths: ProfilePaths,
    pub excludes: Vec<String>,
    pub exclude_files: Vec<String>,
    pub tags: Vec<String>,
    pub options: BackupOptions,
    pub hooks: Vec<HookRef>,
    pub retention: Option<Retention>,
    pub check: Option<CheckOptions>,
    /// In RFC 3339.
    pub created_at: String,
    /// In RFC 3339.
    pub updated_at: String,
    /// Null for an API token actor.
    pub created_by: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProfileListResponse {
    pub items: Vec<ProfileView>,
}

/// The agent identifier of the request (FR-PF-04).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    /// The identifier of the registered agent that supplies the target facts.
    pub agent_id: String,
}

/// One environment variable of a previewed step. A secret value is masked (AC-PF-01).
#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewEnvView {
    pub name: String,
    pub value: String,
}

/// One previewed step: the kind, the maintenance flag, the command line and the environment
/// (FR-PF-04, AC-PF-01).
#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewStepView {
    /// The step kind, for example `backup`, `forget`, `prune`, `check` or `hook`.
    pub kind: String,
    /// True for `forget`, `prune` and `check` (FR-RP-08).
    pub maintenance: bool,
    /// Rendered argv with masked secrets.
    pub command: String,
    /// Process variables with masked secrets.
    pub environment: Vec<PreviewEnvView>,
}

/// The preview of a profile for one target agent (FR-PF-04).
#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewResponse {
    pub profile_id: String,
    pub repository_id: String,
    pub steps: Vec<PreviewStepView>,
}

impl TryFrom<&Profile> for ProfileView {
    type Error = ApiError;

    fn try_from(profile: &Profile) -> Result<Self, ApiError> {
        Ok(Self {
            id: profile.id.to_string(),
            name: profile.name.clone(),
            repository_id: profile.repository_id.to_string(),
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
            },
            hooks: hooks_from_row(profile)?,
            retention: retention_from_row(profile)?,
            check: check_from_row(profile)?,
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

fn hooks_from_row(profile: &Profile) -> Result<Vec<HookRef>, ApiError> {
    serde_json::from_value(profile.hooks.clone())
        .map_err(|error| ApiError::internal(format!("the stored hooks are not valid: {error}")))
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
    pub repository_id: RepositoryId,
    pub paths: ProfilePaths,
    pub excludes: Vec<String>,
    pub exclude_files: Vec<String>,
    pub tags: Vec<String>,
    pub options: BackupOptions,
    pub hooks: Vec<HookRef>,
    pub retention: Option<Retention>,
    pub check: Option<CheckOptions>,
}

impl ValidatedProfile {
    pub fn new_profile<'a>(&'a self, id: ProfileId, created_by: Option<UserId>) -> NewProfile<'a> {
        NewProfile {
            id,
            name: &self.name,
            repository_id: self.repository_id,
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
            retention: self.retention_value(),
            check_options: self.check_value(),
            hooks: self.hooks_value(),
            created_by,
        }
    }

    pub fn update<'a>(&'a self) -> ProfileUpdate<'a> {
        ProfileUpdate {
            name: &self.name,
            repository_id: self.repository_id,
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
            retention: self.retention_value(),
            check_options: self.check_value(),
            hooks: self.hooks_value(),
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

    fn hooks_value(&self) -> Value {
        serde_json::to_value(&self.hooks).unwrap_or_else(|_| json!([]))
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
            repository_id: self.repository_id.to_string(),
            paths: CompilerProfilePaths {
                all: self.paths.all.clone(),
                by_os,
            },
            excludes: self.excludes.clone(),
            exclude_files: self.exclude_files.clone(),
            tags: self.tags.clone(),
            options: CompilerBackupOptions {
                use_fs_snapshot: self.options.use_fs_snapshot,
                one_file_system: self.options.one_file_system,
                exclude_caches: self.options.exclude_caches,
                limit_upload_kib: self.options.limit_upload_kib,
                limit_download_kib: self.options.limit_download_kib,
            },
            hooks: self.hooks.iter().map(compiler_hook).collect(),
            retention: self.retention.as_ref().map(compiler_retention),
            check: self.check.as_ref().map(|check| CompilerCheckOptions {
                read_data_subset: check.read_data_subset.clone(),
            }),
        }
    }
}

/// The stored profile as the compiler input (FR-PF-04).
pub fn compiler_profile_from_row(profile: &Profile) -> Result<CompilerProfile, ApiError> {
    let validated = ValidatedProfile {
        name: profile.name.clone(),
        repository_id: profile.repository_id,
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
        },
        hooks: hooks_from_row(profile)?,
        retention: retention_from_row(profile)?,
        check: check_from_row(profile)?,
    };
    Ok(validated.to_compiler_profile())
}

fn compiler_hook(hook: &HookRef) -> CompilerHookRef {
    CompilerHookRef {
        hook_id: hook.hook_id.clone(),
        phase: match hook.phase {
            HookPhase::Before => CompilerHookPhase::Before,
            HookPhase::After => CompilerHookPhase::After,
            HookPhase::OnSuccess => CompilerHookPhase::OnSuccess,
            HookPhase::OnError => CompilerHookPhase::OnError,
            HookPhase::Finally => CompilerHookPhase::Finally,
        },
        parameters: hook
            .parameters
            .iter()
            .map(|parameter| CompilerHookParameter {
                name: parameter.name.clone(),
                value: parameter.value.clone(),
            })
            .collect(),
    }
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

/// Validates a create request and returns the normalized settings (FR-PF-05).
pub fn validate_create(body: CreateProfileRequest) -> Result<ValidatedProfile, ApiError> {
    validate_profile(
        Some(body.name),
        body.repository_id,
        body.paths,
        body.excludes,
        body.exclude_files,
        body.tags,
        body.options,
        body.hooks,
        body.retention,
        body.check,
    )
}

/// Validates an update request. `current_name` is used when the body holds no name.
pub fn validate_update(
    body: UpdateProfileRequest,
    current_name: &str,
) -> Result<ValidatedProfile, ApiError> {
    let name = body.name.unwrap_or_else(|| current_name.to_owned());
    validate_profile(
        Some(name),
        body.repository_id,
        body.paths,
        body.excludes,
        body.exclude_files,
        body.tags,
        body.options,
        body.hooks,
        body.retention,
        body.check,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_profile(
    name: Option<String>,
    repository_id: String,
    paths: ProfilePaths,
    excludes: Vec<String>,
    exclude_files: Vec<String>,
    tags: Vec<String>,
    options: BackupOptions,
    hooks: Vec<HookRef>,
    retention: Option<Retention>,
    check: Option<CheckOptions>,
) -> Result<ValidatedProfile, ApiError> {
    let name = validate_profile_name("name", &name.unwrap_or_default())?;
    let repository_id = parse_repository_id("repository_id", &repository_id)?;
    let paths = validate_paths(paths)?;
    let excludes = validate_patterns("excludes", excludes)?;
    let exclude_files = validate_patterns("exclude_files", exclude_files)?;
    let tags = validate_tags("tags", tags)?;
    let hooks = validate_hooks(hooks)?;
    let retention = match retention {
        Some(retention) => Some(validate_retention(retention)?),
        None => None,
    };
    let check = match check {
        Some(check) => Some(validate_check(check)?),
        None => None,
    };
    Ok(ValidatedProfile {
        name,
        repository_id,
        paths,
        excludes,
        exclude_files,
        tags,
        options,
        hooks,
        retention,
        check,
    })
}

/// A profile name holds 1 to [`MAX_PROFILE_NAME`] characters from `a-z`, `0-9`, `.`, `_` and `-`
/// (FR-PF-01). The unique index of the `profiles` table makes it unique for the tenant.
pub fn validate_profile_name(field: &'static str, value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 {
        return Err(ApiError::invalid_field(field, Reason::Empty));
    }
    if count > MAX_PROFILE_NAME {
        return Err(ApiError::invalid_field(field, Reason::TooLong));
    }
    if restichub_proto::validate::has_forbidden(value)
        || !value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(ApiError::invalid_field(field, Reason::InvalidCharacter));
    }
    Ok(value.to_owned())
}

fn validate_paths(paths: ProfilePaths) -> Result<ProfilePaths, ApiError> {
    Ok(ProfilePaths {
        all: validate_path_entries("paths.all", paths.all)?,
        windows: validate_path_entries("paths.windows", paths.windows)?,
        linux: validate_path_entries("paths.linux", paths.linux)?,
        macos: validate_path_entries("paths.macos", paths.macos)?,
    })
}

/// A path entry is absolute: it starts with `/` or a drive letter (FR-PF-01). A template variable
/// is allowed inside the path (FR-PF-03).
fn validate_path_entries(
    field: &'static str,
    values: Vec<String>,
) -> Result<Vec<String>, ApiError> {
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let validated = validation::validate_local_path(field, &value)?;
        if validated.chars().count() > MAX_PATH_ENTRY {
            return Err(ApiError::invalid_field(field, Reason::TooLong));
        }
        result.push(validated);
    }
    Ok(result)
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
        result.push(trimmed.to_owned());
    }
    Ok(result)
}

fn validate_hooks(hooks: Vec<HookRef>) -> Result<Vec<HookRef>, ApiError> {
    let mut result = Vec::with_capacity(hooks.len());
    for hook in hooks {
        let hook_id = validate_identifier("hooks.hook_id", &hook.hook_id)?;
        let mut seen = std::collections::BTreeSet::new();
        let mut parameters = Vec::with_capacity(hook.parameters.len());
        for parameter in hook.parameters {
            let name = validate_identifier("hooks.parameters.name", &parameter.name)?;
            if !seen.insert(name.clone()) {
                return Err(ApiError::invalid_field(
                    "hooks.parameters.name",
                    Reason::Unknown,
                ));
            }
            if parameter.value.chars().count() > MAX_PATTERN
                || restichub_proto::validate::has_forbidden(&parameter.value)
            {
                return Err(ApiError::invalid_field(
                    "hooks.parameters.value",
                    Reason::InvalidCharacter,
                ));
            }
            parameters.push(HookParameter {
                name,
                value: parameter.value,
            });
        }
        result.push(HookRef {
            hook_id,
            phase: hook.phase,
            parameters,
        });
    }
    Ok(result)
}

/// A hook identifier or a parameter name holds 1 to [`MAX_HOOK_ID`] characters from `a-z`, `0-9`,
/// `.`, `_` and `-` (FR-HK-02).
fn validate_identifier(field: &'static str, value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 {
        return Err(ApiError::invalid_field(field, Reason::Empty));
    }
    if count > MAX_HOOK_ID {
        return Err(ApiError::invalid_field(field, Reason::TooLong));
    }
    if !value.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '.' | '_' | '-')
    }) {
        return Err(ApiError::invalid_field(field, Reason::InvalidCharacter));
    }
    Ok(value.to_owned())
}

fn validate_retention(retention: Retention) -> Result<Retention, ApiError> {
    if !retention.keep_within.is_empty() && !is_duration(&retention.keep_within) {
        return Err(ApiError::invalid_field(
            "retention.keep_within",
            Reason::InvalidFormat,
        ));
    }
    let keep_tags = validate_tags("retention.keep_tags", retention.keep_tags)?;
    let filter = SnapshotFilter {
        hosts: validate_patterns("retention.filter.hosts", retention.filter.hosts)?,
        tags: validate_tags("retention.filter.tags", retention.filter.tags)?,
        paths: validate_patterns("retention.filter.paths", retention.filter.paths)?,
    };
    Ok(Retention {
        keep_tags,
        filter,
        ..retention
    })
}

fn validate_check(check: CheckOptions) -> Result<CheckOptions, ApiError> {
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

/// The identifier of a profile from a path parameter.
pub fn parse_profile_id(value: &str) -> Result<ProfileId, ApiError> {
    Uuid::parse_str(value)
        .map(ProfileId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause("id", Reason::InvalidFormat, error.to_string())
        })
}

/// The identifier of a repository from a request field.
pub fn parse_repository_id(field: &'static str, value: &str) -> Result<RepositoryId, ApiError> {
    Uuid::parse_str(value)
        .map(RepositoryId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause(field, Reason::InvalidFormat, error.to_string())
        })
}

/// The settings of a validated profile for an audit diff. It holds no secret (FR-PF-01).
pub fn profile_value(profile: &ValidatedProfile) -> Value {
    json!({
        "name": profile.name,
        "repository_id": profile.repository_id.to_string(),
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
        },
        "hooks": profile.hooks,
        "retention": profile.retention,
        "check": profile.check,
    })
}

/// The settings of a stored row for an audit diff.
pub fn stored_value(profile: &Profile) -> Value {
    json!({
        "name": profile.name,
        "repository_id": profile.repository_id.to_string(),
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
        },
        "hooks": profile.hooks,
        "retention": profile.retention,
        "check": profile.check_options,
    })
}

fn format_time(value: time::OffsetDateTime) -> String {
    use time::format_description::well_known::Rfc3339;
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}
