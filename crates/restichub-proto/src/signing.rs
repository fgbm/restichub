//! Job signing, signing key rotation and the job journal (ADR-0005, ADR-0020,
//! FR-PR-03, FR-PR-11).
//!
//! The control plane signs the serialized bytes of a [`Job`] with a context
//! prefix. The agent verifies the signature before it decodes the bytes. Then
//! the agent checks the addressee, the tenant and the time rules. Replay
//! protection uses the job journal, not a sequence number (ADR-0020).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use prost::Message;
use prost_types::Timestamp;

use crate::agent::v1::{Job, JobResult, KeyUpdate, RejectReason, SignedJob, SigningKeyUpdate};

/// Context prefix of the bytes that the control plane signs for a job
/// (ADR-0020).
pub const JOB_SIGNING_CONTEXT: &[u8] = b"restichub-job-v1\0";

/// Context prefix of the bytes that the control plane signs for a key update
/// (FR-PR-11).
pub const KEY_UPDATE_SIGNING_CONTEXT: &[u8] = b"restichub-key-update-v1\0";

/// Maximum lifetime of a job (ADR-0020).
pub const MAX_JOB_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

/// Accepted difference between the clock of the control plane and the clock of
/// the agent (ADR-0020).
pub const CLOCK_SKEW: Duration = Duration::from_secs(5 * 60);

/// A set of public job signing keys, indexed by `key_id` (ADR-0005).
///
/// A revoked key cannot verify a job or a key update (FR-PR-11).
#[derive(Debug, Default, Clone)]
pub struct KeyRing {
    keys: HashMap<String, VerifyingKey>,
    revoked: HashSet<String>,
}

/// A reason why a signing key update did not apply (FR-PR-11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyUpdateError {
    UnknownSigner,
    RevokedSigner,
    BadSignature,
    Malformed,
    SignerMismatch,
    KeyIdConflict,
    RevokedKeyId,
}

impl KeyRing {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one public key to the key ring.
    ///
    /// A key with the same `key_id` and the same bytes is a no-op. A different
    /// key with the same `key_id` is an error. A revoked `key_id` cannot be
    /// inserted again (FR-PR-11).
    ///
    /// # Errors
    ///
    /// Returns [`KeyUpdateError::KeyIdConflict`] for a different key with the
    /// same `key_id`, and [`KeyUpdateError::RevokedKeyId`] for a revoked
    /// `key_id`.
    pub fn insert(
        &mut self,
        key_id: impl Into<String>,
        key: VerifyingKey,
    ) -> Result<(), KeyUpdateError> {
        let key_id = key_id.into();
        if self.revoked.contains(&key_id) {
            return Err(KeyUpdateError::RevokedKeyId);
        }
        match self.keys.get(&key_id) {
            Some(existing) if existing == &key => Ok(()),
            Some(_) => Err(KeyUpdateError::KeyIdConflict),
            None => {
                self.keys.insert(key_id, key);
                Ok(())
            }
        }
    }

    /// Revoked keys are not returned.
    #[must_use]
    pub fn get(&self, key_id: &str) -> Option<&VerifyingKey> {
        if self.revoked.contains(key_id) {
            None
        } else {
            self.keys.get(key_id)
        }
    }

    #[must_use]
    pub fn is_revoked(&self, key_id: &str) -> bool {
        self.revoked.contains(key_id)
    }

