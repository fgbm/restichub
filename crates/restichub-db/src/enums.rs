//! The closed sets that the control plane stores (ADR-0023).
//!
//! The database and the control plane share these types. The string value of each variant is the
//! stored value of a text column. It is stable: do not change a string.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The role of an actor in a tenant (requirements section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum Role {
    PlatformAdmin,
    Admin,
    Operator,
    Viewer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlatformAdmin => "platform_admin",
            Self::Admin => "admin",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "platform_admin" => Some(Self::PlatformAdmin),
            "admin" => Some(Self::Admin),
            "operator" => Some(Self::Operator),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }

    /// Higher roles include the lower roles.
    pub fn rank(self) -> u8 {
        match self {
            Self::PlatformAdmin => 4,
            Self::Admin => 3,
            Self::Operator => 2,
            Self::Viewer => 1,
        }
    }

    pub fn allows(self, minimum: Self) -> bool {
        self.rank() >= minimum.rank()
    }

    /// Used by the count of the last Admin.
    pub fn is_admin(self) -> bool {
        matches!(self, Self::Admin | Self::PlatformAdmin)
    }
}

/// Tenant status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum TenantStatus {
    Active,
    /// Cannot sign in; its API tokens fail (AC-TN-03).
    Suspended,
    /// Keeps its rows; the retention task removes them later.
    Deleted,
}

impl TenantStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Deleted => "deleted",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "suspended" => Some(Self::Suspended),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum AgentStatus {
    /// Accepted at each handshake and each stream open.
    Active,
    /// The stream layer refuses the agent and closes an open stream (FR-ID-04).
    Revoked,
}

impl AgentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
}

/// Whether a host trusts scripts from the control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum ServerScriptsState {
    /// The agent does not report the state. The control plane treats it as disabled.
    Unknown,
    /// The host administrator did not turn the mode on. This is the default.
    Disabled,
    Enabled,
    /// Another account can change the setting file, so the mode is off.
    Insecure,
}

impl ServerScriptsState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
            Self::Insecure => "insecure",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "unknown" => Some(Self::Unknown),
            "disabled" => Some(Self::Disabled),
            "enabled" => Some(Self::Enabled),
            "insecure" => Some(Self::Insecure),
            _ => None,
        }
    }
}

/// Administrator state of a tenant, derived from the memberships.
///
/// The control plane computes the state from the memberships: an active Admin count of zero gives
/// [`AdminState::None`]. The state is not stored, so it cannot drift from the memberships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdminState {
    None,
    Present,
}

impl AdminState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Present => "present",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "present" => Some(Self::Present),
            _ => None,
        }
    }

    /// Zero active administrators give [`AdminState::None`].
    pub fn from_active_admins(count: i64) -> Self {
        if count == 0 {
            Self::None
        } else {
            Self::Present
        }
    }
}

/// The role of a certificate authority (ADR-0026). The string value is part of the associated
/// data of the wrapped key; do not change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum CaRole {
    Root,
    Issuing,
}

impl CaRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Issuing => "issuing",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "root" => Some(Self::Root),
            "issuing" => Some(Self::Issuing),
            _ => None,
        }
    }
}

/// The role of a job signing key (ADR-0026, section Job signing keys). The string value is part of
/// the associated data of the wrapped key; do not change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum SigningKeyRole {
    /// Signs the jobs now.
    Current,
    /// Delivered to the agents and promoted on the next rotation.
    Next,
}

impl SigningKeyRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Next => "next",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "current" => Some(Self::Current),
            "next" => Some(Self::Next),
            _ => None,
        }
    }
}

/// A closed set of repository backends (FR-RP-01).
///
/// The MVP backends are S3-compatible, rest-server and a local path (ADR-0022). The string value of
/// each variant is the stored value of the `backend` column. It is stable: do not change a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum BackendKind {
    /// An S3-compatible object store such as MinIO or Ceph.
    S3,
    /// A rest-server reached over plain HTTP or over TLS.
    Rest,
    /// A path on the host of the agent that runs the job.
    Local,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::S3 => "s3",
            Self::Rest => "rest",
            Self::Local => "local",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "s3" => Some(Self::S3),
            "rest" => Some(Self::Rest),
            "local" => Some(Self::Local),
            _ => None,
        }
    }
}

