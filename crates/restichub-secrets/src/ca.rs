//! The wrapping of the certificate authority private keys (ADR-0026, section CA key format).
//!
//! The installation root CA and each issuing CA have a private key. The control plane wraps each
//! key with the key encryption key (KEK) with AES-256-GCM, a random 96-bit nonce and the
//! associated data `restichub-ca-key-v1\0 || tenant_id || role`. The associated data binds the
//! ciphertext to one tenant and one role, so a swapped blob fails the check (T19).
//!
//! The role is the stored value of `restichub_db::CaRole` (`root` or `issuing`). This crate holds
//! the bytes only; the closed set lives with the storage type.
//!
//! The unwrapping asks the [`KekProvider`] for the KEK with the key ID of the wrapped key, so a
//! key that an older KEK wrapped stays readable. The plaintext key returns in a zeroizing buffer.

use uuid::Uuid;
use zeroize::Zeroizing;

use crate::NONCE_LEN;
use crate::crypto::{cipher, open, random_nonce, seal};
use crate::error::CryptoError;
use crate::kek::{Kek, KekId, KekProvider};

const CA_KEY_DOMAIN: &[u8] = b"restichub-ca-key-v1\0";

/// The wrapped private key. The type holds no plaintext key, so a `Debug` is safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedCaKey {
    pub kek_id: KekId,
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

/// Wraps the private key `key_der` with the key encryption key `kek`.
///
/// The associated data is `restichub-ca-key-v1\0 || tenant_id || role`, with the 4 length bytes of
/// the role in big-endian order before the role bytes (ADR-0026, section CA key format). `role` is
/// the value of a `restichub_db::CaRole`.
pub fn wrap_ca_key(
    kek: &Kek,
    tenant_id: Uuid,
    role: &str,
    key_der: &[u8],
) -> Result<WrappedCaKey, CryptoError> {
    let nonce = random_nonce()?;
    let cipher = cipher(kek.bytes());
    let aad = ca_aad(tenant_id, role);
    let ciphertext = seal(&cipher, nonce, key_der, &aad)?;
    Ok(WrappedCaKey {
        kek_id: kek.id().clone(),
        nonce,
        ciphertext,
    })
}

/// The KEK comes from the provider by the key ID `wrapped.kek_id`.
///
/// [`CryptoError::UnknownKek`] means the provider does not hold the KEK; a changed associated data
/// gives [`CryptoError::Decrypt`]. The plaintext key returns in a zeroizing buffer.
pub fn unwrap_ca_key(
    provider: &dyn KekProvider,
    tenant_id: Uuid,
    role: &str,
    wrapped: &WrappedCaKey,
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let kek = provider
        .by_id(&wrapped.kek_id)
        .ok_or_else(|| CryptoError::UnknownKek(wrapped.kek_id.clone()))?;
    let cipher = cipher(kek.bytes());
    let aad = ca_aad(tenant_id, role);
    let plaintext = open(&cipher, wrapped.nonce, &wrapped.ciphertext, &aad)?;
    Ok(Zeroizing::new(plaintext))
}

fn ca_aad(tenant_id: Uuid, role: &str) -> Vec<u8> {
    let role = role.as_bytes();
    let mut aad = Vec::with_capacity(CA_KEY_DOMAIN.len() + 16 + 4 + role.len());
    aad.extend_from_slice(CA_KEY_DOMAIN);
    aad.extend_from_slice(tenant_id.as_bytes());
    aad.extend_from_slice(&(role.len() as u32).to_be_bytes());
    aad.extend_from_slice(role);
    aad
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{ca_aad, unwrap_ca_key, wrap_ca_key};
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
    fn ca_aad_known_answer() {
        let aad = ca_aad(tenant(), "root");
        let expected: Vec<u8> = [
            b"restichub-ca-key-v1\0".as_slice(),
            &[
                0x01, 0x89, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44,
                0x55, 0x66,
            ],
            &[0x00, 0x00, 0x00, 0x04],
            b"root".as_slice(),
        ]
        .concat();
        assert_eq!(aad, expected);
    }

    #[test]
    fn ca_aad_distinguishes_the_role() {
        assert_ne!(ca_aad(tenant(), "root"), ca_aad(tenant(), "issuing"));
    }

    #[test]
    fn ca_key_round_trip() {
        let key = kek(1);
        let der = b"pkcs8-der-placeholder".to_vec();
        let result = wrap_ca_key(&key, tenant(), "root", &der)
            .ok()
            .and_then(|wrapped| unwrap_ca_key(&provider(1), tenant(), "root", &wrapped).ok())
            .map(|plain| plain.to_vec());
        assert_eq!(result, Some(der));
    }

    #[test]
    fn unknown_kek_fails() {
        let wrapped = wrap_ca_key(&kek(1), tenant(), "issuing", b"key");
        let result = wrapped
            .ok()
            .as_ref()
            .map(|wrapped| unwrap_ca_key(&provider(2), tenant(), "issuing", wrapped));
        assert!(matches!(result, Some(Err(CryptoError::UnknownKek(_)))));
    }

    #[test]
    fn aad_swap_role_fails() {
        // A blob wrapped for the root CA must not unwrap as an issuing CA key (T19).
        let wrapped = wrap_ca_key(&kek(1), tenant(), "root", b"key");
        let result = wrapped
            .ok()
            .as_ref()
            .map(|wrapped| unwrap_ca_key(&provider(1), tenant(), "issuing", wrapped));
        assert!(matches!(result, Some(Err(CryptoError::Decrypt(_)))));
    }

    #[test]
    fn aad_swap_tenant_id_fails() {
        let wrapped = wrap_ca_key(&kek(1), tenant(), "root", b"key");
        let other = Uuid::from_u128(1);
        let result = wrapped
            .ok()
            .as_ref()
            .map(|wrapped| unwrap_ca_key(&provider(1), other, "root", wrapped));
        assert!(matches!(result, Some(Err(CryptoError::Decrypt(_)))));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let result = wrap_ca_key(&kek(1), tenant(), "root", b"key")
            .ok()
            .map(|mut wrapped| {
                if let Some(byte) = wrapped.ciphertext.first_mut() {
                    *byte ^= 0xff;
                }
                unwrap_ca_key(&provider(1), tenant(), "root", &wrapped)
            });
        assert!(matches!(result, Some(Err(CryptoError::Decrypt(_)))));
    }
}