    /// Applies a signed key update (FR-PR-11).
    ///
    /// The agent verifies the signature before it decodes the update (ADR-0005):
    ///
    /// 1. It finds the signer by the `signer_key_id` of the envelope.
    /// 2. The signer must not be revoked.
    /// 3. It verifies the signature over the context prefix and the raw
    ///    `key_update` bytes.
    /// 4. It decodes the `KeyUpdate` and compares its `signer_key_id` with the
    ///    envelope. A difference is an error.
    ///
    /// The update inserts its new key before it applies the revocations, so an
    /// update may revoke its own signer: the ring keeps a usable key when the
    /// new key is in place.
    ///
    /// # Errors
    ///
    /// Returns a [`KeyUpdateError`] when the signer is unknown or revoked, when
    /// the signature is bad, when the update is malformed, when the signer
    /// identifiers differ, or when a new key does not apply.
    pub fn apply_key_update(&mut self, update: &SigningKeyUpdate) -> Result<(), KeyUpdateError> {
        if self.revoked.contains(&update.signer_key_id) {
            return Err(KeyUpdateError::RevokedSigner);
        }
        let signer = self
            .keys
            .get(&update.signer_key_id)
            .ok_or(KeyUpdateError::UnknownSigner)?;

        let signature =
            signature_from_bytes(&update.signature).ok_or(KeyUpdateError::BadSignature)?;
        let input = signing_input(KEY_UPDATE_SIGNING_CONTEXT, &update.key_update);
        signer
            .verify_strict(&input, &signature)
            .map_err(|_| KeyUpdateError::BadSignature)?;

        let key_update = KeyUpdate::decode(update.key_update.as_slice())
            .map_err(|_| KeyUpdateError::Malformed)?;

        if key_update.signer_key_id != update.signer_key_id {
            return Err(KeyUpdateError::SignerMismatch);
        }

        let public_key =
            public_key_from_bytes(&key_update.public_key).ok_or(KeyUpdateError::Malformed)?;
        self.insert(key_update.key_id.clone(), public_key)?;

        for key_id in &key_update.revoked_key_ids {
            self.revoke(key_id);
        }
        Ok(())
    }

    /// Revokes a key identifier, so the ring never returns it again (FR-PR-11).
    ///
    /// The agent applies the persisted revoked set at load, so a key that the control plane retired
    /// stays revoked across a restart.
    pub fn revoke(&mut self, key_id: &str) {
        self.keys.remove(key_id);
        self.revoked.insert(key_id.to_owned());
    }
}

/// The values that the agent expects in an incoming job (FR-PR-03).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    pub agent_id: String,
    pub tenant_id: String,
    pub now: Timestamp,
}

/// Explains why a job was rejected (FR-PR-03).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub reason: RejectReason,
    /// `Some` when the signature was valid and the bytes decoded.
    pub job_id: Option<String>,
}

/// The signature covers the context prefix and the serialized [`Job`] bytes
/// (ADR-0005, ADR-0020).
#[must_use]
pub fn sign_job(job: &Job, key_id: impl Into<String>, key: &SigningKey) -> SignedJob {
    let bytes = job.encode_to_vec();
    let signature = key
        .sign(&signing_input(JOB_SIGNING_CONTEXT, &bytes))
        .to_bytes()
        .to_vec();
    SignedJob {
        job: bytes,
        signature,
        signer_key_id: key_id.into(),
    }
}

/// The signature covers the context prefix and the serialized [`KeyUpdate`]
/// bytes (FR-PR-11). The envelope carries the `signer_key_id` so the agent looks
/// up the signer before it decodes the update (ADR-0005).
#[must_use]
pub fn sign_key_update(update: &KeyUpdate, key: &SigningKey) -> SigningKeyUpdate {
    let bytes = update.encode_to_vec();
    let signature = key
        .sign(&signing_input(KEY_UPDATE_SIGNING_CONTEXT, &bytes))
        .to_bytes()
        .to_vec();
    SigningKeyUpdate {
        key_update: bytes,
        signature,
        signer_key_id: update.signer_key_id.clone(),
    }
}

/// The domain prefix of the signature over the enrollment key set (ADR-0026).
pub const ENROLLMENT_KEYS_SIGNING_CONTEXT: &[u8] = b"restichub-enroll-keys-v1\0";

/// One public job signing key that the enrollment response delivers (ADR-0026).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentKey {
    /// `current` or `next` (ADR-0026).
    pub role: String,
    pub key_id: String,
    /// 32 bytes of an Ed25519 value.
    pub public_key: Vec<u8>,
}