/// The kind is general, not init-specific. The string value of each variant is the stored value of
/// the `kind` column. It is stable: do not change a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum JobKind {
    /// Creates a new restic repository when the repository does not exist (FR-RP-02).
    Init,
    /// Reads the repository configuration (FR-RP-02).
    TestAccess,
    /// Creates one snapshot of the selected paths on the agent.
    Backup,
    /// Tests the repository structure.
    Check,
    /// Applies the retention policy.
    Forget,
    /// Removes unused data.
    Prune,
    /// Restores a snapshot.
    Restore,
    /// Removes stale locks.
    Unlock,
    /// Removes every lock of a repository. Only an Admin action starts it (FR-SC-06).
    UnlockAll,
    /// Lists the snapshots of a repository for the Restore screen (FR-RS-01).
    Snapshots,
    /// Lists one directory of a snapshot for the Restore screen (FR-RS-02).
    Ls,
    /// A job that does not fit any other kind of the closed set.
    Other,
}

/// The kinds that share the repository ([`JobKind::shares_repository`]), as the SQL list of a
/// predicate. The partial unique index `jobs_active_repository` counts the same kinds, so a change
/// here also needs a migration that rebuilds the index.
macro_rules! shared_kinds {
    () => {
        "'backup', 'snapshots', 'ls', 'restore'"
    };
}
pub(crate) use shared_kinds;

/// The kinds whose success proves that the repository holds no lock ([`JobKind::holds_exclusive_lock`]),
/// as the SQL list of a predicate.
macro_rules! exclusive_lock_kinds {
    () => {
        "'check', 'forget', 'prune', 'unlock', 'unlock_all'"
    };
}
pub(crate) use exclusive_lock_kinds;

impl JobKind {
    pub const ALL: [Self; 12] = [
        Self::Init,
        Self::TestAccess,
        Self::Backup,
        Self::Check,
        Self::Forget,
        Self::Prune,
        Self::Restore,
        Self::Unlock,
        Self::UnlockAll,
        Self::Snapshots,
        Self::Ls,
        Self::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::TestAccess => "test_access",
            Self::Backup => "backup",
            Self::Check => "check",
            Self::Forget => "forget",
            Self::Prune => "prune",
            Self::Restore => "restore",
            Self::Unlock => "unlock",
            Self::UnlockAll => "unlock_all",
            Self::Snapshots => "snapshots",
            Self::Ls => "ls",
            Self::Other => "other",
        }
    }

    /// True for a kind that runs next to another job on its repository (FR-SC-04).
    pub fn shares_repository(self) -> bool {
        matches!(
            self,
            Self::Backup | Self::Snapshots | Self::Ls | Self::Restore
        )
    }

    /// True for a kind that runs alone on its repository (FR-SC-04).
    ///
    /// An exclusive kind starts only when no job runs on the repository, and no job starts while it
    /// runs. The other kinds share the repository up to the limit of the queue.
    pub fn is_exclusive(self) -> bool {
        matches!(
            self,
            Self::Init | Self::Check | Self::Forget | Self::Prune | Self::Unlock | Self::UnlockAll
        )
    }

    /// True for a kind whose success proves that the repository holds no lock (FR-SC-06).
    ///
    /// `check`, `forget` and `prune` take the exclusive lock of restic, so a success proves that no
    /// lock held the repository. `unlock` and `unlock_all` take no lock; they remove the locks, so
    /// their success proves the same. A kind that shares the repository runs next to a lock, so its
    /// success proves nothing.
    pub fn holds_exclusive_lock(self) -> bool {
        matches!(
            self,
            Self::Check | Self::Forget | Self::Prune | Self::Unlock | Self::UnlockAll
        )
    }

    /// True for a kind that runs once for a repository (ADR-0050).
    ///
    /// A person starts a `restore` and a `test_access`, and an Administrator starts an `init` and an
    /// `unlock_all`. The control plane starts an `unlock` after an agent loss. The maintenance
    /// schedule repeats `check`, `forget` and `prune`, so their alerts close at their next success
    /// (ADR-0036).
    pub fn is_one_time(self) -> bool {
        matches!(
            self,
            Self::Restore | Self::TestAccess | Self::Init | Self::Unlock | Self::UnlockAll
        )
    }

    /// True for a job that answers a person who waits in the Restore screen. It does not wait
    /// behind the jobs of its agent, because a long backup would hold the screen for hours.
    pub fn is_interactive(self) -> bool {
        matches!(self, Self::Snapshots | Self::Ls)
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "init" => Some(Self::Init),
            "test_access" => Some(Self::TestAccess),
            "backup" => Some(Self::Backup),
            "check" => Some(Self::Check),
            "forget" => Some(Self::Forget),
            "prune" => Some(Self::Prune),
            "restore" => Some(Self::Restore),
            "unlock" => Some(Self::Unlock),
            "unlock_all" => Some(Self::UnlockAll),
            "snapshots" => Some(Self::Snapshots),
            "ls" => Some(Self::Ls),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

/// `queued` is the state before the send and `sent` the state after it, until the agent
/// acknowledges the job. The string value of each variant is the stored value of the `status`
/// column. It is stable: do not change a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum JobStatus {
    /// The row exists; the control plane did not send the job.
    Queued,
    /// The control plane sent the job to the agent over the stream.
    Sent,
    /// The agent acknowledged the job and runs it (FR-PR-06).
    Running,
    /// The job finished and every step reported success to the control plane.
    Succeeded,
    /// The job finished and at least one step reported a failure.
    Failed,
    /// The job expired before the agent ran it, so the control plane discards it (FR-SC-07).
    Expired,
    /// The repository stayed locked after the last retry, so the queue gave up (FR-SC-05).
    DeadLetter,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Sent => "sent",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Expired => "expired",
            Self::DeadLetter => "dead_letter",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "sent" => Some(Self::Sent),
            "running" => Some(Self::Running),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "expired" => Some(Self::Expired),
            "dead_letter" => Some(Self::DeadLetter),
            _ => None,
        }
    }

    /// True for a terminal status. A job in a terminal status never changes again.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Expired | Self::DeadLetter
        )
    }
}

