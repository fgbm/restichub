//! The profile rows of a tenant (FR-PF-01, FR-PF-03, ADR-0006).
//!
//! One row holds the paths, the excludes, the tags, the options, the retention and the check
//! options. The repository belongs to the plan (ADR-0047). The row never holds a secret value: the
//! repository password and the backend credentials live in the encrypted `secrets` table
//! (ADR-0017).
//!
//! The list settings use text arrays. The retention, the check options and the scripts use a jsonb
//! document. The control plane validates the document with the compiler before it writes the row
//! (FR-PF-05); this module stores it as an opaque [`serde_json::Value`].
//!
//! The general tenant policy of the `profiles` table shows the rows of the tenant of the
//! transaction only (ADR-0023, section Tenant isolation).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::DbError;
use crate::ids::{ProfileId, UserId};
use crate::tx::TenantTx;

pub const DEFAULT_PAGE_LIMIT: i64 = 50;

pub const MAX_PAGE_LIMIT: i64 = 200;

/// The process and I/O priority of the restic steps of a profile.
// The string value of each variant is stored: do not change a string.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum ProfilePriority {
    /// The agent keeps the priority of the operating system default.
    #[default]
    Normal,
    /// Lower process and I/O priority (AC-AG-14).
    Low,
}

/// One backup definition of a tenant (FR-PF-01).
#[derive(Debug, Clone)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub paths_all: Vec<String>,
    pub paths_windows: Vec<String>,
    pub paths_linux: Vec<String>,
    pub paths_macos: Vec<String>,
    pub excludes: Vec<String>,
    pub exclude_files: Vec<String>,
    pub tags: Vec<String>,
    pub use_fs_snapshot: bool,
    pub one_file_system: bool,
    pub exclude_caches: bool,
    pub limit_upload_kib: i64,
    pub limit_download_kib: i64,
    pub priority: ProfilePriority,
    /// Minutes without progress after which the agent stops a step (ADR-0039).
    pub stall_timeout_minutes: i32,
    /// Minutes after which the agent stops the job. Zero means no limit (ADR-0039).
    pub max_duration_minutes: i32,
    pub retention: Option<Value>,
    pub check_options: Option<Value>,
    /// Definitions as a JSON array (FR-HK-09).
    pub scripts: Value,
    /// A JSON array of declarations that the compiler and the save check read (FR-PF-09).
    pub parameters: Value,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub created_by: Option<UserId>,
}

/// The keyset cursor of the profile list. It orders by `(created_at, id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileCursor {
    pub created_at: OffsetDateTime,
    pub id: ProfileId,
}

#[derive(Debug, Clone, Copy)]
pub struct ProfilePage {
    /// Clamped to [`MAX_PAGE_LIMIT`].
    pub limit: i64,
    pub after: Option<ProfileCursor>,
}

