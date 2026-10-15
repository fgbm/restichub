//! The typed profile settings, the validation and the compiler mapping.
//!
//! The module holds the API types of the profile endpoints, the validation that runs before a
//! write (FR-PF-05) and the mapping to the compiler types (FR-PF-04). The handlers live in the
//! parent module.
//!
//! The profile holds no secret. The repository password and the backend credentials stay in the
//! `secrets` table (ADR-0017). A hook parameter of the type `secret_ref` holds the identifier of a
//! secret, never the value (FR-HK-03); the job builder resolves it at dispatch time. The shape of
//! the other parameters follows the schema that the target agents report (FR-HK-04), so the check
//! against the schema runs in [`super::hooks`].

use std::collections::BTreeMap;

use restichub_compiler::{
    AllowListedHook, Architecture, BackupOptions as CompilerBackupOptions,
    CheckOptions as CompilerCheckOptions, CompileError, GroupBy as CompilerGroupBy,
    HookParameter as CompilerHookParameter, HookPhase as CompilerHookPhase,
    HookRef as CompilerHookRef, JobLimits as CompilerJobLimits, OperatingSystem,
    Profile as CompilerProfile, ProfilePaths as CompilerProfilePaths, ResticVersion,
    Retention as CompilerRetention, SnapshotFilter as CompilerSnapshotFilter, TargetFacts, compile,
};
use restichub_db::profiles::{NewProfile, Profile, ProfileUpdate};
use restichub_db::{ProfileId, RepositoryId, UserId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::scripts::ScriptDef;
use crate::validation::{self, Reason};

/// Limit in characters of a profile name. The derived value stays within the column.
pub const MAX_PROFILE_NAME: usize = 48;
/// Limit in characters of a hook identifier or a hook parameter name.
pub const MAX_HOOK_ID: usize = 64;
/// Limit in bytes of the text of one hook parameter value.
pub const MAX_HOOK_VALUE: usize = 4096;
/// Limit of the parameters of one hook.
pub const MAX_HOOK_PARAMETERS: usize = restichub_compiler::hook::MAX_PARAMETERS;
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
/// Limit of the hooks.
pub const MAX_HOOKS: usize = 32;
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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
    /// Minutes without progress after which the agent stops a step. From 5 to 10080 (ADR-0039).
    #[serde(default = "default_stall_timeout_minutes")]
    pub stall_timeout_minutes: u32,
    /// Minutes after which the agent stops the job whatever its progress. Zero means no limit.
    /// At most 43200 (ADR-0039).
    #[serde(default)]
    pub max_duration_minutes: u32,
}

/// The stall timeout of a profile that does not set one (ADR-0039).
pub const DEFAULT_STALL_TIMEOUT_MINUTES: u32 = 60;
const MIN_STALL_TIMEOUT_MINUTES: u32 = 5;
const MAX_STALL_TIMEOUT_MINUTES: u32 = 10_080;
const MAX_DURATION_MINUTES: u32 = 43_200;

fn default_stall_timeout_minutes() -> u32 {
    DEFAULT_STALL_TIMEOUT_MINUTES
}

impl Default for BackupOptions {
    fn default() -> Self {
        Self {
            use_fs_snapshot: false,
            one_file_system: false,
            exclude_caches: false,
            limit_upload_kib: 0,
            limit_download_kib: 0,
            stall_timeout_minutes: DEFAULT_STALL_TIMEOUT_MINUTES,
            max_duration_minutes: 0,
        }
    }
}

