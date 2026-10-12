//! The delivery of a secret value to an agent (FR-RP-05, AC-RP-03).
//!
//! The job dispatcher of a later epic reads a secret value for one job with [`SecretReader`]. The
//! reader loads the ciphertext through
//! [`restichub_db::delivery::load_ciphertext_for_delivery`], which writes the `secret_delivered`
//! audit record in the same transaction, then unwraps the data encryption key (DEK) of the tenant
//! and decrypts the value.
//!
//! The caller MUST commit the transaction before it sends the value to the agent. A rollback
//! removes the audit record, and the delivery does not happen: the agent then does not receive the
//! value. A delivery and its record therefore commit together or not at all (AC-RP-03).
//!
//! The plaintext lives in a [`SecretString`] only and is zeroized on drop.

use uuid::Uuid;

use restichub_db::delivery::load_ciphertext_for_delivery;
use restichub_db::kek::{TenantKey, key_by_version};
use restichub_db::{DbError, SecretId, TenantTx};
use restichub_secrets::{
    CryptoError, EncryptedSecret, KekError, KekId, KekProvider, NONCE_LEN, SecretPurpose,
    SecretString, WrappedDek, decrypt_secret, unwrap_dek,
};

/// The error of a secret delivery.
///
/// The type holds no plaintext and no key.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    /// A database operation failed.
    #[error(transparent)]
    Database(#[from] DbError),
    /// The key encryption key (KEK) operation failed.
    #[error(transparent)]
    Kek(#[from] KekError),
    /// The authenticated decryption failed.
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    /// The secret does not exist for the tenant of the transaction.
    #[error("the secret does not exist")]
    NotFound,
    /// The tenant holds no key row with the version of the secret.
    #[error("the tenant holds no key row with the version of the secret")]
    KeyMissing,
    /// The stored purpose is not one of the known values.
    #[error("the stored purpose of the secret is not known")]
    BadPurpose,
    /// The stored key row or the stored ciphertext is not valid.
    #[error("the stored secret material is not valid")]
    BadMaterial,
}

/// Reads a secret value for delivery to an agent (FR-RP-05, AC-RP-03).
///
/// Only the job dispatcher may create this type. Each call writes the `secret_delivered` audit
/// record in the transaction of the caller. The caller MUST commit before it sends the value to the
/// agent.
pub struct SecretReader<'a> {
    /// The key encryption key (KEK) provider that unwraps the data encryption key (DEK).
    provider: &'a dyn KekProvider,
}

impl<'a> SecretReader<'a> {
    /// Creates a reader with a key encryption key (KEK) provider.
    pub fn new(provider: &'a dyn KekProvider) -> Self {
        Self { provider }
    }

    /// Reads the value of one secret and records the delivery.
    ///
    /// The function loads the ciphertext and the `secret_delivered` audit record, unwraps the DEK
    /// of the version that encrypted the secret and decrypts the value. The record holds the
    /// tenant, the agent, the job, the repository and the time; it holds no secret value. The
    /// caller MUST commit the transaction before it sends the value to the agent.
    pub async fn read_for_delivery(
        &self,
        tx: &mut TenantTx<'_>,
        secret_id: SecretId,
        agent_id: Uuid,
        job_id: Uuid,
        repository_id: Uuid,
    ) -> Result<SecretString, DeliveryError> {
        let tenant = tx.tenant();
        let row = load_ciphertext_for_delivery(tx, secret_id, agent_id, job_id, repository_id)
            .await?
            .ok_or(DeliveryError::NotFound)?;
        // The key row of the version that encrypted the secret. The version can be not active,
        // because a rotation keeps the older keys (ADR-0017).
        let key = key_by_version(tx, row.key_version)
            .await?
            .ok_or(DeliveryError::KeyMissing)?;
        let purpose = SecretPurpose::parse(&row.purpose).ok_or(DeliveryError::BadPurpose)?;
        let wrapped = wrapped_dek(&key)?;
        let dek = unwrap_dek(self.provider, tenant.as_uuid(), key.key_version, &wrapped)?;
        let encrypted = EncryptedSecret {
            nonce: row
                .nonce
                .as_slice()
                .try_into()
                .map_err(|_| DeliveryError::BadMaterial)?,
            ciphertext: row.ciphertext,
        };
        let plaintext = decrypt_secret(
            &dek,
            tenant.as_uuid(),
            secret_id.as_uuid(),
            purpose,
            &encrypted,
        )?;
        Ok(plaintext)
    }
}

/// Builds the wrapped data encryption key (DEK) from the stored key row.
fn wrapped_dek(key: &TenantKey) -> Result<WrappedDek, DeliveryError> {
    let kek_id = KekId::parse(&key.kek_id)?;
    let nonce: [u8; NONCE_LEN] = key
        .nonce
        .as_slice()
        .try_into()
        .map_err(|_| DeliveryError::BadMaterial)?;
    Ok(WrappedDek {
        kek_id,
        nonce,
        ciphertext: key.wrapped_dek.clone(),
    })
}
