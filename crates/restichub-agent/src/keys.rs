//! The job signing key ring and its persisted chain (ADR-0026, ADR-0030, FR-PR-11).
//!
//! The agent verifies every job and every `SigningKeyUpdate` with the key ring. The enrollment set
//! gives the current key and the next key; each accepted update adds a key and revokes the keys that
//! the control plane retired. The accepted updates form the chain that a later start replays, so an
//! agent that was offline keeps its trust set without a re-enrollment (ADR-0030).
//!
//! Only the chain file changes on an update: the agent never rewrites the private key for a key
//! update (ADR-0030). The revoked set is a union that never shrinks, and the agent applies it to
//! the ring at load, so a replayed update cannot undo a revocation (FR-PR-11).

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use prost::Message as _;
use restichub_proto::agent::v1::{KeyUpdate, SigningKeyUpdate};
use restichub_proto::signing::{KeyRing, KeyUpdateError};
use serde::{Deserialize, Serialize};

use crate::identity::StoredIdentity;

/// The chain of the accepted signed job signing key updates (ADR-0030).
pub const KEY_UPDATES_FILE: &str = "signing-key-updates.json";

/// The persisted job signing key chain (ADR-0030, FR-PR-11).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredKeyUpdates {
    /// Base64 of each accepted `SigningKeyUpdate`, in acceptance order.
    pub updates: Vec<String>,
    /// Every key identifier that the control plane retired; the set never shrinks (FR-PR-11).
    pub revoked: Vec<String>,
}

/// What one update did to the key ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// The ring holds a new key or a new revocation; the caller persists the chain.
    Changed,
    /// The ring did not change (an identical or a repeated update).
    Unchanged,
    /// The update is a chain step that the agent already applied, or a step that it cannot verify
    /// yet. It carries no new key.
    Skipped,
}

/// The live key ring with the persisted chain that produced it.
pub struct KeyState {
    ring: KeyRing,
    updates: Vec<String>,
    revoked: BTreeSet<String>,
}

impl KeyState {
    /// Builds the trust set from the enrollment and the persisted chain (ADR-0026, ADR-0030).
    ///
    /// # Errors
    ///
    /// Returns an error when the enrollment set is not valid, when the chain file cannot be read,
    /// or when a stored update does not verify. A malformed chain names a re-registration.
    pub fn load(identity: &StoredIdentity, dir: &Path) -> Result<Self> {
        let dir = crate::identity::current(dir);
        let mut ring = crate::verify::verify_key_set(
            &identity.signing_keys.keys,
            &identity.signing_keys.signature,
        )?;
        let stored = read_chain(&dir)?;
        let mut updates = Vec::with_capacity(stored.updates.len());
        for entry in &stored.updates {
            let bytes = STANDARD
                .decode(entry)
                .context("a stored key update is not base64")?;
            let update = SigningKeyUpdate::decode(bytes.as_slice())
                .context("a stored key update does not decode")?;
            ring.apply_key_update(&update)
                .map_err(|error| anyhow::anyhow!("a stored key update is not valid: {error:?}"))?;
            updates.push(entry.clone());
        }
        let revoked: BTreeSet<String> = stored.revoked.into_iter().collect();
        // The stored union is applied last, so a key that the control plane retired stays revoked
        // after a restart, even when the stored chain no longer carries the revocation (FR-PR-11).
        for key_id in &revoked {
            ring.revoke(key_id);
        }
        Ok(Self {
            ring,
            updates,
            revoked,
        })
    }

    /// The trust set that verifies a job (FR-PR-03).
    #[must_use]
    pub fn ring(&self) -> &KeyRing {
        &self.ring
    }

