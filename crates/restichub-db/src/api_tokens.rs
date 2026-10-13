//! API token operations (ADR-0023, section API tokens).
//!
//! The database keeps only the SHA-256 hash of a token and a short display prefix. The table has
//! the general tenant policy. The control plane looks a token up with the `SECURITY DEFINER`
//! function `auth_find_api_token` before it knows the tenant ([`crate::auth::find_api_token`]).
//!
//! An API token is owned by the tenant, not by the user who created it. The token stays valid
//! while the tenant is active, also when the creator is disabled.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::enums::Role;
use crate::ids::{ApiTokenId, UserId};
use crate::tx::TenantTx;

/// Skips the `last_used_at` update below this many seconds.
pub const TOUCH_INTERVAL_SECONDS: i64 = 60;

#[derive(Debug, Clone)]
pub struct NewApiToken<'a> {
    pub name: &'a str,
    /// SHA-256 of the token value.
    pub token_hash: &'a [u8],
    pub display_prefix: &'a str,
    pub role: Role,
    pub created_by: UserId,
    pub expires_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone)]
pub struct ApiToken {
    pub id: ApiTokenId,
    pub name: String,
    pub role: Role,
    pub display_prefix: String,
    pub created_by: UserId,
    pub created_at: OffsetDateTime,
    pub expires_at: Option<OffsetDateTime>,
    pub last_used_at: Option<OffsetDateTime>,
    /// Revoked tokens are refused.
    pub revoked_at: Option<OffsetDateTime>,
}

pub async fn create(tx: &mut TenantTx<'_>, new: &NewApiToken<'_>) -> Result<ApiTokenId, DbError> {
    let id = ApiTokenId::new();
    sqlx::query(
        "INSERT INTO api_tokens \
         (id, tenant_id, name, token_hash, display_prefix, role, created_by, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(tx.tenant())
    .bind(new.name)
    .bind(new.token_hash)
    .bind(new.display_prefix)
    .bind(new.role.as_str())
    .bind(new.created_by)
    .bind(new.expires_at)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(id)
}

pub async fn list(tx: &mut TenantTx<'_>) -> Result<Vec<ApiToken>, DbError> {
    let rows = sqlx::query_as::<_, ApiTokenRow>(
        "SELECT id, name, role, display_prefix, created_by, created_at, expires_at, last_used_at, \
                revoked_at \
         FROM api_tokens ORDER BY created_at DESC, id DESC",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(ApiToken::from).collect())
}

pub async fn get(tx: &mut TenantTx<'_>, id: ApiTokenId) -> Result<Option<ApiToken>, DbError> {
    let row = sqlx::query_as::<_, ApiTokenRow>(
        "SELECT id, name, role, display_prefix, created_by, created_at, expires_at, last_used_at, \
                revoked_at \
         FROM api_tokens WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(ApiToken::from))
}

/// Revokes an API token of the tenant of the transaction.
///
/// `true` when the token changed. A second call on the same token returns `false` and changes
/// nothing (idempotent).
pub async fn revoke(
    tx: &mut TenantTx<'_>,
    id: ApiTokenId,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result =
        sqlx::query("UPDATE api_tokens SET revoked_at = $2 WHERE id = $1 AND revoked_at IS NULL")
            .bind(id)
            .bind(now)
            .execute(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

pub async fn touch(
    tx: &mut TenantTx<'_>,
    id: ApiTokenId,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE api_tokens SET last_used_at = $2 \
         WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < $2 - make_interval(secs => $3))",
    )
    .bind(id)
    .bind(now)
    .bind(TOUCH_INTERVAL_SECONDS)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

type ApiTokenRow = (
    Uuid,
    String,
    Role,
    String,
    Uuid,
    OffsetDateTime,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
);

impl From<ApiTokenRow> for ApiToken {
    fn from(row: ApiTokenRow) -> Self {
        let (
            id,
            name,
            role,
            display_prefix,
            created_by,
            created_at,
            expires_at,
            last_used_at,
            revoked_at,
        ) = row;
        Self {
            id: ApiTokenId::from_uuid(id),
            name,
            role,
            display_prefix,
            created_by: UserId::from_uuid(created_by),
            created_at,
            expires_at,
            last_used_at,
            revoked_at,
        }
    }
}
