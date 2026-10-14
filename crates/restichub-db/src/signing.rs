//! The job signing key rows of the platform tenant (ADR-0026, ADR-0005).
//!
//! The platform tenant holds the current key and the next key. The key encryption key (KEK) wraps
//! each private key; the module returns the wrapped bytes only, never a plaintext key. The control
//! plane unwraps a key on demand (ADR-0026, section CA key format).
//!
//! A rotation schedules the retirement of the old current key, promotes the next key and inserts a
//! new next key. A retired row is outside the active set; its identifier is a revoked key. A
//! retired row is kept, because it is the data side of the re-sync of an agent more than one
//! rotation behind (ADR-0026).

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
    /// Binds the unwrapped key to the row: the load compares it with the unwrapped key, so a
    /// swapped blob fails (T19).
    pub public_key: Vec<u8>,
    pub created_at: OffsetDateTime,
    /// The old current key retires when the agents acknowledge the new set or this time passes.
    pub retire_after: Option<OffsetDateTime>,
    /// Set when the key is retired; its identifier is a revoked key.
    pub retired_at: Option<OffsetDateTime>,
    /// The signed `SigningKeyUpdate` envelope of the step that introduced this key, if any.
    pub step_update: Option<Vec<u8>>,
    /// `rotation` or `compromise` when the key is retired; the chain never carries a compromise.
    pub retired_reason: Option<String>,
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
    pub public_key: &'a [u8],
    /// The signed envelope of the step that introduces the key; the first key holds none.
    pub step_update: Option<&'a [u8]>,
}

/// A cheap change token of the table: the row count, the retired row count and the newest row time.
///
/// A rotation inserts a next key and a retirement sets `retired_at`, so the token changes. A caller
/// that holds a loaded key set compares the token before it signs and reloads on a change
/// (ADR-0026).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub rows: i64,
    pub retired: i64,
    pub newest: Option<OffsetDateTime>,
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
            Vec<u8>,
            OffsetDateTime,
            Option<OffsetDateTime>,
            Option<OffsetDateTime>,
            Option<Vec<u8>>,
            Option<String>,
        ),
    >(
        "SELECT id, role, kek_id, nonce, wrapped_key, public_key, created_at, retire_after, \
         retired_at, step_update, retired_reason FROM signing_keys ORDER BY created_at, id",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(
            |(
                id,
                role,
                kek_id,
                nonce,
                wrapped_key,
                public_key,
                created_at,
                retire_after,
                retired_at,
                step_update,
                retired_reason,
            )| {
                SigningKeyRow {
                    id,
                    role,
                    kek_id,
                    nonce,
                    wrapped_key,
                    public_key,
                    created_at,
                    retire_after,
                    retired_at,
                    step_update,
                    retired_reason,
                }
            },
        )
        .collect())
}

/// The change token of the table, for a cheap freshness check before a signature (ADR-0026).
pub async fn version(tx: &mut TenantTx<'_>) -> Result<Version, DbError> {
    let (rows, retired, newest): (i64, i64, Option<OffsetDateTime>) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE retired_at IS NOT NULL), max(created_at) \
         FROM signing_keys",
    )
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(Version {
        rows,
        retired,
        newest,
    })
}

pub async fn insert(tx: &mut TenantTx<'_>, new: &NewSigningKey<'_>) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO signing_keys (tenant_id, id, role, kek_id, nonce, wrapped_key, public_key, \
         step_update) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(tx.tenant())
    .bind(new.id)
    .bind(new.role)
    .bind(new.kek_id)
    .bind(new.nonce)
    .bind(new.wrapped_key)
    .bind(new.public_key)
    .bind(new.step_update)
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

/// Retires the pending keys whose `retire_after` is not after `now`. Returns the retired rows.
pub async fn retire_due(tx: &mut TenantTx<'_>, now: OffsetDateTime) -> Result<Vec<Uuid>, DbError> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "UPDATE signing_keys SET retired_at = $1, retired_reason = 'rotation' \
         WHERE retired_at IS NULL AND retire_after IS NOT NULL AND retire_after <= $1 \
         RETURNING id",
    )
    .bind(now)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Retires the named pending key after all known online agents acknowledged the new set (ADR-0026).
/// Returns the retired row, or none when the key is not pending.
pub async fn retire_pending(
    tx: &mut TenantTx<'_>,
    id: Uuid,
    now: OffsetDateTime,
) -> Result<Vec<Uuid>, DbError> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "UPDATE signing_keys SET retired_at = $1, retired_reason = 'rotation' \
         WHERE id = $2 AND retired_at IS NULL AND retire_after IS NOT NULL \
         RETURNING id",
    )
    .bind(now)
    .bind(id)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Retires a key for a compromise. Returns the retired row, or none when the key is already retired.
///
/// A key that the control plane retired for a compromise is never part of the chain: the key must
/// not sign a later update, because an attacker who holds it would otherwise be trusted (ADR-0030).
pub async fn retire_compromised(
    tx: &mut TenantTx<'_>,
    id: Uuid,
    now: OffsetDateTime,
) -> Result<Vec<Uuid>, DbError> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "UPDATE signing_keys SET retired_at = $1, retired_reason = 'compromise' \
         WHERE id = $2 AND retired_at IS NULL \
         RETURNING id",
    )
    .bind(now)
    .bind(id)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}