/// How many jobs the scheduler creates after a downtime, or after an agent was not active.
///
/// The policy decides how many jobs the scheduler creates after the control plane was down, or
/// after an agent that was not active for a while. The string value of each variant is the stored
/// value of the `missed_run_policy` column. It is stable: do not change a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum MissedRunPolicy {
    /// One job for each missed fire time.
    RunAll,
    /// One job for the latest missed fire time.
    LatestOnly,
    /// No job for a missed fire time.
    Skip,
}

impl MissedRunPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RunAll => "run_all",
            Self::LatestOnly => "latest_only",
            Self::Skip => "skip",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "run_all" => Some(Self::RunAll),
            "latest_only" => Some(Self::LatestOnly),
            "skip" => Some(Self::Skip),
            _ => None,
        }
    }
}

/// The maintenance task of a repository schedule.
///
/// The string value of each variant is the stored value of the `task` column. It is stable: do not
/// change a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum MaintenanceTask {
    /// The `forget` steps of every profile, then one `prune` step.
    Retention,
    /// One `check` step, with the data subset of the first profile that sets one.
    Check,
}

impl MaintenanceTask {
    pub const ALL: [Self; 2] = [Self::Retention, Self::Check];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retention => "retention",
            Self::Check => "check",
        }
    }

    /// The retention cycle is one `forget` job that also holds the `prune` step. The check is a
    /// `check` job.
    pub fn job_kind(self) -> JobKind {
        match self {
            Self::Retention => JobKind::Forget,
            Self::Check => JobKind::Check,
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "retention" => Some(Self::Retention),
            "check" => Some(Self::Check),
            _ => None,
        }
    }
}

/// The value comes from the `outcome` field of the `JobResult` message. It is a closed set, so the
/// `result_code` column holds one of these strings. It is stable: do not change a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum JobResultCode {
    /// Every step succeeded.
    Succeeded,
    /// A step failed, so the job result carries the status of that step.
    Failed,
    /// The repository password or the backend credentials are wrong (AC-RP-01).
    ConfigError,
    /// The repository does not exist. A test_access job reports it (FR-RP-02).
    NotInitialized,
    /// The repository is not at the stored location on this host. A `not_initialized` result on an
    /// initialized repository gives this code: the path, the mount or the host changed.
    AccessError,
    /// The control plane cancelled the job (FR-AG-10).
    Cancelled,
    /// Every step finished, but restic reported warnings (exit code 3).
    SucceededWithWarnings,
    /// A lock on the repository stopped the job (restic exit code 11, FR-SC-05).
    RepositoryLocked,
    /// An option of the profile needs a newer restic than the agent has (AC-AG-09).
    Incompatible,
}

