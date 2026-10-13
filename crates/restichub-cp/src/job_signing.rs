//! The job signing key set of the installation (ADR-0026, ADR-0005, FR-PR-03, FR-PR-11).
//!
//! The control plane signs each job with an Ed25519 key. One key set serves all tenants and lives
//! in the platform tenant. The set holds the current key that signs the jobs and the next key that
//! the agents receive before a rotation. The key encryption key (KEK) wraps each private key, in
//! the manner of a CA key (ADR-0026, section Job signing keys).
//!
//! A rotation promotes the next key to current, generates a new next key and schedules the
//! retirement of the old current key. The old key retires when all known online agents acknowledge
//! the new set, or at most [`ROTATION_OVERLAP`] after the rotation starts. An agent that was
//! offline longer re-syncs the key set at its reconnect; the delivery is [`KeySet::key_update`].
//!
//! The plaintext key never leaves this module. The types hold no `Debug` that shows a key, and the
//! key set is never written to a log or to a process argument.

use std::sync::Arc;

use ed25519_dalek::{SigningKey, VerifyingKey};
use prost_types::Timestamp;
use restichub_db::audit::{self, AuditAction, AuditEvent};
use restichub_db::signing::{self, NewSigningKey, SigningKeyRow};
use restichub_db::{Database, DbError, PLATFORM_TENANT_ID, SigningKeyRole};
use restichub_proto::agent::v1::{Job, KeyUpdate, SignedJob, SigningKeyUpdate};
use restichub_proto::signing::{sign_job, sign_key_update};
use restichub_secrets::{
    CryptoError, KekError, KekId, KekProvider, WrappedSigningKey, unwrap_signing_key,
    wrap_signing_key,
};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use zeroize::Zeroizing;

/// The old current key retires at most this long after the rotation starts (ADR-0026).
pub const ROTATION_OVERLAP: Duration = Duration::days(7);

/// Ed25519 keys use a 32-byte seed.
const KEY_LEN: usize = 32;

/// The advisory lock scope that serializes the creation and the rotation of the key set.
const SIGNING_KEY_LOCK: &str = "restichub.signing_key_init:";

#[derive(Debug, thiserror::Error)]
pub enum JobSigningError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Kek(#[from] KekError),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error("the operating system random number generator failed")]
    Random(#[source] getrandom::Error),
    #[error("the installation has no job signing key set")]
    Missing,
    #[error("the installation holds no active {role:?} job signing key")]
    NoActiveRole { role: SigningKeyRole },
    #[error("the installation holds more than one active {role:?} job signing key")]
    MultipleActiveRole { role: SigningKeyRole },
    #[error("the stored job signing key {id} is not a valid Ed25519 key")]
    BadKey { id: Uuid },
    #[error("the stored nonce of the job signing key {id} is not 96 bits")]
    BadNonce {
        id: Uuid,
        #[source]
        source: std::array::TryFromSliceError,
    },
}

/// One job signing key with its identifier. Holds the private key, so it derives no `Debug`.
pub struct SigningKeyPair {
    id: String,
    key: SigningKey,
}

impl std::fmt::Debug for SigningKeyPair {
    /// Shows the identifier only; a private key never reaches a log.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SigningKeyPair")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl SigningKeyPair {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.key.verifying_key()
    }

    fn from_stored(id: Uuid, bytes: &[u8]) -> Result<Self, JobSigningError> {
        let bytes: [u8; KEY_LEN] = bytes
            .try_into()
            .map_err(|_| JobSigningError::BadKey { id })?;
        Ok(Self {
            id: id.to_string(),
            key: SigningKey::from_bytes(&bytes),
        })
    }
}

/// The current key, the next key and the identifiers of the retired keys (ADR-0026).
pub struct KeySet {
    current: SigningKeyPair,
    next: SigningKeyPair,
    revoked: Vec<String>,
}

impl std::fmt::Debug for KeySet {
    /// Shows the key identifiers only.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KeySet")
            .field("current", &self.current)
            .field("next", &self.next)
            .field("revoked", &self.revoked)
            .finish()
    }
}

impl KeySet {
    pub fn current_key_id(&self) -> &str {
        self.current.id()
    }

    pub fn next_key_id(&self) -> &str {
        self.next.id()
    }

    /// The identifiers of the keys that the agents must revoke (FR-PR-11).
    pub fn revoked_key_ids(&self) -> &[String] {
        &self.revoked
    }

    /// Makes the signature of a job with the current key (FR-PR-03). The job names its signer key.
    pub fn sign_job(&self, job: &Job) -> SignedJob {
        sign_job(job, self.current.id(), &self.current.key)
    }