impl Default for ProfilePage {
    fn default() -> Self {
        Self {
            limit: DEFAULT_PAGE_LIMIT,
            after: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProfilePageResult {
    pub profiles: Vec<Profile>,
    pub has_more: bool,
}

/// The fields of a new row in the `profiles` table.
#[derive(Debug, Clone)]
pub struct NewProfile<'a> {
    pub id: ProfileId,
    pub name: &'a str,
    pub paths_all: &'a [String],
    pub paths_windows: &'a [String],
    pub paths_linux: &'a [String],
    pub paths_macos: &'a [String],
    pub excludes: &'a [String],
    pub exclude_files: &'a [String],
    pub tags: &'a [String],
    pub use_fs_snapshot: bool,
    pub one_file_system: bool,
    pub exclude_caches: bool,
    pub limit_upload_kib: i64,
    pub limit_download_kib: i64,
    pub priority: ProfilePriority,
    pub stall_timeout_minutes: i32,
    pub max_duration_minutes: i32,
    pub retention: Option<Value>,
    pub check_options: Option<Value>,
    pub scripts: Value,
    /// A JSON array of declarations that the compiler and the save check read (FR-PF-09).
    pub parameters: Value,
    /// Absent for an API token actor.
    pub created_by: Option<UserId>,
}

/// Mutable fields of a stored row.
#[derive(Debug, Clone)]
pub struct ProfileUpdate<'a> {
    pub name: &'a str,
    pub paths_all: &'a [String],
    pub paths_windows: &'a [String],
    pub paths_linux: &'a [String],
    pub paths_macos: &'a [String],
    pub excludes: &'a [String],
    pub exclude_files: &'a [String],
    pub tags: &'a [String],
    pub use_fs_snapshot: bool,
    pub one_file_system: bool,
    pub exclude_caches: bool,
    pub limit_upload_kib: i64,
    pub limit_download_kib: i64,
    pub priority: ProfilePriority,
    /// `None` keeps the stored value.
    pub stall_timeout_minutes: Option<i32>,
    pub max_duration_minutes: Option<i32>,
    pub retention: Option<Value>,
    pub check_options: Option<Value>,
    pub scripts: Value,
    /// A JSON array of declarations that the compiler and the save check read (FR-PF-09).
    pub parameters: Value,
}

pub async fn create(tx: &mut TenantTx<'_>, new: &NewProfile<'_>) -> Result<ProfileId, DbError> {
    sqlx::query(
        "INSERT INTO profiles \
         (id, tenant_id, name, paths_all, paths_windows, paths_linux, paths_macos, \
          excludes, exclude_files, tags, use_fs_snapshot, one_file_system, exclude_caches, \
          limit_upload_kib, limit_download_kib, retention, check_options, scripts, \
          created_by, stall_timeout_minutes, max_duration_minutes, parameters, priority) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, \
                 $18, $19, $20, $21, $22, $23)",
    )
    .bind(new.id)
    .bind(tx.tenant())
    .bind(new.name)
    .bind(new.paths_all)
    .bind(new.paths_windows)
    .bind(new.paths_linux)
    .bind(new.paths_macos)
    .bind(new.excludes)
    .bind(new.exclude_files)
    .bind(new.tags)
    .bind(new.use_fs_snapshot)
    .bind(new.one_file_system)
    .bind(new.exclude_caches)
    .bind(new.limit_upload_kib)
    .bind(new.limit_download_kib)
    .bind(&new.retention)
    .bind(&new.check_options)
    .bind(&new.scripts)
    .bind(new.created_by)
    .bind(new.stall_timeout_minutes)
    .bind(new.max_duration_minutes)
    .bind(&new.parameters)
    .bind(new.priority)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(new.id)
}

/// Lists the profiles of the tenant of the transaction, newest first.
pub async fn list(tx: &mut TenantTx<'_>, page: &ProfilePage) -> Result<ProfilePageResult, DbError> {
    let limit = page.limit.clamp(1, MAX_PAGE_LIMIT);
    let (cursor_time, cursor_id) = match page.after {
        Some(cursor) => (Some(cursor.created_at), Some(cursor.id.as_uuid())),
        None => (None, None),
    };
    let rows = sqlx::query_as::<_, ProfileRow>(
        "SELECT id, name, paths_all, paths_windows, paths_linux, paths_macos, \
                excludes, exclude_files, tags, use_fs_snapshot, one_file_system, exclude_caches, \
                limit_upload_kib, limit_download_kib, priority, stall_timeout_minutes, \
                max_duration_minutes, retention, check_options, scripts, parameters, \
                created_at, updated_at, created_by \
         FROM profiles \
         WHERE ($1::timestamptz IS NULL OR (created_at, id) < ($1::timestamptz, $2::uuid)) \
         ORDER BY created_at DESC, id DESC \
         LIMIT $3",
    )
    .bind(cursor_time)
    .bind(cursor_id)
    .bind(limit + 1)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let mut profiles: Vec<Profile> = rows.into_iter().map(Profile::from).collect();
    let has_more = profiles.len() as i64 > limit;
    profiles.truncate(limit as usize);
    Ok(ProfilePageResult { profiles, has_more })
}

/// Reads one profile of the tenant of the transaction. A profile of another tenant is absent.
pub async fn get(tx: &mut TenantTx<'_>, id: ProfileId) -> Result<Option<Profile>, DbError> {
    let row = sqlx::query_as::<_, ProfileRow>(
        "SELECT id, name, paths_all, paths_windows, paths_linux, paths_macos, \
                excludes, exclude_files, tags, use_fs_snapshot, one_file_system, exclude_caches, \
                limit_upload_kib, limit_download_kib, priority, stall_timeout_minutes, \
                max_duration_minutes, retention, check_options, scripts, parameters, \
                created_at, updated_at, created_by \
         FROM profiles WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(Profile::from))
}

/// Reads one profile and locks its row for the transaction.
///
/// An update or a delete uses this function, so a concurrent change cannot remove the row between
/// the read and the write.
pub async fn get_for_update(
    tx: &mut TenantTx<'_>,
    id: ProfileId,
) -> Result<Option<Profile>, DbError> {
    let row = sqlx::query_as::<_, ProfileRow>(
        "SELECT id, name, paths_all, paths_windows, paths_linux, paths_macos, \
                excludes, exclude_files, tags, use_fs_snapshot, one_file_system, exclude_caches, \
                limit_upload_kib, limit_download_kib, priority, stall_timeout_minutes, \
                max_duration_minutes, retention, check_options, scripts, parameters, \
                created_at, updated_at, created_by \
         FROM profiles WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(Profile::from))
}

/// Writes the mutable fields of a profile and sets `updated_at`. A missing profile returns `false`.
pub async fn update(
    tx: &mut TenantTx<'_>,
    id: ProfileId,
    patch: &ProfileUpdate<'_>,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE profiles SET name = $2, paths_all = $3, paths_windows = $4, \
                paths_linux = $5, paths_macos = $6, excludes = $7, exclude_files = $8, tags = $9, \
                use_fs_snapshot = $10, one_file_system = $11, exclude_caches = $12, \
                limit_upload_kib = $13, limit_download_kib = $14, retention = $15, \
                check_options = $16, scripts = $17, parameters = $20, priority = $21, \
                stall_timeout_minutes = COALESCE($18, stall_timeout_minutes), \
                max_duration_minutes = COALESCE($19, max_duration_minutes), updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(patch.name)
    .bind(patch.paths_all)
    .bind(patch.paths_windows)
    .bind(patch.paths_linux)
    .bind(patch.paths_macos)
    .bind(patch.excludes)
    .bind(patch.exclude_files)
    .bind(patch.tags)
    .bind(patch.use_fs_snapshot)
    .bind(patch.one_file_system)
    .bind(patch.exclude_caches)
    .bind(patch.limit_upload_kib)
    .bind(patch.limit_download_kib)
    .bind(&patch.retention)
    .bind(&patch.check_options)
    .bind(&patch.scripts)
    .bind(patch.stall_timeout_minutes)
    .bind(patch.max_duration_minutes)
    .bind(&patch.parameters)
    .bind(patch.priority)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Removes a profile of the tenant. `true` when it removed the row.
pub async fn delete(tx: &mut TenantTx<'_>, id: ProfileId) -> Result<bool, DbError> {
    let result = sqlx::query("DELETE FROM profiles WHERE id = $1")
        .bind(id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

#[derive(sqlx::FromRow)]
struct ProfileRow {
    id: Uuid,
    name: String,
    paths_all: Vec<String>,
    paths_windows: Vec<String>,
    paths_linux: Vec<String>,
    paths_macos: Vec<String>,
    excludes: Vec<String>,
    exclude_files: Vec<String>,
    tags: Vec<String>,
    use_fs_snapshot: bool,
    one_file_system: bool,
    exclude_caches: bool,
    limit_upload_kib: i64,
    limit_download_kib: i64,
    priority: ProfilePriority,
    stall_timeout_minutes: i32,
    max_duration_minutes: i32,
    retention: Option<Value>,
    check_options: Option<Value>,
    scripts: Value,
    parameters: Value,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    created_by: Option<Uuid>,
}

impl From<ProfileRow> for Profile {
    fn from(row: ProfileRow) -> Self {
        Self {
            id: ProfileId::from_uuid(row.id),
            name: row.name,
            paths_all: row.paths_all,
            paths_windows: row.paths_windows,
            paths_linux: row.paths_linux,
            paths_macos: row.paths_macos,
            excludes: row.excludes,
            exclude_files: row.exclude_files,
            tags: row.tags,
            use_fs_snapshot: row.use_fs_snapshot,
            one_file_system: row.one_file_system,
            exclude_caches: row.exclude_caches,
            limit_upload_kib: row.limit_upload_kib,
            limit_download_kib: row.limit_download_kib,
            priority: row.priority,
            stall_timeout_minutes: row.stall_timeout_minutes,
            max_duration_minutes: row.max_duration_minutes,
            retention: row.retention,
            check_options: row.check_options,
            scripts: row.scripts,
            parameters: row.parameters,
            created_at: row.created_at,
            updated_at: row.updated_at,
            created_by: row.created_by.map(UserId::from_uuid),
        }
    }
}
