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

/// The number of seconds below which the `last_used_at` update is skipped.
pub const TOUCH_INTERVAL_SECONDS: i64 = 60;

/// The data of a new API token.
#[derive(Debug, Clone)]
pub struct NewApiToken<'a> {
    /// The name for the user interface.
    pub name: &'a str,
    /// The SHA-256 hash of the token value.
    pub token_hash: &'a [u8],
    /// The first characters of the token value, for display.
    pub display_prefix: &'a str,
    /// The role of the token in the tenant.
    pub role: Role,
    /// The user that created the token.
    pub created_by: UserId,
    /// The optional expiry time.
    pub expires_at: Option<OffsetDateTime>,
}

/// An API token row.
#[derive(Debug, Clone)]
pub struct ApiToken {
    /// The identifier of the token.
    pub id: ApiTokenId,
    /// The name for the user interface.
    pub name: String,
    /// The role of the token.
    pub role: Role,
    /// The first characters of the token value.
    pub display_prefix: String,
    /// The user that created the token.
    pub created_by: UserId,
    /// The creation time.
    pub created_at: OffsetDateTime,
    /// The optional expiry time.
    pub expires_at: Option<OffsetDateTime>,
    /// The time of the last use.
    pub last_used_at: Option<OffsetDateTime>,
    /// The revocation time. A revoked token is refused.
    pub revoked_at: Option<OffsetDateTime>,
}

/// Creates an API token in the tenant of the transaction and returns its identifier.
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

/// Lists the API tokens of the tenant of the transaction.
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

/// Reads one API token of the tenant of the transaction.
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
/// The function returns true if it changed the token. A second call on the same token returns
/// false and changes nothing (idempotent).
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

/// Updates `last_used_at` when the stored value is older than [`TOUCH_INTERVAL_SECONDS`].
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

/// The column tuple of one API token row.
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