    /// Verifies one signed update against the ring and records it (FR-PR-11, ADR-0030).
    ///
    /// # Errors
    ///
    /// Returns an error when the update does not decode or when its signature does not verify.
    pub fn apply(&mut self, update: &SigningKeyUpdate) -> Result<Applied> {
        let inner = inner_update(update)?;
        let new_key = self.ring.get(&inner.key_id).is_none();
        match self.ring.apply_key_update(update) {
            Ok(()) => {}
            // A chain step that the agent already applied: its signer is revoked or unknown. The step
            // adds no key, so the agent skips it and keeps the ring (ADR-0030).
            Err(KeyUpdateError::UnknownSigner | KeyUpdateError::RevokedSigner) => {
                return Ok(Applied::Skipped);
            }
            Err(error) => return Err(anyhow::anyhow!("the key update is not valid: {error:?}")),
        }
        let new_revocation = inner
            .revoked_key_ids
            .iter()
            .any(|key_id| !self.revoked.contains(key_id));
        if !new_key && !new_revocation {
            return Ok(Applied::Unchanged);
        }
        let encoded = STANDARD.encode(update.encode_to_vec());
        match self
            .updates
            .iter()
            .position(|entry| introduced_key_id(entry).as_deref() == Some(inner.key_id.as_str()))
        {
            Some(index) => self.updates[index] = encoded,
            None => self.updates.push(encoded),
        }
        self.revoked.extend(inner.revoked_key_ids.iter().cloned());
        Ok(Applied::Changed)
    }

    /// Writes the chain to the configuration directory (ADR-0030).
    ///
    /// Only the chain file changes; the private key and the certificate stay untouched.
    ///
    /// # Errors
    ///
    /// Returns an error when the chain cannot be serialized or the file cannot be replaced.
    pub fn persist(&self, dir: &Path) -> Result<()> {
        let stored = StoredKeyUpdates {
            updates: self.updates.clone(),
            revoked: self.revoked.iter().cloned().collect(),
        };
        let bytes = serde_json::to_vec(&stored).context("cannot serialize the key chain")?;
        crate::paths::replace_file(&dir.join(KEY_UPDATES_FILE), &bytes)
    }

    /// Reports whether the ring holds a revoked key identifier (FR-PR-11).
    #[must_use]
    pub fn is_revoked(&self, key_id: &str) -> bool {
        self.revoked.contains(key_id)
    }
}

/// Loads the stored updates. A missing file is an empty chain; any other error names a re-registration
/// (ADR-0030).
fn read_chain(dir: &Path) -> Result<StoredKeyUpdates> {
    let dir = crate::identity::current(dir);
    let path = dir.join(KEY_UPDATES_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "the key chain {} is malformed; register the host again to replace it",
                path.display()
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(StoredKeyUpdates::default())
        }
        Err(error) => {
            Err(error).with_context(|| format!("cannot read the key chain {}", path.display()))
        }
    }
}

/// Decodes the inner `KeyUpdate` of an envelope.
fn inner_update(update: &SigningKeyUpdate) -> Result<KeyUpdate> {
    KeyUpdate::decode(update.key_update.as_slice()).context("the key update does not decode")
}