/// Why an enrollment key set did not verify (ADR-0026).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentKeyError {
    BadSignature,
    Malformed,
}

/// The exact bytes that the signature over an enrollment key set covers (ADR-0026).
///
/// The order is significant: the count, then for each key the length and the bytes of the role, of
/// the key identifier and of the public key. The length prefixes make the encoding unambiguous.
#[must_use]
pub fn enrollment_keys_input(keys: &[EnrollmentKey]) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&length_prefix(keys.len()).to_be_bytes());
    for key in keys {
        push_field(&mut payload, key.role.as_bytes());
        push_field(&mut payload, key.key_id.as_bytes());
        push_field(&mut payload, &key.public_key);
    }
    signing_input(ENROLLMENT_KEYS_SIGNING_CONTEXT, &payload)
}

fn push_field(payload: &mut Vec<u8>, field: &[u8]) {
    payload.extend_from_slice(&length_prefix(field.len()).to_be_bytes());
    payload.extend_from_slice(field);
}

fn length_prefix(length: usize) -> u32 {
    u32::try_from(length).unwrap_or(u32::MAX)
}

/// Signs the delivered public set with the current key (ADR-0026).
#[must_use]
pub fn sign_enrollment_keys(keys: &[EnrollmentKey], key: &SigningKey) -> Vec<u8> {
    key.sign(&enrollment_keys_input(keys)).to_bytes().to_vec()
}

/// Verifies the signature over an enrollment key set (ADR-0026).
///
/// The caller gives the current public key from the same set, so the check proves that the set is
/// self-consistent and detects a partial tamper of the transport. The authenticity itself comes
/// from the enrollment channel: the verified TLS connection and the one-time token (threat T13).
///
/// # Errors
///
/// Returns [`EnrollmentKeyError::Malformed`] for an empty set or a bad key length, and
/// [`EnrollmentKeyError::BadSignature`] for a bad signature.
pub fn verify_enrollment_keys(
    keys: &[EnrollmentKey],
    signature: &[u8],
    current: &VerifyingKey,
) -> Result<(), EnrollmentKeyError> {
    if keys.is_empty() {
        return Err(EnrollmentKeyError::Malformed);
    }
    for key in keys {
        if public_key_from_bytes(&key.public_key).is_none() {
            return Err(EnrollmentKeyError::Malformed);
        }
    }
    let signature = signature_from_bytes(signature).ok_or(EnrollmentKeyError::BadSignature)?;
    current
        .verify_strict(&enrollment_keys_input(keys), &signature)
        .map_err(|_| EnrollmentKeyError::BadSignature)
}

