//! The encryption of the secret values (ADR-0017, ADR-0023).
//!
//! A secret value uses AES-256-GCM with the data encryption key (DEK) of the tenant, a random
//! 96-bit nonce and the associated data `restichub-secret-v1\0 || tenant_id || secret_id ||
//! length(purpose) || purpose`. The associated data binds a value to one tenant, to one secret
//! and to one purpose. A swap of any component makes the decryption fail.
//!
//! The plaintext is a [`SecretString`], so a `Debug` of a value hides it and the buffer is
//! zeroized on drop.

use uuid::Uuid;
use zeroize::Zeroizing;

use crate::NONCE_LEN;
use crate::SecretString;
use crate::crypto::{cipher, open, random_nonce, seal};
use crate::envelope::Dek;
use crate::error::CryptoError;

const SECRET_DOMAIN: &[u8] = b"restichub-secret-v1\0";

/// A part of the associated data. The string value of each variant is stable and MUST NOT
/// change: a change makes every stored ciphertext of that purpose undecryptable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)
)]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum SecretPurpose {
    RepositoryPassword,
    S3AccessKey,
    S3SecretKey,
    RestServerPassword,
}

impl SecretPurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RepositoryPassword => "repository_password",
            Self::S3AccessKey => "s3_access_key",
            Self::S3SecretKey => "s3_secret_key",
            Self::RestServerPassword => "rest_server_password",
        }
    }

    /// The stable strings MUST NOT change: a change makes every stored ciphertext of that purpose
    /// undecryptable. `None` means this version does not know the string.
    pub fn parse(value: &str) -> Option<Self> {
        let purpose = match value {
            "repository_password" => Self::RepositoryPassword,
            "s3_access_key" => Self::S3AccessKey,
            "s3_secret_key" => Self::S3SecretKey,
            "rest_server_password" => Self::RestServerPassword,
            _ => return None,
        };
        Some(purpose)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedSecret {
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

/// A random nonce comes from the operating system random number generator.
pub fn encrypt_secret(
    dek: &Dek,
    tenant_id: Uuid,
    secret_id: Uuid,
    purpose: SecretPurpose,
    plaintext: &SecretString,
) -> Result<EncryptedSecret, CryptoError> {
    let nonce = random_nonce()?;
    let cipher = cipher(dek.bytes());
    let aad = secret_aad(tenant_id, secret_id, purpose)?;
    let ciphertext = seal(&cipher, nonce, plaintext.expose_secret().as_bytes(), &aad)?;
    Ok(EncryptedSecret { nonce, ciphertext })
}

/// The error [`CryptoError::Decrypt`] means the DEK or the associated data changed, and
/// [`CryptoError::Utf8`] means the plaintext is not UTF-8.
pub fn decrypt_secret(
    dek: &Dek,
    tenant_id: Uuid,
    secret_id: Uuid,
    purpose: SecretPurpose,
    encrypted: &EncryptedSecret,
) -> Result<SecretString, CryptoError> {
    let cipher = cipher(dek.bytes());
    let aad = secret_aad(tenant_id, secret_id, purpose)?;
    let plaintext = open(&cipher, encrypted.nonce, &encrypted.ciphertext, &aad)?;
    let plaintext = Zeroizing::new(plaintext);
    let text = std::str::from_utf8(plaintext.as_slice()).map_err(|_| CryptoError::Utf8)?;
    Ok(SecretString::new(text.to_owned()))
}

/// The length of the purpose is written as a big-endian `u32` before the purpose bytes, so the
/// fields do not run together. The conversion of the length is checked.
fn secret_aad(
    tenant_id: Uuid,
    secret_id: Uuid,
    purpose: SecretPurpose,
) -> Result<Vec<u8>, CryptoError> {
    let purpose = purpose.as_str();
    let length = u32::try_from(purpose.len()).map_err(|_| CryptoError::PurposeTooLong)?;
    let mut aad = Vec::with_capacity(SECRET_DOMAIN.len() + 16 + 16 + 4 + purpose.len());
    aad.extend_from_slice(SECRET_DOMAIN);
    aad.extend_from_slice(tenant_id.as_bytes());
    aad.extend_from_slice(secret_id.as_bytes());
    aad.extend_from_slice(&length.to_be_bytes());
    aad.extend_from_slice(purpose.as_bytes());
    Ok(aad)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use uuid::Uuid;

    use super::{SecretPurpose, decrypt_secret, encrypt_secret, secret_aad};
    use crate::SecretString;
    use crate::envelope::Dek;
    use crate::error::CryptoError;

    fn dek() -> Dek {
        Dek::from_bytes([21; 32])
    }

    fn tenant() -> Uuid {
        Uuid::from_u128(0x0189_1234_5678_9abc_def0_1122_3344_5566)
    }

    fn secret_id() -> Uuid {
        Uuid::from_u128(0x0189_9876_5432_10fe_dcba_9876_5432_1000)
    }

    fn plaintext() -> SecretString {
        SecretString::new("s3cr3t-value".to_owned())
    }

    #[test]
    fn a_purpose_round_trips_through_its_string() {
        for purpose in [
            SecretPurpose::RepositoryPassword,
            SecretPurpose::S3AccessKey,
            SecretPurpose::S3SecretKey,
            SecretPurpose::RestServerPassword,
        ] {
            assert_eq!(SecretPurpose::parse(purpose.as_str()), Some(purpose));
        }
        assert_eq!(SecretPurpose::parse("not_a_purpose"), None);
    }

    #[test]
    fn secret_aad_known_answer() {
        let aad = secret_aad(tenant(), secret_id(), SecretPurpose::RepositoryPassword);
        let expected: Vec<u8> = [
            b"restichub-secret-v1\0".as_slice(),
            &[
                0x01, 0x89, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44,
                0x55, 0x66,
            ],
            &[
                0x01, 0x89, 0x98, 0x76, 0x54, 0x32, 0x10, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
                0x10, 0x00,
            ],
            &[0x00, 0x00, 0x00, 0x13],
            b"repository_password".as_slice(),
        ]
        .concat();
        assert_eq!(aad.ok(), Some(expected));
    }

    #[test]
    fn secret_round_trip() {
        let encrypted = encrypt_secret(
            &dek(),
            tenant(),
            secret_id(),
            SecretPurpose::RepositoryPassword,
            &plaintext(),
        );
        let result = encrypted.ok().as_ref().and_then(|encrypted| {
            decrypt_secret(
                &dek(),
                tenant(),
                secret_id(),
                SecretPurpose::RepositoryPassword,
                encrypted,
            )
            .ok()
        });
        assert_eq!(
            result.as_ref().map(SecretString::expose_secret),
            Some("s3cr3t-value")
        );
    }

    #[test]
    fn aad_swap_tenant_id_fails() {
        let encrypted = encrypt_secret(
            &dek(),
            tenant(),
            secret_id(),
            SecretPurpose::RepositoryPassword,
            &plaintext(),
        );
        let other = Uuid::from_u128(2);
        let result = encrypted.ok().as_ref().map(|encrypted| {
            decrypt_secret(
                &dek(),
                other,
                secret_id(),
                SecretPurpose::RepositoryPassword,
                encrypted,
            )
        });
        assert!(matches!(result, Some(Err(CryptoError::Decrypt(_)))));
    }

    #[test]
    fn aad_swap_secret_id_fails() {
        let encrypted = encrypt_secret(
            &dek(),
            tenant(),
            secret_id(),
            SecretPurpose::RepositoryPassword,
            &plaintext(),
        );
        let other = Uuid::from_u128(3);
        let result = encrypted.ok().as_ref().map(|encrypted| {
            decrypt_secret(
                &dek(),
                tenant(),
                other,
                SecretPurpose::RepositoryPassword,
                encrypted,
            )
        });
        assert!(matches!(result, Some(Err(CryptoError::Decrypt(_)))));
    }

    #[test]
    fn aad_swap_purpose_fails() {
        let encrypted = encrypt_secret(
            &dek(),
            tenant(),
            secret_id(),
            SecretPurpose::RepositoryPassword,
            &plaintext(),
        );
        let result = encrypted.ok().as_ref().map(|encrypted| {
            decrypt_secret(
                &dek(),
                tenant(),
                secret_id(),
                SecretPurpose::S3SecretKey,
                encrypted,
            )
        });
        assert!(matches!(result, Some(Err(CryptoError::Decrypt(_)))));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let result = encrypt_secret(
            &dek(),
            tenant(),
            secret_id(),
            SecretPurpose::RepositoryPassword,
            &plaintext(),
        )
        .ok()
        .map(|mut encrypted| {
            if let Some(byte) = encrypted.ciphertext.first_mut() {
                *byte ^= 0xff;
            }
            decrypt_secret(
                &dek(),
                tenant(),
                secret_id(),
                SecretPurpose::RepositoryPassword,
                &encrypted,
            )
        });
        assert!(matches!(result, Some(Err(CryptoError::Decrypt(_)))));
    }

    #[test]
    fn nonces_are_unique_over_ten_thousand_encryptions() {
        let mut seen = HashSet::new();
        for _ in 0..10_000 {
            match encrypt_secret(
                &dek(),
                tenant(),
                secret_id(),
                SecretPurpose::RepositoryPassword,
                &plaintext(),
            ) {
                Ok(encrypted) => assert!(seen.insert(encrypted.nonce), "a nonce repeated"),
                Err(error) => panic!("the encryption must work: {error}"),
            }
        }
    }

    #[test]
    fn canary_plaintext_is_not_in_the_ciphertext() {
        let canary = "RESTICHUB-CANARY-0f7a1b2c3d4e5f";
        let encrypted = encrypt_secret(
            &dek(),
            tenant(),
            secret_id(),
            SecretPurpose::RepositoryPassword,
            &SecretString::new(canary.to_owned()),
        );
        match encrypted {
            Ok(encrypted) => {
                let bytes = canary.as_bytes();
                let found = encrypted
                    .ciphertext
                    .windows(bytes.len())
                    .any(|window| window == bytes);
                assert!(!found, "the ciphertext must not hold the canary");
                assert!(!encrypted.ciphertext.is_empty());
            }
            Err(error) => panic!("the encryption must work: {error}"),
        }
    }
}
