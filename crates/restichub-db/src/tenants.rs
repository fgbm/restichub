//! Tenant operations.
//!
//! Only a [`PlatformTx`] can create and list tenants (ADR-0023, section Tenant isolation). A
//! tenant-scoped transaction reads its own row only.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::enums::TenantStatus;
use crate::ids::{TenantId, UserId};
use crate::tx::{PlatformTx, TenantTx};

pub const DEFAULT_PAGE_LIMIT: i64 = 50;

pub const MAX_PAGE_LIMIT: i64 = 200;

#[derive(Debug, Clone)]
pub struct Tenant {
    pub id: TenantId,
    /// Unique.
    pub name: String,
    pub status: TenantStatus,
}

/// A tenant row with the data of the platform list and the platform read (FR-TN-02).
///
/// The platform endpoints return the count of the users only. They never return a user list or a
/// secret of another tenant.
#[derive(Debug, Clone)]
pub struct TenantSummary {
    pub id: TenantId,
    pub name: String,
    pub status: TenantStatus,
    pub created_at: OffsetDateTime,
    pub user_count: i64,
}

#[derive(Debug, Clone, Default)]
pub struct TenantFilter {
    /// Without a filter the list excludes a deleted tenant; [`TenantStatus::Deleted`] shows them.
    pub status: Option<TenantStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TenantCursor {
    pub created_at: OffsetDateTime,
    pub id: TenantId,
}

#[derive(Debug, Clone, Copy)]
pub struct TenantPage {
    /// Clamped to [`MAX_PAGE_LIMIT`].
    pub limit: i64,
    pub after: Option<TenantCursor>,
}

impl Default for TenantPage {
    fn default() -> Self {
        Self {
            limit: DEFAULT_PAGE_LIMIT,
            after: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TenantPageResult {
    pub tenants: Vec<TenantSummary>,
    pub has_more: bool,
}

/// Returns true if the control plane dispatches jobs to the tenant (FR-TN-02).
///
/// The future dispatcher calls this function. Only an `active` tenant receives jobs. A suspended
/// tenant and a deleted tenant receive none. The agents of the tenant stay connected: the control
/// plane does not close their connections when the tenant is suspended.
pub fn is_dispatch_allowed(status: TenantStatus) -> bool {
    status == TenantStatus::Active
}

/// Optional first administrator of a new tenant.
#[derive(Debug, Clone)]
pub struct BootstrapAdmin<'a> {
    /// Unique for the tenant.
    pub username: &'a str,
    /// Argon2id PHC string.
    pub password_hash: &'a str,
    pub display_name: &'a str,
}

/// Input of [`create_with_bootstrap`].
///
/// The caller wraps the data encryption key (DEK). The caller passes the wrapped bytes. The caller
/// also passes the identifier of the tenant, because the associated data of the wrapped DEK holds
/// the tenant id: the caller must know the id before it wraps the DEK.
///
/// The caller passes the actor and the source IP of the operation. The function writes them on the
/// audit records of the creation.
#[derive(Debug, Clone)]
pub struct BootstrapTenant<'a> {
    pub id: TenantId,
    /// Unique.
    pub name: &'a str,
    pub key_version: i32,
    pub kek_id: &'a str,
    pub wrapped_dek: &'a [u8],
    pub nonce: &'a [u8],
    pub actor: crate::audit::Actor,
    pub source_ip: Option<std::net::IpAddr>,
    pub admin: Option<BootstrapAdmin<'a>>,
}

/// Only this tenant is valid for `PlatformTx::enter_tenant`.
pub async fn create(tx: &mut PlatformTx<'_>, name: &str) -> Result<TenantId, DbError> {
    create_with_id(tx, TenantId::new(), name).await
}

/// Only this tenant is valid for [`PlatformTx::enter_tenant`].
async fn create_with_id(
    tx: &mut PlatformTx<'_>,
    id: TenantId,
    name: &str,
) -> Result<TenantId, DbError> {
    sqlx::query("INSERT INTO tenants (id, tenant_id, name) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(id)
        .bind(name)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    tx.mark_created(id);
    Ok(id)
}

/// The function writes every row in the transaction of the caller. The rows commit together or not
/// at all.
pub async fn create_with_bootstrap(
    tx: &mut PlatformTx<'_>,
    input: &BootstrapTenant<'_>,
) -> Result<TenantId, DbError> {
    let tenant = create_with_id(tx, input.id, input.name).await?;
    tx.enter_tenant(tenant).await?;

    sqlx::query(
        "INSERT INTO tenant_keys (tenant_id, key_version, kek_id, wrapped_dek, nonce) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(tenant)
    .bind(input.key_version)
    .bind(input.kek_id)
    .bind(input.wrapped_dek)
    .bind(input.nonce)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;

    if let Some(admin) = &input.admin {
        let user = UserId::new();
        sqlx::query(
            "INSERT INTO users (id, tenant_id, username, password_hash, display_name) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user)
        .bind(tenant)
        .bind(admin.username)
        .bind(admin.password_hash)
        .bind(admin.display_name)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
        sqlx::query("INSERT INTO memberships (tenant_id, user_id, role) VALUES ($1, $2, 'admin')")
            .bind(tenant)
            .bind(user)
            .execute(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
        let mut user_event =
            crate::audit::AuditEvent::new(crate::audit::AuditAction::UserCreate, "user");
        user_event.actor = input.actor.clone();
        user_event.source_ip = input.source_ip;
        user_event.object_id = Some(user.as_uuid());
        crate::audit::insert_platform(tx, tenant, &user_event).await?;
    }

    // The platform tenant records the creation. The policy of migration 0014 accepts the platform
    // tenant directly, so the transaction needs no tenant scope for this write.
    let mut tenant_event =
        crate::audit::AuditEvent::new(crate::audit::AuditAction::TenantCreate, "tenant");
    tenant_event.actor = input.actor.clone();
    tenant_event.source_ip = input.source_ip;
    tenant_event.object_id = Some(tenant.as_uuid());
    crate::audit::insert_platform(tx, crate::PLATFORM_TENANT_ID, &tenant_event).await?;

    Ok(tenant)
}

pub async fn list(tx: &mut PlatformTx<'_>) -> Result<Vec<Tenant>, DbError> {
    let rows = sqlx::query_as::<_, (Uuid, String, TenantStatus)>(
        "SELECT id, name, status FROM tenants ORDER BY id",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(|(id, name, status)| Tenant {
            id: TenantId::from_uuid(id),
            name,
            status,
        })
        .collect())
}

/// Platform operation (FR-TN-02).
pub async fn set_status(
    tx: &mut PlatformTx<'_>,
    id: TenantId,
    status: TenantStatus,
) -> Result<bool, DbError> {
    let result = sqlx::query("UPDATE tenants SET status = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(status.as_str())
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// The result of a status transition of a tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    Changed,
    AlreadyInTarget,
    NotFound,
    Refused,
}

/// Moves a tenant from one status to another and writes the audit records.
///
/// The function changes the status only when the current status is `from`. A concurrent change
/// therefore wins or loses cleanly: the update of the loser touches no row. When the update touches
/// no row, the function re-reads the status. A current status equal to `to` is idempotent
/// ([`Transition::AlreadyInTarget`]); another status is [`Transition::Refused`].
///
/// On a change the function writes `tenant_event` in the affected tenant (with
/// [`crate::audit::insert_platform_for`]) and `platform_event` in the platform tenant, in the
/// transaction of the caller. The records commit with the status change or not at all.
pub async fn set_status_from(
    tx: &mut PlatformTx<'_>,
    id: TenantId,
    from: TenantStatus,
    to: TenantStatus,
    platform_event: &crate::audit::AuditEvent,
    tenant_event: &crate::audit::AuditEvent,
) -> Result<Transition, DbError> {
    let changed = sqlx::query(
        "UPDATE tenants SET status = $2, updated_at = now() WHERE id = $1 AND status = $3",
    )
    .bind(id)
    .bind(to.as_str())
    .bind(from.as_str())
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?
    .rows_affected()
        == 1;
    if changed {
        crate::audit::insert_platform_for(tx, id, tenant_event).await?;
        crate::audit::insert_platform(tx, crate::PLATFORM_TENANT_ID, platform_event).await?;
        return Ok(Transition::Changed);
    }
    let current: Option<(TenantStatus,)> =
        sqlx::query_as("SELECT status FROM tenants WHERE id = $1")
            .bind(id)
            .fetch_optional(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    match current {
        None => Ok(Transition::NotFound),
        Some((status,)) if status == to => Ok(Transition::AlreadyInTarget),
        Some(_) => Ok(Transition::Refused),
    }
}

/// Reads one tenant in a platform transaction (FR-TN-02).
///
/// The platform scope reads every row of `tenants`. The function returns the user count only, never
/// a user row.
pub async fn get_platform(
    tx: &mut PlatformTx<'_>,
    id: TenantId,
) -> Result<Option<TenantSummary>, DbError> {
    let row = sqlx::query_as::<_, SummaryRow>(
        "SELECT tenant.id, tenant.name, tenant.status, tenant.created_at, \
                coalesce(counts.user_count, 0) \
         FROM tenants AS tenant \
         LEFT JOIN public.tenant_user_counts() AS counts ON counts.tenant_id = tenant.id \
         WHERE tenant.id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(TenantSummary::from))
}

/// Lists the tenants in a platform transaction (FR-TN-02).
///
/// The list orders the tenants by creation time and identifier, from the newest to the oldest. The
/// default list (no status filter) excludes a deleted tenant. The filter `status = deleted` shows
/// the deleted tenants. The list reads one row more than the page holds, to report `has_more`
/// correctly. The keyset cursor of `page.after` continues the list.
pub async fn list_platform(
    tx: &mut PlatformTx<'_>,
    filter: &TenantFilter,
    page: &TenantPage,
) -> Result<TenantPageResult, DbError> {
    let limit = page.limit.clamp(1, MAX_PAGE_LIMIT);
    let (cursor_time, cursor_id) = match page.after {
        Some(cursor) => (Some(cursor.created_at), Some(cursor.id.as_uuid())),
        None => (None, None),
    };
    let rows = sqlx::query_as::<_, SummaryRow>(
        "SELECT tenant.id, tenant.name, tenant.status, tenant.created_at, \
                coalesce(counts.user_count, 0) \
         FROM tenants AS tenant \
         LEFT JOIN public.tenant_user_counts() AS counts ON counts.tenant_id = tenant.id \
         WHERE (($1::text IS NULL AND tenant.status <> 'deleted') OR tenant.status = $1) \
           AND ($2::timestamptz IS NULL \
                OR (tenant.created_at, tenant.id) < ($2::timestamptz, $3::uuid)) \
         ORDER BY tenant.created_at DESC, tenant.id DESC \
         LIMIT $4",
    )
    .bind(filter.status.map(TenantStatus::as_str))
    .bind(cursor_time)
    .bind(cursor_id)
    .bind(limit + 1)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let mut tenants: Vec<TenantSummary> = rows.into_iter().map(TenantSummary::from).collect();
    let has_more = tenants.len() as i64 > limit;
    tenants.truncate(limit as usize);
    Ok(TenantPageResult { tenants, has_more })
}

type SummaryRow = (Uuid, String, TenantStatus, OffsetDateTime, i64);

impl From<SummaryRow> for TenantSummary {
    fn from(row: SummaryRow) -> Self {
        let (id, name, status, created_at, user_count) = row;
        Self {
            id: TenantId::from_uuid(id),
            name,
            status,
            created_at,
            user_count,
        }
    }
}

/// Reads the row of the tenant of the transaction.
///
/// The policy `tenants_self_read` shows the own row only. The function returns `None` for another
/// tenant.
pub async fn get(tx: &mut TenantTx<'_>, id: TenantId) -> Result<Option<Tenant>, DbError> {
    let row = sqlx::query_as::<_, (Uuid, String, TenantStatus)>(
        "SELECT id, name, status FROM tenants WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(|(id, name, status)| Tenant {
        id: TenantId::from_uuid(id),
        name,
        status,
    }))
}