/// Verifies a signed job and returns the decoded [`Job`].
///
/// The function verifies the signature over the raw bytes before it decodes the
/// job (ADR-0005). Then it checks the addressee, the tenant and the time rules
/// (FR-PR-03, ADR-0020). Replay protection is not here; the caller uses
/// [`admit`] with a [`JobJournal`] (ADR-0020).
///
/// # Errors
///
/// Returns a [`Rejection`] for the first failed check.
pub fn verify_job(
    signed: &SignedJob,
    keys: &KeyRing,
    expected: &Expected,
) -> Result<Job, Rejection> {
    let key = keys
        .get(&signed.signer_key_id)
        .ok_or_else(no_job_id(RejectReason::UnknownKey))?;

    let signature = signature_from_bytes(&signed.signature)
        .ok_or_else(no_job_id(RejectReason::BadSignature))?;
    let input = signing_input(JOB_SIGNING_CONTEXT, &signed.job);
    key.verify_strict(&input, &signature)
        .map_err(|_| no_job_id(RejectReason::BadSignature)())?;

    let job =
        Job::decode(signed.job.as_slice()).map_err(|_| no_job_id(RejectReason::Malformed)())?;
    let job_id = job.job_id.clone();

    if expected.agent_id.is_empty()
        || expected.tenant_id.is_empty()
        || job.agent_id.is_empty()
        || job.tenant_id.is_empty()
    {
        return Err(rejection(RejectReason::Malformed, &job_id));
    }
    if job.agent_id != expected.agent_id {
        return Err(rejection(RejectReason::WrongAgent, &job_id));
    }
    if job.tenant_id != expected.tenant_id {
        return Err(rejection(RejectReason::WrongTenant, &job_id));
    }

    let expires_at = job
        .expires_at
        .as_ref()
        .ok_or_else(|| rejection(RejectReason::Malformed, &job_id))?;
    let issued_at = job
        .issued_at
        .as_ref()
        .ok_or_else(|| rejection(RejectReason::Malformed, &job_id))?;

    let issued_at_nanos = timestamp_nanos(issued_at);
    let expires_at_nanos = timestamp_nanos(expires_at);
    let now_nanos = timestamp_nanos(&expected.now);

    if expires_at_nanos - issued_at_nanos > MAX_JOB_LIFETIME.as_nanos() as i128 {
        return Err(rejection(RejectReason::LifetimeTooLong, &job_id));
    }
    if issued_at_nanos > now_nanos + CLOCK_SKEW.as_nanos() as i128 {
        return Err(rejection(RejectReason::NotYetValid, &job_id));
    }
    if now_nanos >= expires_at_nanos {
        return Err(rejection(RejectReason::Expired, &job_id));
    }

    Ok(job)
}

/// State of one job in a [`JobJournal`] (ADR-0020).
#[derive(Debug, Clone, PartialEq)]
pub enum JournalState {
    Unknown,
    InProgress,
    Completed(JobResult),
}

/// Result of [`admit`] for one job (ADR-0020).
#[derive(Debug, Clone, PartialEq)]
pub enum Admission {
    New,
    InProgress,
    AlreadyCompleted(JobResult),
}

/// Storage of accepted and completed jobs (ADR-0020). The agent crate adds a
/// persistent implementation; [`InMemoryJournal`] serves tests.
pub trait JobJournal {
    fn lookup(&self, job_id: &str) -> JournalState;
    fn record_accepted(&mut self, job_id: &str, expires_at: Timestamp);
    fn record_completed(&mut self, job_id: &str, result: JobResult);
    /// Removes the accepted jobs whose expiry time plus the clock skew is
    /// before `now` (ADR-0020).
    fn purge_expired(&mut self, now: Timestamp);
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryJournal {
    entries: HashMap<String, JournalEntry>,
}

#[derive(Debug, Clone)]
enum JournalEntry {
    InProgress { expires_at: Timestamp },
    Completed(JobResult),
}

impl JobJournal for InMemoryJournal {
    fn lookup(&self, job_id: &str) -> JournalState {
        match self.entries.get(job_id) {
            None => JournalState::Unknown,
            Some(JournalEntry::InProgress { .. }) => JournalState::InProgress,
            Some(JournalEntry::Completed(result)) => JournalState::Completed(result.clone()),
        }
    }

    fn record_accepted(&mut self, job_id: &str, expires_at: Timestamp) {
        self.entries
            .insert(job_id.to_owned(), JournalEntry::InProgress { expires_at });
    }

    fn record_completed(&mut self, job_id: &str, result: JobResult) {
        self.entries
            .insert(job_id.to_owned(), JournalEntry::Completed(result));
    }

