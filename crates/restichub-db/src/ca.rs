//! The certificate authority rows of the platform tenant (ADR-0026).
//!
//! The platform tenant holds one root CA and one or more issuing CAs. The key encryption key (KEK)
//! wraps each private key; the certificates are stored in clear. The module returns the wrapped
//! bytes only, never a plaintext key. The control plane unwraps a key on demand (ADR-0026,
//! section CA key format).
//!
//! A row belongs to one tenant. The general tenant policy of the `ca_keys` table shows the rows of
//! the tenant of the transaction only (ADR-0023, section Tenant isolation).

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::tx::TenantTx;

/// The type holds the wrapped private key, never the plaintext key.
#[derive(Debug, Clone)]
pub struct CaKeyRow {
    pub id: Uuid,
    /// Allowed values are owned by the `restichub-secrets` crate (`root` or `issuing`).
    pub role: String,
    pub kek_id: String,
    pub nonce: Vec<u8>,
    pub wrapped_key: Vec<u8>,
    /// Public certificate, DER encoded.
    pub cert_der: Vec<u8>,
    pub not_before: OffsetDateTime,
    pub not_after: OffsetDateTime,
}

/// The caller wraps the private key and passes the ciphertext with the key encryption key (KEK)
/// that wrapped it. The identifier is generated before the wrapping, because it names the row.
#[derive(Debug, Clone)]
pub struct NewCaKey<'a> {
    pub id: Uuid,
    pub role: &'a str,
    pub kek_id: &'a str,
    pub nonce: &'a [u8],
    pub wrapped_key: &'a [u8],
    pub cert_der: &'a [u8],
    pub not_before: OffsetDateTime,
    pub not_after: OffsetDateTime,
}

/// The rows of the tenant of the transaction, the root CA first, then the issuing CAs by start
/// time. A rotation leaves more than one issuing row.
pub async fn list(tx: &mut TenantTx<'_>) -> Result<Vec<CaKeyRow>, DbError> {
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Vec<u8>,
            Vec<u8>,
            Vec<u8>,
            OffsetDateTime,
            OffsetDateTime,
        ),
    >(
        "SELECT id, role, kek_id, nonce, wrapped_key, cert_der, not_before, not_after \
         FROM ca_keys ORDER BY role DESC, not_before",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(
            |(id, role, kek_id, nonce, wrapped_key, cert_der, not_before, not_after)| CaKeyRow {
                id,
                role,
                kek_id,
                nonce,
                wrapped_key,
                cert_der,
                not_before,
                not_after,
            },
        )
        .collect())
}

pub async fn insert(tx: &mut TenantTx<'_>, new: &NewCaKey<'_>) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO ca_keys \
         (tenant_id, id, role, kek_id, nonce, wrapped_key, cert_der, not_before, not_after) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(tx.tenant())
    .bind(new.id)
    .bind(new.role)
    .bind(new.kek_id)
    .bind(new.nonce)
    .bind(new.wrapped_key)
    .bind(new.cert_der)
    .bind(new.not_before)
    .bind(new.not_after)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}
