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

#[derive(Debug, Clone)]
pub struct SessionLookup {
    pub tenant_id: TenantId,
    pub session_id: SessionId,
    pub user_id: UserId,
    pub expires_at: OffsetDateTime,
    pub last_seen_at: OffsetDateTime,
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