    fn purge_expired(&mut self, now: Timestamp) {
        let now = timestamp_nanos(&now);
        let retain_until = CLOCK_SKEW.as_nanos() as i128;
        self.entries.retain(|_, entry| match entry {
            JournalEntry::InProgress { expires_at } => {
                timestamp_nanos(expires_at) + retain_until > now
            }
            JournalEntry::Completed(_) => true,
        });
    }
}

/// A new job is recorded as accepted; a completed job returns its stored result (ADR-0020).
///
/// An accepted job that has no result is `InProgress`. The caller decides what that means: the
/// agent runs a job of restic steps again after a restart, and answers a job with a script or hook
/// step with a final result instead, because a program that ran may not run twice (ADR-0037).
#[must_use]
pub fn admit(job: &Job, journal: &mut impl JobJournal, now: Timestamp) -> Admission {
    match journal.lookup(&job.job_id) {
        JournalState::Unknown => {
            journal.record_accepted(&job.job_id, job.expires_at.unwrap_or(now));
            Admission::New
        }
        JournalState::InProgress => Admission::InProgress,
        JournalState::Completed(result) => Admission::AlreadyCompleted(result),
    }
}

fn signing_input(context: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut input = Vec::with_capacity(context.len() + payload.len());
    input.extend_from_slice(context);
    input.extend_from_slice(payload);
    input
}

/// `None` for a wrong length.
fn signature_from_bytes(bytes: &[u8]) -> Option<Signature> {
    let bytes: [u8; Signature::BYTE_SIZE] = bytes.try_into().ok()?;
    Some(Signature::from_bytes(&bytes))
}

/// Returns `None` for a wrong length or an invalid point.
fn public_key_from_bytes(bytes: &[u8]) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = bytes.try_into().ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}

fn timestamp_nanos(timestamp: &Timestamp) -> i128 {
    i128::from(timestamp.seconds) * 1_000_000_000 + i128::from(timestamp.nanos)
}

fn rejection(reason: RejectReason, job_id: &str) -> Rejection {
    Rejection {
        reason,
        job_id: Some(job_id.to_owned()),
    }
}

