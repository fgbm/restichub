//! The job signing key set of the installation (ADR-0026, ADR-0005, FR-PR-03, FR-PR-11).
//!
//! The control plane signs each job with an Ed25519 key. One key set serves all tenants and lives
//! in the platform tenant. The set holds the current key that signs the jobs and the next key that
//! the agents receive before a rotation. The key encryption key (KEK) wraps each private key, in
//! the manner of a CA key (ADR-0026, section Job signing keys).
//!
//! A rotation promotes the next key to current, generates a new next key and schedules the
//! retirement of the old current key. The old key retires when all known online agents
//! acknowledged the new set, or at most [`ROTATION_OVERLAP`] after the rotation starts; a periodic
//! control plane task calls [`JobSigningService::retire_due`], so the bound holds without a manual
//! call. An agent that was offline longer re-syncs the key set at its reconnect (ADR-0030); the
//! delivery is [`KeySet::key_update`]. A signature checks the table version first and reloads a key
//! set that another control plane rotated or retired.
//!
//! The plaintext key never leaves this module. The types hold no `Debug` that shows a key, and the
//! key set is never written to a log or to a process argument.

use std::sync::{Arc, Mutex};

use ed25519_dalek::{SigningKey, VerifyingKey};
use prost_types::Timestamp;
use restichub_db::audit::{self, AuditAction, AuditEvent};
use restichub_db::signing::{self, NewSigningKey, SigningKeyRow, Version};
use restichub_db::{Database, DbError, PLATFORM_TENANT_ID, SigningKeyRole};
use restichub_proto::agent::v1::{Job, KeyUpdate, SignedJob, SigningKeyUpdate};
use restichub_proto::signing::{EnrollmentKey, sign_enrollment_keys, sign_job, sign_key_update};
use restichub_secrets::{
    CryptoError, KekError, KekId, KekProvider, WrappedSigningKey, unwrap_signing_key,
    wrap_signing_key,
};
use time::{Duration, OffsetDateTime};
use tokio::sync::broadcast;
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
    #[error("every stored job signing key is retired; the installation has no active key set")]
    AllRetired,
    #[error("a job signing key is still in its rotation overlap; the next rotation must wait")]
    RotationPending,
    #[error("the installation holds no active {role:?} job signing key")]
    NoActiveRole { role: SigningKeyRole },
    #[error("the installation holds more than one active {role:?} job signing key")]
    MultipleActiveRole { role: SigningKeyRole },
    #[error("the stored job signing key {id} is not a valid Ed25519 key")]
    BadKey { id: Uuid },
    #[error("the stored job signing key {id} does not match its stored public key")]
    KeyMismatch { id: Uuid },
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
        let seed: Zeroizing<[u8; KEY_LEN]> = Zeroizing::new(
            bytes
                .try_into()
                .map_err(|_| JobSigningError::BadKey { id })?,
        );
        Ok(Self {
            id: id.to_string(),
            key: SigningKey::from_bytes(&seed),
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

    /// The public job signing set that the response delivers (ADR-0026).
    ///
    /// The set holds the current key and the next key, so an agent verifies a job that the current
    /// key signed at once. The current key signs the set (ADR-0026).
    pub fn enrollment_keys(&self) -> (Vec<EnrollmentKey>, Vec<u8>) {
        let keys = vec![
            EnrollmentKey {
                role: "current".to_owned(),
                key_id: self.current.id().to_owned(),
                public_key: self.current.verifying_key().to_bytes().to_vec(),
            },
            EnrollmentKey {
                role: "next".to_owned(),
                key_id: self.next.id().to_owned(),
                public_key: self.next.verifying_key().to_bytes().to_vec(),
            },
        ];
        let signature = sign_enrollment_keys(&keys, &self.current.key);
        (keys, signature)
    }
}

/// The loaded key set with the table version it was read at, for the freshness check before a
/// signature (ADR-0026).
#[derive(Debug)]
struct CachedSet {
    version: Version,
    set: Arc<KeySet>,
}

/// One signing key rotation, announced to the agent streams (ADR-0026).
#[derive(Debug, Clone)]
pub struct RotationEvent {
    /// The old current key that retires when every online agent acknowledged the new set.
    pub pending_key_id: Uuid,
    /// The new current key that the agents acknowledge.
    pub current_key_id: String,
}

/// Owns the job signing key set of the installation.
#[derive(Debug, Clone)]
pub struct JobSigningService {
    provider: Arc<dyn KekProvider>,
    cache: Arc<Mutex<Option<CachedSet>>>,
    events: broadcast::Sender<RotationEvent>,
}

impl JobSigningService {
    pub fn new(provider: Arc<dyn KekProvider>) -> Self {
        // The channel carries a rotation to the open agent streams (ADR-0026). A lost event is
        // harmless: the periodic retirement still bounds the overlap.
        let (events, _) = broadcast::channel(16);
        Self {
            provider,
            cache: Arc::new(Mutex::new(None)),
            events,
        }
    }

    /// Subscribes to the rotation events. The agent stream re-delivers the key set after a rotation.
    pub fn subscribe(&self) -> broadcast::Receiver<RotationEvent> {
        self.events.subscribe()
    }

    /// Creates the key set on the first start and keeps it healthy on every later start.
    ///
    /// An advisory lock serializes two starts, so exactly one start creates the set. A later start
    /// loads the stored keys; an incorrect KEK fails the unwrap, so the control plane refuses to
    /// start (AC-PR-02). A non-empty table with every row retired also refuses the start; the
    /// operator must investigate rather than let a silent new set replace the revoked keys.
    pub async fn ensure_initialized(&self, db: &Database) -> Result<KeySet, JobSigningError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        tx.lock(SIGNING_KEY_LOCK).await?;
        let rows = signing::list(&mut tx).await?;
        if rows.is_empty() {
            self.generate_and_insert(&mut tx, SigningKeyRole::Current)
                .await?;
            self.generate_and_insert(&mut tx, SigningKeyRole::Next)
                .await?;
        } else if rows.iter().all(|row| row.retired_at.is_some()) {
            return Err(JobSigningError::AllRetired);
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

    /// Signs with the current key after a version check (FR-PR-03).
    ///
    /// The service checks the key set version before each signature and reloads the set when
    /// another control plane rotated it, so a long-lived instance never signs with a stale current
    /// key (ADR-0026).
    pub async fn sign_job(&self, db: &Database, job: &Job) -> Result<SignedJob, JobSigningError> {
        Ok(self.current_set(db).await?.sign_job(job))
    }

    /// Builds the signed set update for an agent (FR-PR-11).
    ///
    /// The version check reloads a set that another control plane rotated or retired, so the update
    /// names the current key and the latest revoked key identifiers.
    pub async fn key_update(
        &self,
        db: &Database,
        now: OffsetDateTime,
    ) -> Result<SigningKeyUpdate, JobSigningError> {
        Ok(self.current_set(db).await?.key_update(now))
    }

    /// Builds the public job signing set that the response delivers (ADR-0026).
    ///
    /// The version check reloads a set that another control plane rotated, so the response names
    /// the current key and the next key.
    pub async fn enrollment_keys(
        &self,
        db: &Database,
    ) -> Result<(Vec<EnrollmentKey>, Vec<u8>), JobSigningError> {
        Ok(self.current_set(db).await?.enrollment_keys())
    }

    async fn current_set(&self, db: &Database) -> Result<Arc<KeySet>, JobSigningError> {
        let version = {
            let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
            let version = signing::version(&mut tx).await?;
            tx.commit().await?;
            version
        };
        if let Some(set) = self.cached(&version) {
            return Ok(set);
        }
        let set = Arc::new(self.load(db).await?.ok_or(JobSigningError::Missing)?);
        if let Ok(mut cache) = self.cache.lock() {
            *cache = Some(CachedSet {
                version,
                set: Arc::clone(&set),
            });
        }
        Ok(set)
    }

    fn cached(&self, version: &Version) -> Option<Arc<KeySet>> {
        let cache = self.cache.lock().ok()?;
        let cached = cache.as_ref()?;
        (&cached.version == version).then(|| Arc::clone(&cached.set))
    }

    /// Rotates the key set (ADR-0026).
    ///
    /// The next key becomes the current key, a new next key is generated and the old current key is
    /// scheduled for retirement after [`ROTATION_OVERLAP`]. An advisory lock serializes two
    /// rotations. The new current key signs the jobs at once; the agents already hold it from the
    /// earlier delivery. A rotation refuses while an earlier rotation still holds its overlap, so
    /// one overlap row exists at a time and the re-sync path stays simple.
    pub async fn rotate(
        &self,
        db: &Database,
        now: OffsetDateTime,
    ) -> Result<KeySet, JobSigningError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        tx.lock(SIGNING_KEY_LOCK).await?;
        let rows = signing::list(&mut tx).await?;
        if rows
            .iter()
            .any(|row| row.retire_after.is_some() && row.retired_at.is_none())
        {
            return Err(JobSigningError::RotationPending);
        }
        let current = active_row(&rows, SigningKeyRole::Current)?.id;
        let next_row = active_row(&rows, SigningKeyRole::Next)?;
        let next = next_row.id;
        // The promoted key is unwrapped and checked against its row before any write, so a corrupt
        // row cannot be promoted. The associated data of the wrapped key binds the role and the key
        // ID, so the promoted key is re-wrapped under the current role (ADR-0026, section CA key
        // format).
        let material = self.unwrap_row(next_row)?;

        signing::schedule_retire(&mut tx, current, now + ROTATION_OVERLAP).await?;
        let wrapped = wrap_signing_key(
            self.provider.current(),
            PLATFORM_TENANT_ID.as_uuid(),
            next,
            SigningKeyRole::Current.as_str(),
            material.key.as_bytes(),
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

        // Announce the rotation, so each open stream delivers the fresh key set (ADR-0026).
        let _ = self.events.send(RotationEvent {
            pending_key_id: current,
            current_key_id: next.to_string(),
        });

        self.load(db).await?.ok_or(JobSigningError::Missing)
    }

    /// Retires the old keys whose overlap ended. Returns the number of retired keys (ADR-0026).
    ///
    /// The control plane writes a [`AuditAction::SigningKeyRetire`] record for each retired key in
    /// the same transaction.
    pub async fn retire_due(
        &self,
        db: &Database,
        now: OffsetDateTime,
    ) -> Result<u64, JobSigningError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        let retired = signing::retire_due(&mut tx, now).await?;
        for id in &retired {
            audit_retire(&mut tx, *id).await?;
        }
        tx.commit().await?;
        Ok(retired.len() as u64)
    }

    /// Retires the named old key after all known online agents acknowledged the new set
    /// (ADR-0026).
    ///
    /// The advisory lock is the same as the rotation lock, so an acknowledgement and a rotation do
    /// not interleave. Only the named key retires; a newer rotation's overlap key stays. The
    /// control plane writes a [`AuditAction::SigningKeyRetire`] record in the same transaction.
    pub async fn retire_acknowledged(
        &self,
        db: &Database,
        key_id: Uuid,
        now: OffsetDateTime,
    ) -> Result<u64, JobSigningError> {
        let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
        tx.lock(SIGNING_KEY_LOCK).await?;
        let retired = signing::retire_pending(&mut tx, key_id, now).await?;
        for id in &retired {
            audit_retire(&mut tx, *id).await?;
        }
        tx.commit().await?;
        Ok(retired.len() as u64)
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
        let pair = SigningKeyPair::from_stored(row.id, &key)?;
        // The AAD binds the blob to the row ID; the public key binds the unwrapped key to the row,
        // so a swapped blob fails even if the AAD check passes (T19).
        if pair.verifying_key().to_bytes().as_slice() != row.public_key.as_slice() {
            return Err(JobSigningError::KeyMismatch { id: row.id });
        }
        Ok(pair)
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
            row.id,
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
        let public_key = key.verifying_key().to_bytes();
        let wrapped = wrap_signing_key(
            self.provider.current(),
            PLATFORM_TENANT_ID.as_uuid(),
            id,
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
                public_key: public_key.as_slice(),
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

async fn audit_retire(
    tx: &mut restichub_db::TenantTx<'_>,
    id: Uuid,
) -> Result<(), JobSigningError> {
    let mut event = AuditEvent::new(AuditAction::SigningKeyRetire, "signing_key");
    event.object_id = Some(id);
    audit::insert(tx, &event).await?;
    Ok(())
}

fn timestamp(time: OffsetDateTime) -> Timestamp {
    Timestamp {
        seconds: time.unix_timestamp(),
        nanos: time.nanosecond() as i32,
    }
}