fn validate_limits(options: &BackupOptions) -> Result<(), ApiError> {
    if options.stall_timeout_minutes < MIN_STALL_TIMEOUT_MINUTES {
        return Err(ApiError::invalid_field(
            "options.stall_timeout_minutes",
            Reason::TooShort,
        ));
    }
    if options.stall_timeout_minutes > MAX_STALL_TIMEOUT_MINUTES {
        return Err(ApiError::invalid_field(
            "options.stall_timeout_minutes",
            Reason::TooLong,
        ));
    }
    if options.max_duration_minutes > MAX_DURATION_MINUTES {
        return Err(ApiError::invalid_field(
            "options.max_duration_minutes",
            Reason::TooLong,
        ));
    }
    Ok(())
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
    /// Replaces the whole list. Only an Admin can change it (FR-HK-12).
    #[serde(default)]
    pub scripts: Vec<ScriptDef>,
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
    pub scripts: Vec<ScriptDef>,
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

/// The outcome of one synthetic compile at save time (FR-PF-05, AC-PF-02).
enum SaveOutcome {
    /// The profile is wrong for every target: the request is refused.
    Refusal(ApiError),
    /// The profile is wrong for one target only: the caller gets a warning.
    Warning(ProfileWarning),
}

/// Runs the compiler against a synthetic target for each operating system (FR-PF-05).
///
/// A template error is wrong for every target, so it refuses the request. An operating-system or
/// version mismatch is a warning, because a profile may target several operating systems. The
/// synthetic target has no hooks: the hooks are checked against the report of the real agents
/// (FR-HK-04).
///
/// # Errors
///
/// Returns a field error for a defect that no target accepts.
pub fn save_warnings(profile: &ValidatedProfile) -> Result<Vec<ProfileWarning>, ApiError> {
    // The hooks depend on the allow-lists of the target agents, so the check against them runs
    // apart from this one (AC-HK-05).
    let mut compiler_profile = profile.to_compiler_profile();
    compiler_profile.hooks.clear();
    // Scripts depend on the trust mode of the target agents, which the synthetic target lacks.
    compiler_profile.scripts.clear();
    let allow_list: BTreeMap<String, AllowListedHook> = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for os in [
        OperatingSystem::Linux,
        OperatingSystem::Windows,
        OperatingSystem::Macos,
    ] {
        let target = synthetic_target(os, &allow_list);
        if let Err(error) = compile(&compiler_profile, &target) {
            match classify_save_error(error) {
                SaveOutcome::Refusal(api) => return Err(api),
                SaveOutcome::Warning(warning) => {
                    let key = (
                        warning.field.clone(),
                        warning.reason.clone(),
                        warning.os.clone(),
                    );
                    if seen.insert(key) {
                        warnings.push(warning);
                    }
                }
            }
        }
    }
    Ok(warnings)
}

fn synthetic_target(
    os: OperatingSystem,
    allow_list: &BTreeMap<String, AllowListedHook>,
) -> TargetFacts {
    TargetFacts {
        agent_id: "synthetic".to_owned(),
        agent_name: "agent".to_owned(),
        hostname: "host".to_owned(),
        os,
        arch: Architecture::X86_64,
        labels: BTreeMap::new(),
        // The newest supported version: the option and version check runs in the preview against
        // the real agent (FR-PF-05, AC-AG-09).
        restic_version: ResticVersion {
            major: 0,
            minor: 19,
            patch: 0,
        },
        now: time::OffsetDateTime::now_utc(),
        allow_list: allow_list.clone(),
        server_scripts: restichub_compiler::ServerScripts::default(),
    }
}

fn classify_save_error(error: CompileError) -> SaveOutcome {
    let warning = |field: &str, reason: &str, os: Option<String>| {
        SaveOutcome::Warning(ProfileWarning {
            field: field.to_owned(),
            reason: reason.to_owned(),
            os,
        })
    };
    match error {
        CompileError::UnknownVariable { field, .. }
        | CompileError::InvalidDate { field, .. }
        | CompileError::UnterminatedTemplate { field }
        | CompileError::InvalidValue { field } => SaveOutcome::Refusal(ApiError::invalid_field(
            field.as_str(),
            Reason::InvalidFormat,
        )),
        CompileError::UnknownHook { field, .. }
        | CompileError::UnknownHookParameter { field, .. }
        | CompileError::DuplicateHookParameter { field, .. } => {
            SaveOutcome::Refusal(ApiError::invalid_field(field.as_str(), Reason::Unknown))
        }
        CompileError::InvalidHookParameter { field, .. } => SaveOutcome::Refusal(
            ApiError::invalid_field(field.as_str(), Reason::InvalidFormat),
        ),
        CompileError::MissingHookParameter { field, .. } => {
            SaveOutcome::Refusal(ApiError::invalid_field(field.as_str(), Reason::Empty))
        }
        CompileError::HookPhaseNotAllowed { field, .. } => {
            SaveOutcome::Refusal(ApiError::invalid_field(field.as_str(), Reason::Unsupported))
        }
        CompileError::ServerScriptsDisabled { field }
        | CompileError::InterpreterUnavailable { field, .. } => {
            SaveOutcome::Refusal(ApiError::invalid_field(field.as_str(), Reason::Unavailable))
        }
        CompileError::StepsRejected { .. } => {
            SaveOutcome::Refusal(ApiError::invalid_field("steps", Reason::InvalidFormat))
        }
        CompileError::MissingLabel { field, .. } => warning(field.as_str(), "unknown", None),
        CompileError::UnsupportedOs { field, os, .. } => {
            warning(field.as_str(), "unsupported", Some(os.as_str().to_owned()))
        }
        CompileError::NoPaths { os } => warning("paths", "empty", Some(os.as_str().to_owned())),
        CompileError::IncompatibleOption { field, .. } => {
            warning(field.as_str(), "incompatible", None)
        }
        CompileError::UnsupportedRestic { .. } => warning("agent_id", "incompatible", None),
        CompileError::InvalidCron { .. } => warning("cron", "invalid_format", None),
    }
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
                stall_timeout_minutes: u32::try_from(profile.stall_timeout_minutes)
                    .unwrap_or(DEFAULT_STALL_TIMEOUT_MINUTES),
                max_duration_minutes: u32::try_from(profile.max_duration_minutes).unwrap_or(0),
            },
            hooks: hooks_from_row(profile)?,
            scripts: crate::scripts::from_row(&profile.scripts)?,
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
    pub scripts: Vec<ScriptDef>,
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
            stall_timeout_minutes: i32::try_from(self.options.stall_timeout_minutes)
                .unwrap_or(i32::MAX),
            max_duration_minutes: i32::try_from(self.options.max_duration_minutes)
                .unwrap_or(i32::MAX),
            retention: self.retention_value(),
            check_options: self.check_value(),
            hooks: self.hooks_value(),
            scripts: self.scripts_value(),
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
            stall_timeout_minutes: i32::try_from(self.options.stall_timeout_minutes)
                .unwrap_or(i32::MAX),
            max_duration_minutes: i32::try_from(self.options.max_duration_minutes)
                .unwrap_or(i32::MAX),
            retention: self.retention_value(),
            check_options: self.check_value(),
            hooks: self.hooks_value(),
            scripts: self.scripts_value(),
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

    fn scripts_value(&self) -> Value {
        serde_json::to_value(&self.scripts).unwrap_or_else(|_| json!([]))
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
            // The profile has no identifier before the first save, so this tag only satisfies the
            // compile check. The stored profile gets its own tag in `compiler_profile_from_row`.
            profile_tag: Some(format!("{RESERVED_TAG_PREFIX}profile-validation")),
            options: CompilerBackupOptions {
                use_fs_snapshot: self.options.use_fs_snapshot,
                one_file_system: self.options.one_file_system,
                exclude_caches: self.options.exclude_caches,
                limit_upload_kib: self.options.limit_upload_kib,
                limit_download_kib: self.options.limit_download_kib,
            },
            limits: CompilerJobLimits {
                stall_timeout_seconds: self.options.stall_timeout_minutes.saturating_mul(60),
                max_duration_seconds: self.options.max_duration_minutes.saturating_mul(60),
            },
            hooks: self.hooks.iter().map(compiler_hook).collect(),
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
            stall_timeout_minutes: u32::try_from(profile.stall_timeout_minutes)
                .unwrap_or(DEFAULT_STALL_TIMEOUT_MINUTES),
            max_duration_minutes: u32::try_from(profile.max_duration_minutes).unwrap_or(0),
        },
        hooks: hooks_from_row(profile)?,
        scripts: crate::scripts::from_row(&profile.scripts)?,
        retention: retention_from_row(profile)?,
        check: check_from_row(profile)?,
    };
    let mut compiler_profile = validated.to_compiler_profile();
    compiler_profile.profile_tag = Some(profile_tag(profile.id));
    Ok(compiler_profile)
}