/// The closure fits `Result::map_err` and `Option::ok_or_else`.
fn no_job_id(reason: RejectReason) -> impl Fn() -> Rejection {
    move || Rejection {
        reason,
        job_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_ID: &str = "job-key-1";
    const JOB_ID: &str = "job-1";

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn other_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[9u8; 32])
    }

    fn key_ring() -> KeyRing {
        let mut ring = KeyRing::new();
        assert!(ring.insert(KEY_ID, signing_key().verifying_key()).is_ok());
        ring
    }

    fn expected() -> Expected {
        Expected {
            agent_id: "agent-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            now: Timestamp {
                seconds: 1_000,
                nanos: 0,
            },
        }
    }

    fn job() -> Job {
        Job {
            job_id: JOB_ID.to_owned(),
            tenant_id: "tenant-1".to_owned(),
            agent_id: "agent-1".to_owned(),
            issued_at: Some(Timestamp {
                seconds: 900,
                nanos: 0,
            }),
            expires_at: Some(Timestamp {
                seconds: 2_000,
                nanos: 0,
            }),
            ..Job::default()
        }
    }

    fn rejection_with(reason: RejectReason) -> Rejection {
        Rejection {
            reason,
            job_id: Some(JOB_ID.to_owned()),
        }
    }

    fn no_id(reason: RejectReason) -> Rejection {
        Rejection {
            reason,
            job_id: None,
        }
    }

    #[test]
    fn ac_pr_01_rejects_tampered_payload() {
        let mut signed = sign_job(&job(), KEY_ID, &signing_key());
        signed.job[4] ^= 0xff;

        assert_eq!(
            verify_job(&signed, &key_ring(), &expected()),
            Err(no_id(RejectReason::BadSignature))
        );
    }

    #[test]
    fn ac_pr_01_rejects_other_agent() {
        let mut other = job();
        other.agent_id = "agent-2".to_owned();
        let signed = sign_job(&other, KEY_ID, &signing_key());

        assert_eq!(
            verify_job(&signed, &key_ring(), &expected()),
            Err(rejection_with(RejectReason::WrongAgent))
        );
    }

    #[test]
    fn ac_pr_01_rejects_other_tenant() {
        let mut other = job();
        other.tenant_id = "tenant-2".to_owned();
        let signed = sign_job(&other, KEY_ID, &signing_key());

        assert_eq!(
            verify_job(&signed, &key_ring(), &expected()),
            Err(rejection_with(RejectReason::WrongTenant))
        );
    }

    #[test]
    fn ac_pr_01_rejects_expired() {
        let mut other = job();
        other.expires_at = Some(Timestamp {
            seconds: 500,
            nanos: 0,
        });
        let signed = sign_job(&other, KEY_ID, &signing_key());

        assert_eq!(
            verify_job(&signed, &key_ring(), &expected()),
            Err(rejection_with(RejectReason::Expired))
        );
    }

    #[test]
    fn ac_pr_01_rejects_unknown_key() {
        let signed = sign_job(&job(), "job-key-2", &signing_key());

        assert_eq!(
            verify_job(&signed, &key_ring(), &expected()),
            Err(no_id(RejectReason::UnknownKey))
        );
    }

    #[test]
    fn accepts_valid_job() {
        let original = job();
        let signed = sign_job(&original, KEY_ID, &signing_key());

        assert_eq!(verify_job(&signed, &key_ring(), &expected()), Ok(original));
    }

    #[test]
    fn rejects_missing_issued_at() {
        let mut other = job();
        other.issued_at = None;
        let signed = sign_job(&other, KEY_ID, &signing_key());

        assert_eq!(
            verify_job(&signed, &key_ring(), &expected()),
            Err(rejection_with(RejectReason::Malformed))
        );
    }

    #[test]
    fn rejects_missing_expires_at() {
        let mut other = job();
        other.expires_at = None;
        let signed = sign_job(&other, KEY_ID, &signing_key());

        assert_eq!(
            verify_job(&signed, &key_ring(), &expected()),
            Err(rejection_with(RejectReason::Malformed))
        );
    }

    #[test]
    fn rejects_lifetime_too_long() {
        let mut other = job();
        other.expires_at = Some(Timestamp {
            seconds: 900 + 24 * 60 * 60 + 1,
            nanos: 0,
        });
        let signed = sign_job(&other, KEY_ID, &signing_key());

        assert_eq!(
            verify_job(&signed, &key_ring(), &expected()),
            Err(rejection_with(RejectReason::LifetimeTooLong))
        );
    }

    #[test]
    fn rejects_not_yet_valid() {
        let mut other = job();
        other.issued_at = Some(Timestamp {
            seconds: 1_000 + 5 * 60 + 1,
            nanos: 0,
        });
        other.expires_at = Some(Timestamp {
            seconds: 2_000,
            nanos: 0,
        });
        let signed = sign_job(&other, KEY_ID, &signing_key());

        assert_eq!(
            verify_job(&signed, &key_ring(), &expected()),
            Err(rejection_with(RejectReason::NotYetValid))
        );
    }

    #[test]
    fn rejects_empty_expected_identity() {
        let signed = sign_job(&job(), KEY_ID, &signing_key());
        let mut expected = expected();
        expected.agent_id.clear();

        assert_eq!(
            verify_job(&signed, &key_ring(), &expected),
            Err(rejection_with(RejectReason::Malformed))
        );
    }

    #[test]
    fn rejects_empty_job_identity() {
        let mut other = job();
        other.tenant_id.clear();
        let signed = sign_job(&other, KEY_ID, &signing_key());

        assert_eq!(
            verify_job(&signed, &key_ring(), &expected()),
            Err(rejection_with(RejectReason::Malformed))
        );
    }

    fn completed_result(job_id: &str) -> JobResult {
        JobResult {
            job_id: job_id.to_owned(),
            ..JobResult::default()
        }
    }

    #[test]
    fn admit_new_job() {
        let mut journal = InMemoryJournal::default();
        let now = Timestamp {
            seconds: 1_000,
            nanos: 0,
        };

        assert_eq!(admit(&job(), &mut journal, now), Admission::New);
        assert_eq!(journal.lookup(JOB_ID), JournalState::InProgress);
    }

    #[test]
    fn admit_in_progress_job() {
        let mut journal = InMemoryJournal::default();
        let now = Timestamp {
            seconds: 1_000,
            nanos: 0,
        };
        let _ = admit(&job(), &mut journal, now);

        assert_eq!(admit(&job(), &mut journal, now), Admission::InProgress);
    }

    #[test]
    fn admit_completed_job() {
        let mut journal = InMemoryJournal::default();
        let result = completed_result(JOB_ID);
        let now = Timestamp {
            seconds: 1_000,
            nanos: 0,
        };
        journal.record_accepted(JOB_ID, job().expires_at.unwrap_or(now));
        journal.record_completed(JOB_ID, result.clone());

        assert_eq!(
            admit(&job(), &mut journal, now),
            Admission::AlreadyCompleted(result)
        );
    }

    #[test]
    fn purge_expired_jobs() {
        let mut journal = InMemoryJournal::default();
        journal.record_accepted(
            JOB_ID,
            Timestamp {
                seconds: 1_000,
                nanos: 0,
            },
        );

        journal.purge_expired(Timestamp {
            seconds: 1_299,
            nanos: 0,
        });
        assert_eq!(journal.lookup(JOB_ID), JournalState::InProgress);

        journal.purge_expired(Timestamp {
            seconds: 1_300,
            nanos: 0,
        });
        assert_eq!(journal.lookup(JOB_ID), JournalState::Unknown);
    }

    fn rotation_update(
        key_id: &str,
        public_key: &SigningKey,
        signer_key_id: &str,
        revoked: &[&str],
    ) -> KeyUpdate {
        KeyUpdate {
            key_id: key_id.to_owned(),
            public_key: public_key.verifying_key().to_bytes().to_vec(),
            signer_key_id: signer_key_id.to_owned(),
            issued_at: Some(Timestamp {
                seconds: 1_000,
                nanos: 0,
            }),
            revoked_key_ids: revoked.iter().map(|id| (*id).to_owned()).collect(),
        }
    }

    #[test]
    fn ac_pr_09_accepts_key_rotation() {
        let mut ring = key_ring();
        let new_key = other_signing_key();
        let update = rotation_update("job-key-2", &new_key, KEY_ID, &[]);
        let signed = sign_key_update(&update, &signing_key());

        assert_eq!(ring.apply_key_update(&signed), Ok(()));
        assert_eq!(ring.get("job-key-2"), Some(&new_key.verifying_key()));
        assert_eq!(ring.get(KEY_ID), Some(&signing_key().verifying_key()));
    }

    #[test]
    fn ac_pr_09_rejects_key_id_overwrite() {
        let mut ring = key_ring();
        let conflicting = other_signing_key();
        let update = rotation_update(KEY_ID, &conflicting, KEY_ID, &[]);
        let signed = sign_key_update(&update, &signing_key());

        assert_eq!(
            ring.apply_key_update(&signed),
            Err(KeyUpdateError::KeyIdConflict)
        );
        assert_eq!(ring.get(KEY_ID), Some(&signing_key().verifying_key()));
    }

    #[test]
    fn ac_pr_09_same_key_is_a_no_op() {
        let mut ring = key_ring();
        let update = rotation_update(KEY_ID, &signing_key(), KEY_ID, &[]);
        let signed = sign_key_update(&update, &signing_key());

        assert_eq!(ring.apply_key_update(&signed), Ok(()));
        assert_eq!(ring.get(KEY_ID), Some(&signing_key().verifying_key()));
    }

    #[test]
    fn ac_pr_09_rejects_update_signed_by_revoked_key() {
        let mut ring = key_ring();
        let next = other_signing_key();
        let rotation = rotation_update("job-key-2", &next, KEY_ID, &[KEY_ID]);
        let signed_rotation = sign_key_update(&rotation, &signing_key());
        assert_eq!(ring.apply_key_update(&signed_rotation), Ok(()));
        assert!(ring.is_revoked(KEY_ID));

        let later = rotation_update("job-key-3", &next, KEY_ID, &[]);
        let signed_later = sign_key_update(&later, &signing_key());
        assert_eq!(
            ring.apply_key_update(&signed_later),
            Err(KeyUpdateError::RevokedSigner)
        );
        assert_eq!(ring.get("job-key-3"), None);

        let signed_job = sign_job(&job(), KEY_ID, &signing_key());
        assert_eq!(
            verify_job(&signed_job, &ring, &expected()),
            Err(no_id(RejectReason::UnknownKey))
        );
    }

    #[test]
    fn ac_pr_09_rejects_replayed_update_of_revoked_key() {
        let mut ring = key_ring();
        let next = other_signing_key();
        let rotation = rotation_update("job-key-2", &next, KEY_ID, &[KEY_ID]);
        let signed_rotation = sign_key_update(&rotation, &signing_key());
        assert_eq!(ring.apply_key_update(&signed_rotation), Ok(()));

        assert_eq!(
            ring.apply_key_update(&signed_rotation),
            Err(KeyUpdateError::RevokedSigner)
        );
        assert_eq!(ring.get("job-key-2"), Some(&next.verifying_key()));
    }

    #[test]
    fn ac_pr_09_rejects_reinsert_of_revoked_key() {
        let mut ring = key_ring();
        let next = other_signing_key();
        let rotation = rotation_update("job-key-2", &next, KEY_ID, &[KEY_ID]);
        let signed_rotation = sign_key_update(&rotation, &signing_key());
        let _ = ring.apply_key_update(&signed_rotation);

        assert_eq!(
            ring.insert(KEY_ID, signing_key().verifying_key()),
            Err(KeyUpdateError::RevokedKeyId)
        );
    }

    #[test]
    fn ac_pr_09_rejects_bad_signature() {
        let mut ring = key_ring();
        let next = other_signing_key();
        let update = rotation_update("job-key-2", &next, KEY_ID, &[]);
        let mut signed = sign_key_update(&update, &signing_key());
        signed.signature[0] ^= 0xff;

        assert_eq!(
            ring.apply_key_update(&signed),
            Err(KeyUpdateError::BadSignature)
        );
        assert_eq!(ring.get("job-key-2"), None);
        assert_eq!(ring.get(KEY_ID), Some(&signing_key().verifying_key()));
    }

    #[test]
    fn ac_pr_09_rejects_tampered_update() {
        let mut ring = key_ring();
        let next = other_signing_key();
        let update = rotation_update("job-key-2", &next, KEY_ID, &[]);
        let mut signed = sign_key_update(&update, &signing_key());

        // The bytes are not a valid KeyUpdate message. The signature check runs
        // first, so the error is BadSignature and not Malformed.
        signed.key_update = vec![0xff, 0xff, 0xff];

        assert_eq!(
            ring.apply_key_update(&signed),
            Err(KeyUpdateError::BadSignature)
        );
        assert_eq!(ring.get("job-key-2"), None);
    }

    #[test]
    fn ac_pr_09_rejects_signer_id_mismatch() {
        let mut ring = key_ring();
        let next = other_signing_key();
        let update = rotation_update("job-key-2", &next, "job-key-9", &[]);
        let mut signed = sign_key_update(&update, &signing_key());
        // The envelope names the known signer KEY_ID, so the lookup and the
        // signature check pass, but the decoded bytes name job-key-9.
        signed.signer_key_id = KEY_ID.to_owned();

        assert_eq!(
            ring.apply_key_update(&signed),
            Err(KeyUpdateError::SignerMismatch)
        );
        assert_eq!(ring.get("job-key-2"), None);
    }

    #[test]
    fn ac_pr_09_rejects_unknown_signer() {
        let mut ring = key_ring();
        let next = other_signing_key();
        let update = rotation_update("job-key-2", &next, "job-key-9", &[]);
        let signed = sign_key_update(&update, &signing_key());

        assert_eq!(
            ring.apply_key_update(&signed),
            Err(KeyUpdateError::UnknownSigner)
        );
        assert_eq!(ring.get("job-key-2"), None);
    }
}
