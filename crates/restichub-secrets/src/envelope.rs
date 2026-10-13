//! The envelope encryption of the data encryption keys (ADR-0017, ADR-0023).
//!
//! Each tenant has a data encryption key (DEK). The control plane encrypts the secret values with
//! the DEK and the DEK with the key encryption key (KEK). The wrapping uses AES-256-GCM with a
//! random 96-bit nonce and the associated data
//! `restichub-dek-v1\0 || tenant_id || key_version`. The associated data binds the wrapped DEK to
//! one tenant and to one key version. A swap of the tenant or of the key version makes the
//! unwrapping fail.
//!
//! The unwrapping asks the [`KekProvider`] for the KEK with the key ID of the wrapped DEK, so the
//! control plane can read a DEK that an older KEK wrapped.

use std::fmt;

use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::NONCE_LEN;
use crate::crypto::{KEY_LEN, cipher, open, random_nonce, seal};
use crate::error::CryptoError;
use crate::kek::{Kek, KekId, KekProvider};

/// The length of a data encryption key in bytes.
pub const DEK_LEN: usize = KEY_LEN;

const DEK_DOMAIN: &[u8] = b"restichub-dek-v1\0";

/// 32 bytes; zeroizes the value on drop and shows `<redacted>` in `Debug`.
pub struct Dek([u8; DEK_LEN]);

impl Dek {
    pub fn from_bytes(bytes: [u8; DEK_LEN]) -> Self {
        Self(bytes)
    }

    /// Crate-internal, so the exact key never leaves the crate.
    pub(crate) fn bytes(&self) -> &[u8; DEK_LEN] {
        &self.0
    }
}