    /// The signed key set that the control plane sends to an agent over the stream (FR-PR-11).
    ///
    /// The current key signs the next key. The message names the next key and the revoked keys, so
    /// an agent that reconnects re-syncs the set and learns the revocations. The delivery is
    /// idempotent on the agent: an existing key with the same identifier and bytes is a no-op.
    pub fn key_update(&self, now: OffsetDateTime) -> SigningKeyUpdate {
        let update = KeyUpdate {
            key_id: self.next.id().to_owned(),
            public_key: self.next.verifying_key().to_bytes().to_vec(),
            signer_key_id: self.current.id().to_owned(),
            issued_at: Some(timestamp(now)),
            revoked_key_ids: self.revoked.clone(),
        };
        sign_key_update(&update, &self.current.key)
    }
}

/// Owns the job signing key set of the installation.
#[derive(Debug, Clone)]
pub struct JobSigningService {
    provider: Arc<dyn KekProvider>,
}

impl JobSigningService {
    pub fn new(provider: Arc<dyn KekProvider>) -> Self {
        Self { provider }
    }

    /// Creates the key set on the first start and keeps it healthy on every later start.
    ///
    /// An advisory lock serializes two starts, so exactly one start creates the set. A later start
    /// loads the stored keys; an incorrect KEK fails the unwrap, so the control plane refuses to
    /// start (AC-PR-02).
    pub async fn ensure_initialized(&self, db: &Database) -> Result<KeySet, JobSigningError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        tx.lock(SIGNING_KEY_LOCK).await?;
        let rows = signing::list(&mut tx).await?;
        if rows.iter().all(|row| row.retired_at.is_some()) {
            self.generate_and_insert(&mut tx, SigningKeyRole::Current)
                .await?;
            self.generate_and_insert(&mut tx, SigningKeyRole::Next)
                .await?;
        }
        tx.commit().await?;
        self.load(db).await?.ok_or(JobSigningError::Missing)
    }

    /// Reads and validates the key set of the platform tenant. `None` when the set is absent.
    pub async fn load(&self, db: &Database) -> Result<Option<KeySet>, JobSigningError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        let rows = signing::list(&mut tx).await?;
        tx.commit().await?;
        if rows.iter().all(|row| row.retired_at.is_some()) {
            return Ok(None);
        }
        Ok(Some(self.build(&rows)?))
    }

    /// Rotates the key set (ADR-0026).
    ///
    /// The next key becomes the current key, a new next key is generated and the old current key is
    /// scheduled for retirement after [`ROTATION_OVERLAP`]. An advisory lock serializes two
    /// rotations. The new current key signs the jobs at once; the agents already hold it from the
    /// earlier delivery.
    pub async fn rotate(
        &self,
        db: &Database,
        now: OffsetDateTime,
    ) -> Result<KeySet, JobSigningError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        tx.lock(SIGNING_KEY_LOCK).await?;
        let rows = signing::list(&mut tx).await?;
        let current = active_row(&rows, SigningKeyRole::Current)?.id;
        let next_row = active_row(&rows, SigningKeyRole::Next)?;
        let next = next_row.id;
        // The associated data of the wrapped key binds the role, so the promoted key is re-wrapped
        // under the current role (ADR-0026, section CA key format).
        let material = self.unwrap_bytes(next_row)?;

        signing::schedule_retire(&mut tx, current, now + ROTATION_OVERLAP).await?;
        let wrapped = wrap_signing_key(
            self.provider.current(),
            PLATFORM_TENANT_ID.as_uuid(),
            SigningKeyRole::Current.as_str(),
            &material,
        )?;
        signing::set_role_and_wrapped(
            &mut tx,
            next,
            SigningKeyRole::Current,
            wrapped.kek_id.as_str(),
            &wrapped.nonce,
            &wrapped.ciphertext,
        )
        .await?;
        let new_next = self
            .generate_and_insert(&mut tx, SigningKeyRole::Next)
            .await?;
        audit_rotation(&mut tx, next, current, new_next).await?;
        tx.commit().await?;

        self.load(db).await?.ok_or(JobSigningError::Missing)
    }

    /// Retires the old keys whose overlap ended. Returns the number of retired keys (ADR-0026).
    pub async fn retire_due(
        &self,
        db: &Database,
        now: OffsetDateTime,
    ) -> Result<u64, JobSigningError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        let retired = signing::retire_due(&mut tx, now).await?;
        tx.commit().await?;
        Ok(retired)
    }

    /// Retires every pending old key, because all known online agents acknowledged the new set
    /// (ADR-0026). Returns the number of retired keys.
    pub async fn retire_acknowledged(
        &self,
        db: &Database,
        now: OffsetDateTime,
    ) -> Result<u64, JobSigningError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        let retired = signing::retire_all_pending(&mut tx, now).await?;
        tx.commit().await?;
        Ok(retired)
    }

    fn build(&self, rows: &[SigningKeyRow]) -> Result<KeySet, JobSigningError> {
        let mut current: Option<SigningKeyPair> = None;
        let mut next: Option<SigningKeyPair> = None;
        let mut revoked = Vec::new();
        for row in rows {
            if row.retired_at.is_some() {
                revoked.push(row.id.to_string());
                continue;
            }
            // A key that is scheduled for retirement is in the overlap only; the agents still hold
            // it, and it is not part of the active set.
            if row.retire_after.is_some() {
                continue;
            }
            let pair = self.unwrap_row(row)?;
            match row.role {
                SigningKeyRole::Current => set_once(&mut current, pair, SigningKeyRole::Current)?,
                SigningKeyRole::Next => set_once(&mut next, pair, SigningKeyRole::Next)?,
            }
        }
        Ok(KeySet {
            current: current.ok_or(JobSigningError::Missing)?,
            next: next.ok_or(JobSigningError::Missing)?,
            revoked,
        })
    }

    fn unwrap_row(&self, row: &SigningKeyRow) -> Result<SigningKeyPair, JobSigningError> {
        let key = self.unwrap_bytes(row)?;
        SigningKeyPair::from_stored(row.id, &key)
    }

    fn unwrap_bytes(&self, row: &SigningKeyRow) -> Result<Zeroizing<Vec<u8>>, JobSigningError> {
        let wrapped = WrappedSigningKey {
            kek_id: KekId::parse(&row.kek_id)?,
            nonce: row
                .nonce
                .as_slice()
                .try_into()
                .map_err(|source| JobSigningError::BadNonce { id: row.id, source })?,
            ciphertext: row.wrapped_key.clone(),
        };
        Ok(unwrap_signing_key(
            self.provider.as_ref(),
            PLATFORM_TENANT_ID.as_uuid(),
            row.role.as_str(),
            &wrapped,
        )?)
    }

    async fn generate_and_insert(
        &self,
        tx: &mut restichub_db::TenantTx<'_>,
        role: SigningKeyRole,
    ) -> Result<Uuid, JobSigningError> {
        let id = Uuid::now_v7();
        let mut seed = Zeroizing::new([0_u8; KEY_LEN]);
        getrandom::fill(seed.as_mut_slice()).map_err(JobSigningError::Random)?;
        let key = SigningKey::from_bytes(&seed);
        let wrapped = wrap_signing_key(
            self.provider.current(),
            PLATFORM_TENANT_ID.as_uuid(),
            role.as_str(),
            key.as_bytes().as_slice(),
        )?;
        signing::insert(
            tx,
            &NewSigningKey {
                id,
                role,
                kek_id: wrapped.kek_id.as_str(),
                nonce: &wrapped.nonce,
                wrapped_key: &wrapped.ciphertext,
            },
        )
        .await?;
        Ok(id)
    }
}