impl JobResultCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::ConfigError => "config_error",
            Self::NotInitialized => "not_initialized",
            Self::AccessError => "access_error",
            Self::Cancelled => "cancelled",
            Self::SucceededWithWarnings => "succeeded_with_warnings",
            Self::RepositoryLocked => "repository_locked",
            Self::Incompatible => "incompatible",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "config_error" => Some(Self::ConfigError),
            "not_initialized" => Some(Self::NotInitialized),
            "access_error" => Some(Self::AccessError),
            "cancelled" => Some(Self::Cancelled),
            "succeeded_with_warnings" => Some(Self::SucceededWithWarnings),
            "repository_locked" => Some(Self::RepositoryLocked),
            "incompatible" => Some(Self::Incompatible),
            _ => None,
        }
    }
}

/// Audit actor type (ADR-0023, section Audit log).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum ActorType {
    User,
    ApiToken,
    Agent,
    System,
}

impl ActorType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::ApiToken => "api_token",
            Self::Agent => "agent",
            Self::System => "system",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "api_token" => Some(Self::ApiToken),
            "agent" => Some(Self::Agent),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

/// How one step of a job ended. The agent maps the restic exit code to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum StepStatus {
    Success,
    Warning,
    Retry,
    ConfigError,
    Cancelled,
    Failed,
    Skipped,
}

impl StepStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Retry => "retry",
            Self::ConfigError => "config_error",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "success" => Some(Self::Success),
            "warning" => Some(Self::Warning),
            "retry" => Some(Self::Retry),
            "config_error" => Some(Self::ConfigError),
            "cancelled" => Some(Self::Cancelled),
            "failed" => Some(Self::Failed),
            "skipped" => Some(Self::Skipped),
            _ => None,
        }
    }
}

/// Where a log line came from: a restic stream or the agent itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
    Agent,
}

impl LogStream {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::Agent => "agent",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "stdout" => Some(Self::Stdout),
            "stderr" => Some(Self::Stderr),
            "agent" => Some(Self::Agent),
            _ => None,
        }
    }
}

