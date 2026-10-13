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

/// A tenant found by name.
#[derive(Debug, Clone)]
pub struct TenantLookup {
    /// The identifier of the tenant.
    pub id: TenantId,
    /// The status of the tenant.
    pub status: TenantStatus,
}

/// A session found by token hash.
#[derive(Debug, Clone)]
pub struct SessionLookup {
    /// The tenant of the session.
    pub tenant_id: TenantId,
    /// The identifier of the session.
    pub session_id: SessionId,
    /// The user of the session.
    pub user_id: UserId,
    /// The absolute expiry time.
    pub expires_at: OffsetDateTime,
    /// The time of the last request.
    pub last_seen_at: OffsetDateTime,
}

/// An API token found by token hash.
#[derive(Debug, Clone)]
pub struct ApiTokenLookup {
    /// The tenant of the token.
    pub tenant_id: TenantId,
    /// The identifier of the token.
    pub token_id: ApiTokenId,
    /// The role of the token.
    pub role: Role,
    /// The optional expiry time.
    pub expires_at: Option<OffsetDateTime>,
    /// The revocation time. A revoked token is refused.
    pub revoked_at: Option<OffsetDateTime>,
}

/// Finds a tenant by name. The comparison ignores the letter case.
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

/// Finds a session by token hash.
pub async fn find_session(
    db: &Database,
    token_hash: &[u8],
) -> Result<Option<SessionLookup>, DbError> {
    let row = sqlx::query_as::<_, (Uuid, Uuid, Uuid, OffsetDateTime, OffsetDateTime)>(
        "SELECT tenant_id, session_id, user_id, expires_at, last_seen_at FROM auth_find_session($1)",
    )
    .bind(token_hash)
    .fetch_optional(db.pool())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(
        |(tenant_id, session_id, user_id, expires_at, last_seen_at)| SessionLookup {
            tenant_id: TenantId::from_uuid(tenant_id),
            session_id: SessionId::from_uuid(session_id),
            user_id: UserId::from_uuid(user_id),
            expires_at,
            last_seen_at,
        },
    ))
}

/// Finds an API token by token hash.
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
