//! The `kek_check` row (ADR-0023, section Secrets).
//!
//! The platform tenant holds the `kek_check` rows. The control plane creates one row on the first
//! start and verifies it on every later start, so it refuses to start with an incorrect key
//! encryption key (AC-RP-02). The table has the general tenant policy, so the functions run in a
//! tenant transaction with [`crate::PLATFORM_TENANT_ID`].

use crate::DbError;
use crate::ids::TenantId;
use crate::tx::TenantTx;

#[derive(Debug, Clone)]
pub struct KekCheckRow {
    pub kek_id: String,
    /// 96-bit nonce of the encryption.
    pub nonce: Vec<u8>,
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

#[derive(Debug, Clone)]
pub struct TenantKey {
    pub key_version: i32,
    pub kek_id: String,
    pub wrapped_dek: Vec<u8>,
    /// 96-bit nonce of the wrapping.
    pub nonce: Vec<u8>,
}

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

/// Reads the active key row of the tenant of the transaction.
///
/// The function returns the wrapped data encryption key (DEK). The control plane unwraps it with
/// the key encryption key (KEK) before it encrypts or decrypts a secret value (ADR-0017). A tenant
/// has one active key version, so the function returns at most one row.
pub async fn active_key(tx: &mut TenantTx<'_>) -> Result<Option<TenantKey>, DbError> {
    read_tenant_key(tx, tx.tenant()).await
}

/// Reads the key row of the tenant of the transaction with a given version.
///
/// The function returns the row also when the version is not active. The delivery path uses it: a
/// secret holds the key version that encrypted it, so a rotated tenant must still deliver a secret
/// that an older version encrypted (ADR-0017).
pub async fn key_by_version(
    tx: &mut TenantTx<'_>,
    version: i32,
) -> Result<Option<TenantKey>, DbError> {
    let row = sqlx::query_as::<_, (i32, String, Vec<u8>, Vec<u8>)>(
        "SELECT key_version, kek_id, wrapped_dek, nonce \
         FROM tenant_keys WHERE tenant_id = $1 AND key_version = $2",
    )
    .bind(tx.tenant())
    .bind(version)
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
