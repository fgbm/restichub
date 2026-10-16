//! Cross-tenant authentication lookups (ADR-0023, section Local authentication).
//!
//! Sign-in and token authentication happen before the control plane knows the tenant. The runtime
//! role therefore cannot use the tenant policies. Three narrow `SECURITY DEFINER` functions do the
//! lookup (migration 0008).
//!
//! The caller checks the session expiry, the token expiry and revocation, the user `disabled` flag
//! and the tenant status. These functions only find the row.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::Database;
use crate::DbError;
use crate::enums::{Role, TenantStatus};
use crate::ids::{ApiTokenId, SessionId, TenantId, UserId};

#[derive(Debug, Clone)]
pub struct TenantLookup {
    pub id: TenantId,
    pub status: TenantStatus,
}

/// One active membership of an account in an active tenant (ADR-0025).
#[derive(Debug, Clone)]
pub struct MembershipLookup {
    pub tenant_id: TenantId,
    pub tenant_name: String,
    pub role: Role,
}

#[derive(Debug, Clone)]
pub struct SessionLookup {
    pub tenant_id: TenantId,
    pub session_id: SessionId,
    pub user_id: UserId,
    pub expires_at: OffsetDateTime,
    pub last_seen_at: OffsetDateTime,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApiTokenLookup {
    pub tenant_id: TenantId,
    pub token_id: ApiTokenId,
    pub role: Role,
    pub expires_at: Option<OffsetDateTime>,
    /// Revoked tokens are refused.
    pub revoked_at: Option<OffsetDateTime>,
}

/// Ignores the letter case of the name.
pub async fn find_tenant(db: &Database, name: &str) -> Result<Option<TenantLookup>, DbError> {
    let row =
        sqlx::query_as::<_, (Uuid, TenantStatus)>("SELECT id, status FROM auth_find_tenant($1)")
            .bind(name)
            .fetch_optional(db.pool())
            .await
            .map_err(DbError::Query)?;
    Ok(row.map(|(id, status)| TenantLookup {
        id: TenantId::from_uuid(id),
        status,
    }))
}

pub async fn find_session(
    db: &Database,
    token_hash: &[u8],
) -> Result<Option<SessionLookup>, DbError> {
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            Uuid,
            OffsetDateTime,
            OffsetDateTime,
            Option<String>,
        ),
    >(
        "SELECT tenant_id, session_id, user_id, expires_at, last_seen_at, user_agent \
         FROM auth_find_session($1)",
    )
    .bind(token_hash)
    .fetch_optional(db.pool())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(
        |(tenant_id, session_id, user_id, expires_at, last_seen_at, user_agent)| SessionLookup {
            tenant_id: TenantId::from_uuid(tenant_id),
            session_id: SessionId::from_uuid(session_id),
            user_id: UserId::from_uuid(user_id),
            expires_at,
            last_seen_at,
            user_agent,
        },
    ))
}

pub async fn find_api_token(
    db: &Database,
    token_hash: &[u8],
) -> Result<Option<ApiTokenLookup>, DbError> {
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            Role,
            Option<OffsetDateTime>,
            Option<OffsetDateTime>,
        ),
    >(
        "SELECT tenant_id, token_id, role, expires_at, revoked_at FROM auth_find_api_token($1)",
    )
    .bind(token_hash)
    .fetch_optional(db.pool())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(
        |(tenant_id, token_id, role, expires_at, revoked_at)| ApiTokenLookup {
            tenant_id: TenantId::from_uuid(tenant_id),
            token_id: ApiTokenId::from_uuid(token_id),
            role,
            expires_at,
            revoked_at,
        },
    ))
}

/// Lists the active memberships of an account in active tenants (ADR-0025).
pub async fn list_memberships(
    db: &Database,
    account_id: UserId,
) -> Result<Vec<MembershipLookup>, DbError> {
    let rows = sqlx::query_as::<_, (Uuid, String, Role)>(
        "SELECT tenant_id, tenant_name, role FROM auth_list_memberships($1)",
    )
    .bind(account_id)
    .fetch_all(db.pool())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(|(tenant_id, tenant_name, role)| MembershipLookup {
            tenant_id: TenantId::from_uuid(tenant_id),
            tenant_name,
            role,
        })
        .collect())
}

/// Deletes the sessions of an account. A password change keeps the current session.
///
/// The database function reaches the whole installation only from the platform tenant, from the
/// platform scope, or for the own password change. Otherwise it removes the sessions of the current
/// tenant only.
pub async fn delete_account_sessions(
    tx: &mut crate::TenantTx<'_>,
    account_id: UserId,
    keep_session: Option<SessionId>,
) -> Result<u64, DbError> {
    let (deleted,): (i64,) = sqlx::query_as("SELECT auth_delete_account_sessions($1, $2)")
        .bind(account_id)
        .bind(keep_session)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(deleted as u64)
}

/// Deletes every session of the account in every tenant, in a platform transaction.
pub async fn delete_all_account_sessions(
    tx: &mut crate::PlatformTx<'_>,
    account_id: UserId,
) -> Result<u64, DbError> {
    let (deleted,): (i64,) = sqlx::query_as("SELECT auth_delete_account_sessions($1, NULL)")
        .bind(account_id)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(deleted as u64)
}

/// Deletes one session of the account. The tenant switch removes the source session through it.
pub async fn delete_session(
    tx: &mut crate::TenantTx<'_>,
    session_id: SessionId,
    account_id: UserId,
) -> Result<u64, DbError> {
    let (deleted,): (i64,) = sqlx::query_as("SELECT auth_delete_session($1, $2)")
        .bind(session_id)
        .bind(account_id)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(deleted as u64)
}

/// Gives an existing identity a role in a tenant (Platform Admin only).
///
/// The database function requires the platform scope, takes the account row lock, refuses the
/// platform tenant and re-enables a disabled membership.
pub async fn add_membership(
    tx: &mut crate::PlatformTx<'_>,
    account_id: UserId,
    tenant_id: TenantId,
    role: Role,
) -> Result<bool, DbError> {
    let (added,): (bool,) = sqlx::query_as("SELECT auth_add_membership($1, $2, $3)")
        .bind(account_id)
        .bind(tenant_id)
        .bind(role.as_str())
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(added)
}
