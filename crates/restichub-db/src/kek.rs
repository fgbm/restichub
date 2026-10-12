//! The `kek_check` row (ADR-0023, section Secrets; issue #20).
//!
//! The platform tenant holds the `kek_check` rows. The control plane creates one row on the first
//! start and verifies it on every later start, so it refuses to start with an incorrect key
//! encryption key (AC-RP-02). The table has the general tenant policy, so the functions run in a
//! tenant transaction with [`crate::PLATFORM_TENANT_ID`].

use crate::DbError;
use crate::ids::TenantId;
use crate::tx::TenantTx;

/// The stored `kek_check` row of the tenant of the transaction.
#[derive(Debug, Clone)]
pub struct KekCheckRow {
    /// The key ID of the key encryption key that created the check.
    pub kek_id: String,
    /// The 96-bit nonce of the encryption.
    pub nonce: Vec<u8>,
    /// The authenticated ciphertext of the fixed plaintext.
    pub ciphertext: Vec<u8>,
}

/// Reads the `kek_check` row of the tenant of the transaction.
///
/// The function returns `None` on the first start, when the platform tenant holds no check row.
pub async fn read_check(tx: &mut TenantTx<'_>) -> Result<Option<KekCheckRow>, DbError> {
    let row = sqlx::query_as::<_, (String, Vec<u8>, Vec<u8>)>(
        "SELECT kek_id, nonce, ciphertext FROM kek_check ORDER BY created_at, kek_id LIMIT 1",
    )
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(|(kek_id, nonce, ciphertext)| KekCheckRow {
        kek_id,
        nonce,
        ciphertext,
    }))
}

/// Inserts a `kek_check` row in the tenant of the transaction.
pub async fn insert_check(
    tx: &mut TenantTx<'_>,
    kek_id: &str,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO kek_check (tenant_id, kek_id, nonce, ciphertext) VALUES ($1, $2, $3, $4)",
    )
    .bind(tx.tenant())
    .bind(kek_id)
    .bind(nonce)
    .bind(ciphertext)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

/// The active key row of one tenant.
#[derive(Debug, Clone)]
pub struct TenantKey {
    /// The version of the key.
    pub key_version: i32,
    /// The key ID of the key encryption key that wrapped the data encryption key.
    pub kek_id: String,
    /// The wrapped data encryption key.
    pub wrapped_dek: Vec<u8>,
    /// The 96-bit nonce of the wrapping.
    pub nonce: Vec<u8>,
}

/// Reads the active key row of a tenant in a tenant transaction.
pub async fn read_tenant_key(
    tx: &mut TenantTx<'_>,
    tenant: TenantId,
) -> Result<Option<TenantKey>, DbError> {
    let row = sqlx::query_as::<_, (i32, String, Vec<u8>, Vec<u8>)>(
        "SELECT key_version, kek_id, wrapped_dek, nonce \
         FROM tenant_keys WHERE tenant_id = $1 AND active",
    )
    .bind(tenant)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(
        row.map(|(key_version, kek_id, wrapped_dek, nonce)| TenantKey {
            key_version,
            kek_id,
            wrapped_dek,
            nonce,
        }),
    )
}