impl fmt::Debug for Dek {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

impl Drop for Dek {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub fn generate_dek() -> Result<Dek, CryptoError> {
    let mut bytes = Zeroizing::new([0_u8; DEK_LEN]);
    getrandom::fill(&mut *bytes).map_err(CryptoError::Random)?;
    Ok(Dek(*bytes))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedDek {
    pub kek_id: KekId,
    /// 96-bit nonce of the wrapping.
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

/// Wraps the data encryption key `dek` with the key encryption key `kek`.
///
/// The associated data is `restichub-dek-v1\0 || tenant_id || key_version`, with the key version
/// in big-endian order.
pub fn wrap_dek(
    kek: &Kek,
    tenant_id: Uuid,
    key_version: i32,
    dek: &Dek,
) -> Result<WrappedDek, CryptoError> {
    let nonce = random_nonce()?;
    let cipher = cipher(kek.bytes());
    let aad = dek_aad(tenant_id, key_version);
    let ciphertext = seal(&cipher, nonce, dek.bytes(), &aad)?;
    Ok(WrappedDek {
        kek_id: kek.id().clone(),
        nonce,
        ciphertext,
    })
}

/// Unwraps the data encryption key of the value `wrapped`.
///
/// The function asks the provider for the KEK with the key ID `wrapped.kek_id`. The function
/// returns [`CryptoError::UnknownKek`] if the provider does not hold this KEK. The function uses
/// the same associated data as [`wrap_dek`]. The function returns [`CryptoError::Decrypt`] if the
/// associated data changed.
pub fn unwrap_dek(
    provider: &dyn KekProvider,
    tenant_id: Uuid,
    key_version: i32,
    wrapped: &WrappedDek,
) -> Result<Dek, CryptoError> {
    let kek = provider
        .by_id(&wrapped.kek_id)
        .ok_or_else(|| CryptoError::UnknownKek(wrapped.kek_id.clone()))?;
    let cipher = cipher(kek.bytes());
    let aad = dek_aad(tenant_id, key_version);
    let plaintext = open(&cipher, wrapped.nonce, &wrapped.ciphertext, &aad)?;
    let plaintext = Zeroizing::new(plaintext);
    let bytes: [u8; DEK_LEN] = plaintext
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::Length)?;
    Ok(Dek::from_bytes(bytes))
}

fn dek_aad(tenant_id: Uuid, key_version: i32) -> Vec<u8> {
    let mut aad = Vec::with_capacity(DEK_DOMAIN.len() + 16 + 4);
    aad.extend_from_slice(DEK_DOMAIN);
    aad.extend_from_slice(tenant_id.as_bytes());
    aad.extend_from_slice(&key_version.to_be_bytes());
    aad
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use uuid::Uuid;

    use super::{Dek, dek_aad, generate_dek, unwrap_dek, wrap_dek};
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
    fn dek_aad_known_answer() {
        let aad = dek_aad(tenant(), 1);
        let expected: Vec<u8> = [
            b"restichub-dek-v1\0".as_slice(),
            &[
                0x01, 0x89, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44,
                0x55, 0x66,
            ],
            &[0x00, 0x00, 0x00, 0x01],
        ]
        .concat();
        assert_eq!(aad, expected);
    }

    #[test]
    fn dek_round_trip() {
        let key = kek(1);
        let dek = Dek::from_bytes([5; 32]);
        let result = wrap_dek(&key, tenant(), 1, &dek)
            .ok()
            .and_then(|wrapped| unwrap_dek(&provider(1), tenant(), 1, &wrapped).ok())
            .map(|dek| *dek.bytes());
        assert_eq!(result, Some([5; 32]));
    }

    #[test]
    fn generated_dek_is_32_bytes_and_unique() {
        let first = generate_dek();
        let second = generate_dek();
        let both = first
            .ok()
            .as_ref()
            .zip(second.ok().as_ref())
            .map(|(first, second)| (*first.bytes(), *second.bytes()));
        let (first, second) = both.unwrap_or_else(|| panic!("the generator must work"));
        assert_ne!(first, second);
    }

    #[test]
    fn unknown_kek_fails() {
        let dek = Dek::from_bytes([6; 32]);
        let wrapped = wrap_dek(&kek(1), tenant(), 1, &dek);
        let result = wrapped
            .ok()
            .as_ref()
            .map(|wrapped| unwrap_dek(&provider(2), tenant(), 1, wrapped));
        assert!(matches!(result, Some(Err(CryptoError::UnknownKek(_)))));
    }

    #[test]
    fn aad_swap_tenant_id_fails() {
        let dek = Dek::from_bytes([7; 32]);
        let key = kek(1);
        let wrapped = wrap_dek(&key, tenant(), 1, &dek);
        let other = Uuid::from_u128(1);
        let result = wrapped
            .ok()
            .as_ref()
            .map(|wrapped| unwrap_dek(&provider(1), other, 1, wrapped));
        assert!(matches!(result, Some(Err(CryptoError::Decrypt(_)))));
    }

    #[test]
    fn aad_swap_key_version_fails() {
        let dek = Dek::from_bytes([8; 32]);
        let key = kek(1);
        let wrapped = wrap_dek(&key, tenant(), 1, &dek);
        let result = wrapped
            .ok()
            .as_ref()
            .map(|wrapped| unwrap_dek(&provider(1), tenant(), 2, wrapped));
        assert!(matches!(result, Some(Err(CryptoError::Decrypt(_)))));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let dek = Dek::from_bytes([9; 32]);
        let key = kek(1);
        let result = wrap_dek(&key, tenant(), 1, &dek).ok().map(|mut wrapped| {
            if let Some(byte) = wrapped.ciphertext.first_mut() {
                *byte ^= 0xff;
            }
            unwrap_dek(&provider(1), tenant(), 1, &wrapped)
        });
        assert!(matches!(result, Some(Err(CryptoError::Decrypt(_)))));
    }

    #[test]
    fn nonces_are_unique_over_ten_thousand_wrappings() {
        let dek = Dek::from_bytes([10; 32]);
        let key = kek(1);
        let mut seen = HashSet::new();
        for _ in 0..10_000 {
            match wrap_dek(&key, tenant(), 1, &dek) {
                Ok(wrapped) => assert!(seen.insert(wrapped.nonce), "a nonce repeated"),
                Err(error) => panic!("the wrapping must work: {error}"),
            }
        }
    }

    #[test]
    fn wrapped_dek_debug_shows_the_ciphertext_only() {
        // The wrapped DEK holds no plaintext key, so a `Debug` is safe.
        let wrapped = wrap_dek(&kek(1), tenant(), 1, &Dek::from_bytes([11; 32]));
        assert!(format!("{wrapped:?}").contains("ciphertext"));
    }

    #[test]
    fn debug_hides_the_dek() {
        assert_eq!(format!("{:?}", Dek::from_bytes([12; 32])), "<redacted>");
    }
}
