//! The delivery of a secret value to an agent (FR-RP-05, AC-RP-03).
//!
//! A delivery without its `secret_delivered` audit record cannot commit (AC-RP-03).
//!
//! Only the job dispatcher may call [`load_ciphertext_for_delivery`], and only through the
//! `SecretReader` of the control plane. The caller MUST commit the transaction before it sends the
//! value to the agent. A rollback removes the record and the delivery does not happen; the agent
//! then does not receive the value.
//!
//! This crate does not decrypt. The control plane unwraps the data encryption key (DEK) and
//! decrypts the value outside it (ADR-0017, ADR-0023).

use uuid::Uuid;

use crate::DbError;
use crate::ids::SecretId;
use crate::tx::TenantTx;

/// The type holds the associated data inputs (`purpose`, `key_version`) and the ciphertext. It
/// holds no plaintext.
#[derive(Debug, Clone)]
pub struct CiphertextRow {
    /// Part of the associated data.
    pub purpose: String,
    pub key_version: i32,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

/// Reads the encrypted value of one secret and records the delivery (AC-RP-03).
///
/// `None` for a missing secret or a secret of another tenant; no record is written then (AC-TN-01).
/// For an existing secret the `secret_delivered` audit record is inserted in the transaction of the
/// caller before the ciphertext is returned. The caller MUST commit before it sends the value to the
/// agent: a rollback removes the record and the delivery does not happen.
pub async fn load_ciphertext_for_delivery(
    tx: &mut TenantTx<'_>,
    id: SecretId,
    agent_id: Uuid,
    job_id: Uuid,
    repository_id: Uuid,
) -> Result<Option<CiphertextRow>, DbError> {
    let row = read_row(tx, id).await?;
    if row.is_none() {
        return Ok(None);
    }
    crate::audit::insert_secret_delivered(tx, id, agent_id, job_id, repository_id).await?;
    Ok(row)
}

/// Reads the encrypted signing secret or URL part of a webhook channel (ADR-0036).
///
/// The control plane itself uses these values, and no agent receives them, so no
/// `secret_delivered` record is written. The query matches the purposes `webhook_secret` and
/// `webhook_url` only: no other secret can leave through this path.
pub async fn load_webhook_ciphertext(
    tx: &mut TenantTx<'_>,
    id: SecretId,
) -> Result<Option<CiphertextRow>, DbError> {
    let row = read_row(tx, id).await?;
    Ok(row.filter(|row| matches!(row.purpose.as_str(), "webhook_secret" | "webhook_url")))
}

/// Reads the encrypted value of one secret without an audit record.
///
/// The function is a test helper behind the feature `test-util`. The delivery path uses
/// [`load_ciphertext_for_delivery`], so a delivery always writes its record.
#[cfg(feature = "test-util")]
pub async fn load_ciphertext(
    tx: &mut TenantTx<'_>,
    id: SecretId,
) -> Result<Option<CiphertextRow>, DbError> {
    read_row(tx, id).await
}

async fn read_row(tx: &mut TenantTx<'_>, id: SecretId) -> Result<Option<CiphertextRow>, DbError> {
    let row = sqlx::query_as::<_, (String, i32, Vec<u8>, Vec<u8>)>(
        "SELECT purpose, key_version, nonce, ciphertext FROM secrets WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(
        row.map(|(purpose, key_version, nonce, ciphertext)| CiphertextRow {
            purpose,
            key_version,
            nonce,
            ciphertext,
        }),
    )
}
