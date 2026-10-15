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
use restichub_db::kek::{TenantKey, active_key, key_by_version};
use restichub_db::queue::StoredSignedJob;
use restichub_db::{DbError, JobId, SecretId, TenantTx};
use restichub_secrets::{
    CryptoError, EncryptedSecret, KekError, KekId, KekProvider, NONCE_LEN, SecretPurpose,
    SecretString, WrappedDek, decrypt_job_blob, decrypt_secret, encrypt_job_blob, unwrap_dek,
};
use zeroize::Zeroizing;

/// Holds no plaintext and no key.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Kek(#[from] KekError),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("the secret does not exist")]
    NotFound,
    #[error("the tenant holds no key row with the version of the secret")]
    KeyMissing,
    #[error("the stored purpose of the secret is not known")]
    BadPurpose,
    #[error("the stored secret material is not valid")]
    BadMaterial,
}

/// Reads a secret value for delivery to an agent (FR-RP-05, AC-RP-03).
///
/// Only the job dispatcher may create this type. Each call writes the `secret_delivered` audit
/// record in the transaction of the caller. The caller MUST commit before it sends the value to the
/// agent.
pub struct SecretReader<'a> {
    provider: &'a dyn KekProvider,
}

impl<'a> SecretReader<'a> {
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
        let row = load_ciphertext_for_delivery(tx, secret_id, agent_id, job_id, repository_id)
            .await?
            .ok_or(DeliveryError::NotFound)?;
        self.decrypt_row(tx, secret_id, row).await
    }

    /// Reads the value of a secret that a profile hands to a script and records the delivery
    /// (FR-HK-03, ADR-0017).
    ///
    /// Only a secret of the purpose `hook_secret` passes: a profile cannot make the agent receive
    /// a repository password through a script.
    pub async fn read_hook_secret_for_delivery(
        &self,
        tx: &mut TenantTx<'_>,
        secret_id: SecretId,
        agent_id: Uuid,
        job_id: Uuid,
        repository_id: Uuid,
    ) -> Result<SecretString, DeliveryError> {
        let metadata = restichub_db::secrets::get(tx, secret_id)
            .await?
            .ok_or(DeliveryError::NotFound)?;
        if metadata.purpose != SecretPurpose::HookSecret.as_str() {
            return Err(DeliveryError::BadPurpose);
        }
        self.read_for_delivery(tx, secret_id, agent_id, job_id, repository_id)
            .await
    }

    /// Reads the value of a secret that the control plane itself uses, such as the signing key of
    /// a webhook channel (ADR-0036). The call writes no `secret_delivered` record, because no
    /// value goes to an agent.
    pub async fn read_for_control_plane(
        &self,
        tx: &mut TenantTx<'_>,
        secret_id: SecretId,
    ) -> Result<SecretString, DeliveryError> {
        let row = restichub_db::delivery::load_webhook_ciphertext(tx, secret_id)
            .await?
            .ok_or(DeliveryError::NotFound)?;
        self.decrypt_row(tx, secret_id, row).await
    }

    async fn decrypt_row(
        &self,
        tx: &mut TenantTx<'_>,
        secret_id: SecretId,
        row: restichub_db::delivery::CiphertextRow,
    ) -> Result<SecretString, DeliveryError> {
        let tenant = tx.tenant();
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

/// The ciphertext of a signed job, with the version of the tenant key that encrypted it.
pub struct SealedJob {
    pub key_version: i32,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

/// Encrypts the signed bytes of a job with the active tenant key, as a secret value (ADR-0017).
pub async fn seal_signed_job(
    tx: &mut TenantTx<'_>,
    provider: &dyn KekProvider,
    job_id: JobId,
    bytes: &[u8],
) -> Result<SealedJob, DeliveryError> {
    let tenant = tx.tenant();
    let key = active_key(tx).await?.ok_or(DeliveryError::KeyMissing)?;
    let dek = unwrap_dek(
        provider,
        tenant.as_uuid(),
        key.key_version,
        &wrapped_dek(&key)?,
    )?;
    let encrypted = encrypt_job_blob(&dek, tenant.as_uuid(), job_id.as_uuid(), bytes)?;
    Ok(SealedJob {
        key_version: key.key_version,
        nonce: encrypted.nonce.to_vec(),
        ciphertext: encrypted.ciphertext,
    })
}

/// Decrypts the signed bytes that [`seal_signed_job`] stored. The caller sends the bytes as they
/// are, so a redelivery never builds or signs the job again (ADR-0020).
pub async fn open_signed_job(
    tx: &mut TenantTx<'_>,
    provider: &dyn KekProvider,
    job_id: JobId,
    stored: &StoredSignedJob,
) -> Result<Zeroizing<Vec<u8>>, DeliveryError> {
    let tenant = tx.tenant();
    let key = key_by_version(tx, stored.key_version)
        .await?
        .ok_or(DeliveryError::KeyMissing)?;
    let dek = unwrap_dek(
        provider,
        tenant.as_uuid(),
        key.key_version,
        &wrapped_dek(&key)?,
    )?;
    // The error of the length check holds no cause: the stored nonce has a wrong length.
    let Ok(nonce) = <[u8; NONCE_LEN]>::try_from(stored.nonce.as_slice()) else {
        return Err(DeliveryError::BadMaterial);
    };
    let encrypted = EncryptedSecret {
        nonce,
        ciphertext: stored.ciphertext.clone(),
    };
    Ok(decrypt_job_blob(
        &dek,
        tenant.as_uuid(),
        job_id.as_uuid(),
        &encrypted,
    )?)
}

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