fn set_once(
    slot: &mut Option<SigningKeyPair>,
    pair: SigningKeyPair,
    role: SigningKeyRole,
) -> Result<(), JobSigningError> {
    if slot.replace(pair).is_some() {
        return Err(JobSigningError::MultipleActiveRole { role });
    }
    Ok(())
}

/// The one active row of `role`: neither retired nor scheduled for retirement.
fn active_row(
    rows: &[SigningKeyRow],
    role: SigningKeyRole,
) -> Result<&SigningKeyRow, JobSigningError> {
    let mut found: Option<&SigningKeyRow> = None;
    for row in rows {
        if row.role == role && row.retired_at.is_none() && row.retire_after.is_none() {
            if found.is_some() {
                return Err(JobSigningError::MultipleActiveRole { role });
            }
            found = Some(row);
        }
    }
    found.ok_or(JobSigningError::NoActiveRole { role })
}

async fn audit_rotation(
    tx: &mut restichub_db::TenantTx<'_>,
    promoted: Uuid,
    previous: Uuid,
    new_next: Uuid,
) -> Result<(), JobSigningError> {
    let mut event = AuditEvent::new(AuditAction::SigningKeyRotate, "signing_key");
    event.object_id = Some(promoted);
    let after = serde_json::json!({
        "current": promoted.to_string(),
        "previous": previous.to_string(),
        "next": new_next.to_string(),
    });
    event.diff = Some(audit::diff(None, Some(&after), &[]));
    audit::insert(tx, &event).await?;
    Ok(())
}

fn timestamp(time: OffsetDateTime) -> Timestamp {
    Timestamp {
        seconds: time.unix_timestamp(),
        nanos: time.nanosecond() as i32,
    }
}
