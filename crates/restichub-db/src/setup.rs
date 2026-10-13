//! One-time setup token operations (FR-AU-03).
//!
//! The server generates a setup token at start when the platform tenant has no user. The database
//! keeps only the SHA-256 hash. The token belongs to the platform tenant and uses a tenant
//! transaction with `PLATFORM_TENANT_ID`. The table has the general tenant policy.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::ids::SetupTokenId;
use crate::tx::TenantTx;

pub async fn create(
    tx: &mut TenantTx<'_>,
    token_hash: &[u8],
    expires_at: OffsetDateTime,
) -> Result<SetupTokenId, DbError> {
    let id = SetupTokenId::new();
    sqlx::query(
        "INSERT INTO setup_tokens (id, tenant_id, token_hash, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(tx.tenant())
    .bind(token_hash)
    .bind(expires_at)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(id)
}

/// Finds an unused, unexpired setup token in the tenant of the transaction.
///
/// The function locks the row (`FOR UPDATE`), so two setup requests with the same token cannot both
/// read it as unused. The caller also holds the tenant lock ([`TenantTx::lock`]).
pub async fn find_valid(
    tx: &mut TenantTx<'_>,
    token_hash: &[u8],
    now: OffsetDateTime,
) -> Result<Option<SetupTokenId>, DbError> {
    let row = sqlx::query_as::<_, (Uuid,)>(
        "SELECT id FROM setup_tokens \
         WHERE token_hash = $1 AND used_at IS NULL AND expires_at > $2 \
         FOR UPDATE",
    )
    .bind(token_hash)
    .bind(now)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(|(id,)| SetupTokenId::from_uuid(id)))
}

pub async fn mark_used(
    tx: &mut TenantTx<'_>,
    id: SetupTokenId,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result =
        sqlx::query("UPDATE setup_tokens SET used_at = $2 WHERE id = $1 AND used_at IS NULL")
            .bind(id)
            .bind(now)
            .execute(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Marks every unused setup token of the tenant of the transaction as used.
///
/// The caller runs this in the transaction of a successful setup, so a token that the control
/// plane issued earlier cannot be used after the setup is done (ADR-0023).
pub async fn mark_all_used(tx: &mut TenantTx<'_>, now: OffsetDateTime) -> Result<u64, DbError> {
    let result = sqlx::query("UPDATE setup_tokens SET used_at = $1 WHERE used_at IS NULL")
        .bind(now)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}

pub async fn user_count(tx: &mut TenantTx<'_>) -> Result<i64, DbError> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM memberships")
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(count)
}

/// Removes the expired and the used setup tokens of the tenant of the transaction.
///
/// The control plane maintenance task calls this function.
pub async fn delete_expired_or_used(
    tx: &mut TenantTx<'_>,
    now: OffsetDateTime,
) -> Result<u64, DbError> {
    let result =
        sqlx::query("DELETE FROM setup_tokens WHERE expires_at < $1 OR used_at IS NOT NULL")
            .bind(now)
            .execute(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}
