//! The job signing key rows of the platform tenant (ADR-0026, ADR-0005).
//!
//! The platform tenant holds the current key and the next key. The key encryption key (KEK) wraps
//! each private key; the module returns the wrapped bytes only, never a plaintext key. The control
//! plane unwraps a key on demand (ADR-0026, section CA key format).
//!
//! A rotation schedules the retirement of the old current key, promotes the next key and inserts a
//! new next key. A retired row is outside the active set; its identifier is a revoked key.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::enums::SigningKeyRole;
use crate::tx::TenantTx;

/// The type holds the wrapped private key, never the plaintext key.
#[derive(Debug, Clone)]
pub struct SigningKeyRow {
    pub id: Uuid,
    pub role: SigningKeyRole,
    pub kek_id: String,
    pub nonce: Vec<u8>,
    pub wrapped_key: Vec<u8>,
    pub created_at: OffsetDateTime,
    /// The old current key retires when the agents acknowledge the new set or this time passes.
    pub retire_after: Option<OffsetDateTime>,
    /// Set when the key is retired; its identifier is a revoked key.
    pub retired_at: Option<OffsetDateTime>,
}

/// The caller wraps the private key and passes the ciphertext with the key encryption key (KEK)
/// that wrapped it. The identifier is generated before the wrapping, because it names the row.
#[derive(Debug, Clone)]
pub struct NewSigningKey<'a> {
    pub id: Uuid,
    pub role: SigningKeyRole,
    pub kek_id: &'a str,
    pub nonce: &'a [u8],
    pub wrapped_key: &'a [u8],
}

/// The rows of the tenant of the transaction, oldest first.
pub async fn list(tx: &mut TenantTx<'_>) -> Result<Vec<SigningKeyRow>, DbError> {
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            SigningKeyRole,
            String,
            Vec<u8>,
            Vec<u8>,
            OffsetDateTime,
            Option<OffsetDateTime>,
            Option<OffsetDateTime>,
        ),
    >(
        "SELECT id, role, kek_id, nonce, wrapped_key, created_at, retire_after, retired_at \
         FROM signing_keys ORDER BY created_at, id",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(
            |(id, role, kek_id, nonce, wrapped_key, created_at, retire_after, retired_at)| {
                SigningKeyRow {
                    id,
                    role,
                    kek_id,
                    nonce,
                    wrapped_key,
                    created_at,
                    retire_after,
                    retired_at,
                }
            },
        )
        .collect())
}

pub async fn insert(tx: &mut TenantTx<'_>, new: &NewSigningKey<'_>) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO signing_keys (tenant_id, id, role, kek_id, nonce, wrapped_key) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(tx.tenant())
    .bind(new.id)
    .bind(new.role)
    .bind(new.kek_id)
    .bind(new.nonce)
    .bind(new.wrapped_key)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

/// Promotes a key to a new role and stores the key material wrapped under that role.
///
/// The associated data of the wrapped key binds the role (ADR-0026, section CA key format), so a
/// promotion MUST pass the material re-wrapped under the new role. The key identifier stays.
pub async fn set_role_and_wrapped(
    tx: &mut TenantTx<'_>,
    id: Uuid,
    role: SigningKeyRole,
    kek_id: &str,
    nonce: &[u8],
    wrapped_key: &[u8],
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE signing_keys SET role = $1, kek_id = $2, nonce = $3, wrapped_key = $4 WHERE id = $5",
    )
    .bind(role)
    .bind(kek_id)
    .bind(nonce)
    .bind(wrapped_key)
    .bind(id)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

/// Marks the old current key for retirement when the agents acknowledge the new set or `retire_after`
/// passes (ADR-0026).
pub async fn schedule_retire(
    tx: &mut TenantTx<'_>,
    id: Uuid,
    retire_after: OffsetDateTime,
) -> Result<(), DbError> {
    sqlx::query("UPDATE signing_keys SET retire_after = $1 WHERE id = $2")
        .bind(retire_after)
        .bind(id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(())
}

/// Retires the pending keys whose `retire_after` is not after `now`. Returns the number of rows.
pub async fn retire_due(tx: &mut TenantTx<'_>, now: OffsetDateTime) -> Result<u64, DbError> {
    let result = sqlx::query(
        "UPDATE signing_keys SET retired_at = $1 \
         WHERE retired_at IS NULL AND retire_after IS NOT NULL AND retire_after <= $1",
    )
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}

/// Retires every pending key, because all known online agents acknowledged the new set (ADR-0026).
pub async fn retire_all_pending(
    tx: &mut TenantTx<'_>,
    now: OffsetDateTime,
) -> Result<u64, DbError> {
    let result = sqlx::query(
        "UPDATE signing_keys SET retired_at = $1 \
         WHERE retired_at IS NULL AND retire_after IS NOT NULL",
    )
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}