/// The key that a stored envelope introduces, or `None` when it does not decode.
fn introduced_key_id(entry: &str) -> Option<String> {
    let bytes = STANDARD.decode(entry).ok()?;
    let update = SigningKeyUpdate::decode(bytes.as_slice()).ok()?;
    Some(inner_update(&update).ok()?.key_id)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use ed25519_dalek::SigningKey;
    use restichub_proto::agent::v1::KeyUpdate;
    use restichub_proto::signing::{KeyRing, sign_key_update};

    use super::{Applied, Engine, KeyState, STANDARD, StoredKeyUpdates};

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn update_at(
        key_id: &str,
        signer: &SigningKey,
        signer_id: &str,
        revoked: &[&str],
        seconds: i64,
    ) -> restichub_proto::agent::v1::SigningKeyUpdate {
        let inner = KeyUpdate {
            key_id: key_id.to_owned(),
            public_key: signing_key(99).verifying_key().to_bytes().to_vec(),
            signer_key_id: signer_id.to_owned(),
            issued_at: Some(prost_types::Timestamp { seconds, nanos: 0 }),
            revoked_key_ids: revoked.iter().map(|id| (*id).to_owned()).collect(),
        };
        sign_key_update(&inner, signer)
    }

    /// A state whose ring already holds the signer key `k1`.
    fn state_with(k1: &SigningKey) -> KeyState {
        let mut ring = KeyRing::new();
        ring.insert("k1", k1.verifying_key()).expect("insert");
        KeyState {
            ring,
            updates: Vec::new(),
            revoked: Default::default(),
        }
    }

    #[test]
    fn m4_a_replayed_update_keeps_the_key_revoked() {
        let k1 = signing_key(1);
        let mut state = state_with(&k1);
        // A rotation at t=2 revokes k1 and introduces k2, signed by k1.
        let rotation = update_at("k2", &k1, "k1", &["k1"], 2);
        assert_eq!(state.apply(&rotation).expect("apply"), Applied::Changed);
        assert!(state.is_revoked("k1"));
        // A replay of the same rotation names a revoked signer, so the agent skips it and k1 stays
        // revoked (FR-PR-11).
        let replay = update_at("k2", &k1, "k1", &[], 1);
        assert_eq!(state.apply(&replay).expect("apply"), Applied::Skipped);
        assert!(state.is_revoked("k1"));
    }

    #[test]
    fn m3_an_identical_update_does_not_change_the_chain() {
        let k1 = signing_key(2);
        let mut state = state_with(&k1);
        let first = update_at("k2", &k1, "k1", &[], 1);
        assert_eq!(state.apply(&first).expect("apply"), Applied::Changed);
        let length = state.updates.len();
        let again = update_at("k2", &k1, "k1", &[], 1);
        assert_eq!(state.apply(&again).expect("apply"), Applied::Unchanged);
        assert_eq!(state.updates.len(), length);
    }

    #[test]
    fn l3_an_absent_chain_is_empty_and_a_bad_chain_is_named() {
        let dir = std::env::temp_dir().join(format!("restichub-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        // Absent: an empty chain.
        let stored = super::read_chain(&dir).expect("absent is empty");
        assert_eq!(stored, StoredKeyUpdates::default());
        // Malformed: an error that names a re-registration.
        std::fs::write(dir.join(super::KEY_UPDATES_FILE), b"{not json").expect("write");
        let error = super::read_chain(&dir).expect_err("malformed must fail");
        assert!(
            error.to_string().contains("register the host again"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The stored revoked union is applied to the ring at load, so a retired key stays revoked
    /// across a restart (FR-PR-11).
    #[test]
    fn l1_the_stored_revoked_union_survives_a_restart() {
        use prost::Message as _;
        use restichub_proto::signing::{EnrollmentKey, sign_enrollment_keys};

        let k1 = signing_key(1);
        let dir =
            std::env::temp_dir().join(format!("restichub-keys-revoked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");

        let keys = vec![EnrollmentKey {
            role: "current".to_owned(),
            key_id: "k1".to_owned(),
            public_key: k1.verifying_key().to_bytes().to_vec(),
        }];
        let signature = sign_enrollment_keys(&keys, &k1);
        let identity = crate::identity::StoredIdentity {
            agent_id: "agent-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            cp_url: "https://cp.example".to_owned(),
            grpc_endpoint: None,
            key_pem: String::new(),
            certificate_der: Vec::new(),
            ca_chain_der: Vec::new(),
            signing_keys: crate::identity::StoredSigningKeys {
                keys: vec![crate::enroll::EnrollmentKeyBody {
                    role: "current".to_owned(),
                    key_id: "k1".to_owned(),
                    public_key: STANDARD.encode(k1.verifying_key().to_bytes()),
                }],
                signature: STANDARD.encode(signature),
            },
        };

        // The stored chain introduced k2 and revoked k1.
        let rotation = update_at("k2", &k1, "k1", &["k1"], 2);
        let stored = StoredKeyUpdates {
            updates: vec![STANDARD.encode(rotation.encode_to_vec())],
            revoked: vec!["k1".to_owned()],
        };
        std::fs::write(
            dir.join(super::KEY_UPDATES_FILE),
            serde_json::to_vec(&stored).expect("json"),
        )
        .expect("write");

        let state = KeyState::load(&identity, &dir).expect("the chain must load");
        assert!(
            state.ring().get("k1").is_none(),
            "a revoked key stays revoked"
        );
        assert!(state.ring().get("k2").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