pub(super) fn compiler_hook(hook: &HookRef) -> CompilerHookRef {
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
        body.scripts,
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
        body.scripts,
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
    scripts: Vec<ScriptDef>,
    retention: Option<Retention>,
    check: Option<CheckOptions>,
) -> Result<ValidatedProfile, ApiError> {
    let name = validate_profile_name("name", &name.unwrap_or_default())?;
    let repository_id = parse_repository_id("repository_id", &repository_id)?;
    let paths = validate_paths(paths)?;
    let excludes = validate_patterns("excludes", excludes)?;
    ensure_count("excludes", excludes.len(), MAX_EXCLUDES)?;
    let exclude_files = validate_patterns("exclude_files", exclude_files)?;
    ensure_count("exclude_files", exclude_files.len(), MAX_EXCLUDE_FILES)?;
    let tags = validate_tags("tags", tags)?;
    ensure_count("tags", tags.len(), MAX_TAGS)?;
    validate_limits(&options)?;
    let hooks = validate_hooks(hooks)?;
    let scripts = crate::scripts::validate_scripts(scripts, "scripts")?;
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
        scripts,
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

fn validate_hooks(hooks: Vec<HookRef>) -> Result<Vec<HookRef>, ApiError> {
    ensure_count("hooks", hooks.len(), MAX_HOOKS)?;
    let mut result = Vec::with_capacity(hooks.len());
    for (index, hook) in hooks.into_iter().enumerate() {
        let hook_id = validate_identifier("hooks.hook_id", &hook.hook_id)?;
        if hook.parameters.len() > MAX_HOOK_PARAMETERS {
            return Err(ApiError::invalid_field(
                format!("hooks[{index}].parameters"),
                Reason::TooLong,
            ));
        }
        let mut parameters = Vec::with_capacity(hook.parameters.len());
        for (position, parameter) in hook.parameters.into_iter().enumerate() {
            let field = format!("hooks[{index}].parameters[{position}]");
            if !restichub_compiler::hook::is_valid_parameter_name(&parameter.name) {
                return Err(ApiError::invalid_field(
                    format!("{field}.name"),
                    Reason::InvalidCharacter,
                ));
            }
            // The text form of a typed value: a number, a flag, an enumeration value, a string or
            // the identifier of a secret. Control characters never belong to it (FR-PR-10).
            if parameter.value.len() > MAX_HOOK_VALUE {
                return Err(ApiError::invalid_field(
                    format!("{field}.value"),
                    Reason::TooLong,
                ));
            }
            if restichub_proto::validate::has_forbidden(&parameter.value) {
                return Err(ApiError::invalid_field(
                    format!("{field}.value"),
                    Reason::InvalidCharacter,
                ));
            }
            parameters.push(parameter);
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
            "stall_timeout_minutes": profile.options.stall_timeout_minutes,
            "max_duration_minutes": profile.options.max_duration_minutes,
        },
        "hooks": profile.hooks,
        "scripts": crate::scripts::audit_value(&profile.scripts),
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
            "stall_timeout_minutes": profile.stall_timeout_minutes,
            "max_duration_minutes": profile.max_duration_minutes,
        },
        "hooks": profile.hooks,
        "scripts": stored_scripts_value(&profile.scripts),
        "retention": profile.retention,
        "check": profile.check_options,
    })
}

/// The stored scripts as the audit diff shows them. A row that does not parse shows as null.
fn stored_scripts_value(scripts: &Value) -> Value {
    crate::scripts::from_row(scripts)
        .map_or(Value::Null, |scripts| crate::scripts::audit_value(&scripts))
}

fn format_time(value: time::OffsetDateTime) -> String {
    use time::format_description::well_known::Rfc3339;
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}