/// The level that the control plane derives for a job log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "info" => Some(Self::Info),
            "warn" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActorType, AdminState, AgentStatus, BackendKind, CaRole, JobKind, JobResultCode, JobStatus,
        LogLevel, LogStream, Role, SigningKeyRole, StepStatus, TenantStatus,
    };

    #[test]
    fn the_role_strings_are_stable() {
        for role in [
            Role::PlatformAdmin,
            Role::Admin,
            Role::Operator,
            Role::Viewer,
        ] {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse("nope"), None);
    }

    #[test]
    fn the_tenant_status_strings_are_stable() {
        for status in [
            TenantStatus::Active,
            TenantStatus::Suspended,
            TenantStatus::Deleted,
        ] {
            assert_eq!(TenantStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(TenantStatus::parse("nope"), None);
    }

    #[test]
    fn the_agent_status_strings_are_stable() {
        for status in [AgentStatus::Active, AgentStatus::Revoked] {
            assert_eq!(AgentStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(AgentStatus::parse("nope"), None);
    }

    #[test]
    fn the_admin_state_strings_are_stable() {
        for state in [AdminState::None, AdminState::Present] {
            assert_eq!(AdminState::parse(state.as_str()), Some(state));
        }
        assert_eq!(AdminState::parse("nope"), None);
        assert_eq!(AdminState::from_active_admins(0), AdminState::None);
        assert_eq!(AdminState::from_active_admins(2), AdminState::Present);
    }

    #[test]
    fn the_ca_role_strings_are_stable() {
        for role in [CaRole::Root, CaRole::Issuing] {
            assert_eq!(CaRole::parse(role.as_str()), Some(role));
        }
        assert_eq!(CaRole::parse("nope"), None);
    }

    #[test]
    fn the_signing_key_role_strings_are_stable() {
        for role in [SigningKeyRole::Current, SigningKeyRole::Next] {
            assert_eq!(SigningKeyRole::parse(role.as_str()), Some(role));
        }
        assert_eq!(SigningKeyRole::parse("nope"), None);
    }

    #[test]
    fn the_actor_type_strings_are_stable() {
        for actor_type in [
            ActorType::User,
            ActorType::ApiToken,
            ActorType::Agent,
            ActorType::System,
        ] {
            assert_eq!(ActorType::parse(actor_type.as_str()), Some(actor_type));
        }
        assert_eq!(ActorType::parse("nope"), None);
    }

    #[test]
    fn the_backend_kind_strings_are_stable() {
        for kind in [BackendKind::S3, BackendKind::Rest, BackendKind::Local] {
            assert_eq!(BackendKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(BackendKind::parse("sftp"), None);
    }

    #[test]
    fn the_job_kind_strings_are_stable() {
        for kind in JobKind::ALL {
            assert_eq!(JobKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(JobKind::parse("nope"), None);
    }

    #[test]
    fn the_sql_list_of_the_shared_kinds_holds_the_enum_rule() {
        let list = shared_kinds!();
        for kind in JobKind::ALL {
            assert_eq!(
                list.contains(&format!("'{}'", kind.as_str())),
                kind.shares_repository(),
                "{}",
                kind.as_str()
            );
        }
    }

    #[test]
    fn the_sql_list_of_the_exclusive_lock_kinds_holds_the_enum_rule() {
        let list = exclusive_lock_kinds!();
        for kind in JobKind::ALL {
            assert_eq!(
                list.contains(&format!("'{}'", kind.as_str())),
                kind.holds_exclusive_lock(),
                "{}",
                kind.as_str()
            );
        }
    }

    #[test]
    fn the_one_time_kinds_skip_the_kinds_of_the_maintenance_schedule() {
        let one_time: Vec<&str> = JobKind::ALL
            .iter()
            .filter(|kind| kind.is_one_time())
            .map(|kind| kind.as_str())
            .collect();
        assert_eq!(
            one_time,
            ["init", "test_access", "restore", "unlock", "unlock_all"]
        );
    }

    #[test]
    fn the_job_status_strings_are_stable() {
        for status in [
            JobStatus::Queued,
            JobStatus::Sent,
            JobStatus::Running,
            JobStatus::Succeeded,
            JobStatus::Failed,
            JobStatus::Expired,
            JobStatus::DeadLetter,
        ] {
            assert_eq!(JobStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(JobStatus::parse("nope"), None);
        assert!(JobStatus::Succeeded.is_terminal());
        assert!(JobStatus::Failed.is_terminal());
        assert!(JobStatus::Expired.is_terminal());
        assert!(JobStatus::DeadLetter.is_terminal());
        assert!(!JobStatus::Queued.is_terminal());
        assert!(!JobStatus::Sent.is_terminal());
        assert!(!JobStatus::Running.is_terminal());
    }

    #[test]
    fn the_job_result_code_strings_are_stable() {
        for code in [
            JobResultCode::Succeeded,
            JobResultCode::Failed,
            JobResultCode::ConfigError,
            JobResultCode::NotInitialized,
            JobResultCode::AccessError,
            JobResultCode::Cancelled,
            JobResultCode::SucceededWithWarnings,
            JobResultCode::RepositoryLocked,
            JobResultCode::Incompatible,
        ] {
            assert_eq!(JobResultCode::parse(code.as_str()), Some(code));
        }
        assert_eq!(JobResultCode::parse("nope"), None);
    }

    #[test]
    fn the_log_and_step_strings_are_stable() {
        for status in [
            StepStatus::Success,
            StepStatus::Warning,
            StepStatus::Retry,
            StepStatus::ConfigError,
            StepStatus::Cancelled,
            StepStatus::Failed,
            StepStatus::Skipped,
        ] {
            assert_eq!(StepStatus::parse(status.as_str()), Some(status));
        }
        for stream in [LogStream::Stdout, LogStream::Stderr, LogStream::Agent] {
            assert_eq!(LogStream::parse(stream.as_str()), Some(stream));
        }
        for level in [LogLevel::Info, LogLevel::Warn, LogLevel::Error] {
            assert_eq!(LogLevel::parse(level.as_str()), Some(level));
        }
        assert_eq!(StepStatus::parse("nope"), None);
    }
}

/// The kind of a planned step, as the job detail shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum PlannedKind {
    Backup,
    /// A step of an allow-list hook in a job that ran before ADR-0040. The history keeps the row.
    Hook,
    Script,
    /// A step of another kind: the job detail needs no more.
    Other,
}

/// Position of a script in the job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum PlannedPhase {
    Before,
    After,
    OnSuccess,
    OnError,
    Finally,
}

/// The kind of a parameter of a profile and of its value.
///
/// The string value is the stored value of the `kind` column and of the profile JSON. It is stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum ParameterKind {
    /// A value that a template field expands and that a person may read.
    Text,
    /// A reference to a secret of the purpose `hook_secret`.
    Secret,
}

impl ParameterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Secret => "secret",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "secret" => Some(Self::Secret),
            _ => None,
        }
    }
}

/// The kind of the object that references a secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum SecretUsageKind {
    Repository,
    Channel,
    Profile,
    JobTemplate,
    /// A value on a machine that the tenant enrolled (FR-PF-10).
    Host,
    /// A value that overrides the value of the host for one plan (FR-PF-10).
    Plan,
}
