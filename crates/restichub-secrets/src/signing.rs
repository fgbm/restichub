//! The wrapping of the job signing keys (ADR-0026, section CA key format).
//!
//! The control plane wraps each Ed25519 job signing key with the key encryption key (KEK) with
//! AES-256-GCM, a random 96-bit nonce and the associated data
//! `restichub-signing-key-v1\0 || tenant_id || role`. The associated data binds the ciphertext to
//! one tenant and one role, so a swapped blob fails the check (T19).
//!
//! The role is the stored value of `restichub_db::SigningKeyRole` (`current` or `next`). This crate
//! holds the bytes only; the closed set lives with the storage type.
//!
//! The unwrapping asks the [`KekProvider`] for the KEK with the key ID of the wrapped key, so a key
//! that an older KEK wrapped stays readable. The plaintext key returns in a zeroizing buffer.

use uuid::Uuid;
use zeroize::Zeroizing;

use crate::NONCE_LEN;
use crate::crypto::{cipher, open, random_nonce, seal};
use crate::error::CryptoError;
use crate::kek::{Kek, KekId, KekProvider};

const SIGNING_KEY_DOMAIN: &[u8] = b"restichub-signing-key-v1\0";

/// The wrapped job signing key. The type holds no plaintext key, so a `Debug` is safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedSigningKey {
    pub kek_id: KekId,
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

/// Wraps the signing key `key` with the key encryption key `kek`.
///
/// The associated data is `restichub-signing-key-v1\0 || tenant_id || role`, with the 4 length
/// bytes of the role in big-endian order before the role bytes (ADR-0026, section CA key format).
/// `role` is the value of a `restichub_db::SigningKeyRole`.
pub fn wrap_signing_key(
    kek: &Kek,
    tenant_id: Uuid,
    role: &str,
    key: &[u8],
) -> Result<WrappedSigningKey, CryptoError> {
    let nonce = random_nonce()?;
    let cipher = cipher(kek.bytes());
    let aad = signing_aad(tenant_id, role);
    let ciphertext = seal(&cipher, nonce, key, &aad)?;
    Ok(WrappedSigningKey {
        kek_id: kek.id().clone(),
        nonce,
        ciphertext,
    })
}

/// The KEK comes from the provider by the key ID `wrapped.kek_id`.
///
/// [`CryptoError::UnknownKek`] means the provider does not hold the KEK; a changed associated data
/// gives [`CryptoError::Decrypt`]. The plaintext key returns in a zeroizing buffer.
pub fn unwrap_signing_key(
    provider: &dyn KekProvider,
    tenant_id: Uuid,
    role: &str,
    wrapped: &WrappedSigningKey,
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let kek = provider
        .by_id(&wrapped.kek_id)
        .ok_or_else(|| CryptoError::UnknownKek(wrapped.kek_id.clone()))?;
    let cipher = cipher(kek.bytes());
    let aad = signing_aad(tenant_id, role);
    let plaintext = open(&cipher, wrapped.nonce, &wrapped.ciphertext, &aad)?;
    Ok(Zeroizing::new(plaintext))
}

fn signing_aad(tenant_id: Uuid, role: &str) -> Vec<u8> {
    let role = role.as_bytes();
    let mut aad = Vec::with_capacity(SIGNING_KEY_DOMAIN.len() + 16 + 4 + role.len());
    aad.extend_from_slice(SIGNING_KEY_DOMAIN);
    aad.extend_from_slice(tenant_id.as_bytes());
    aad.extend_from_slice(&(role.len() as u32).to_be_bytes());
    aad.extend_from_slice(role);
    aad
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{signing_aad, unwrap_signing_key, wrap_signing_key};
    use crate::error::CryptoError;
    use crate::kek::{Kek, KekId, KekProvider};

    #[derive(Debug)]
    struct OneKek(Kek);

    impl KekProvider for OneKek {
        fn current(&self) -> &Kek {
            &self.0
        }

        fn by_id(&self, id: &KekId) -> Option<&Kek> {
            (self.0.id() == id).then_some(&self.0)
        }
    }

    fn kek(fill: u8) -> Kek {
        let mut bytes = [fill; 32];
        Kek::from_bytes(&mut bytes)
    }

    fn provider(fill: u8) -> OneKek {
        OneKek(kek(fill))
    }

    fn tenant() -> Uuid {
        Uuid::from_u128(0x0189_1234_5678_9abc_def0_1122_3344_5566)
    }

    #[test]
    fn signing_aad_known_answer() {
        let aad = signing_aad(tenant(), "current");
        let expected: Vec<u8> = [
            b"restichub-signing-key-v1\0".as_slice(),
            &[
                0x01, 0x89, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44,
                0x55, 0x66,
            ],
            &[0x00, 0x00, 0x00, 0x07],
            b"current".as_slice(),
        ]
        .concat();
        assert_eq!(aad, expected);
    }

    #[test]
    fn signing_aad_distinguishes_the_role() {
        assert_ne!(
            signing_aad(tenant(), "current"),
            signing_aad(tenant(), "next")
        );
    }

    #[test]
    fn signing_key_round_trip() {
        let key = kek(1);
        let material = b"ed25519-seed-placeholder-32bytes!".to_vec();
        let result = wrap_signing_key(&key, tenant(), "current", &material)
            .ok()
            .and_then(|wrapped| {
                unwrap_signing_key(&provider(1), tenant(), "current", &wrapped).ok()
            })
            .map(|plain| plain.to_vec());
        assert_eq!(result, Some(material));
    }

    #[test]
    fn unknown_kek_fails() {
        let wrapped = wrap_signing_key(&kek(1), tenant(), "next", b"key");
        let result = wrapped
            .ok()
            .as_ref()
            .map(|wrapped| unwrap_signing_key(&provider(2), tenant(), "next", wrapped));
        assert!(matches!(result, Some(Err(CryptoError::UnknownKek(_)))));
    }

    #[test]
    fn aad_swap_role_fails() {
        // A blob wrapped for the current key must not unwrap as the next key (T19).
        let wrapped = wrap_signing_key(&kek(1), tenant(), "current", b"key");
        let result = wrapped
            .ok()
            .as_ref()
            .map(|wrapped| unwrap_signing_key(&provider(1), tenant(), "next", wrapped));
        assert!(matches!(result, Some(Err(CryptoError::Decrypt(_)))));
    }

    #[test]
    fn aad_swap_tenant_id_fails() {
        let wrapped = wrap_signing_key(&kek(1), tenant(), "current", b"key");
        let other = Uuid::from_u128(1);
        let result = wrapped
            .ok()
            .as_ref()
            .map(|wrapped| unwrap_signing_key(&provider(1), other, "current", wrapped));
        assert!(matches!(result, Some(Err(CryptoError::Decrypt(_)))));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let result = wrap_signing_key(&kek(1), tenant(), "current", b"key")
            .ok()
            .map(|mut wrapped| {
                if let Some(byte) = wrapped.ciphertext.first_mut() {
                    *byte ^= 0xff;
                }
                unwrap_signing_key(&provider(1), tenant(), "current", &wrapped)
            });
        assert!(matches!(result, Some(Err(CryptoError::Decrypt(_)))));
    }
}
