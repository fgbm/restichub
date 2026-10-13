//! The `kek_check` value (ADR-0023, section Secrets).
//!
//! The control plane stores one `kek_check` value that it encrypted with the current KEK. At
//! start the control plane decrypts the value with the loaded KEK and compares the plaintext with
//! a fixed value. A wrong KEK makes the comparison fail, and the control plane refuses to start
//! and shows the cause (AC-RP-02).
//!
//! The check encrypts a fixed, non-secret plaintext. The check therefore holds no secret value of
//! the operator.

use crate::NONCE_LEN;
use crate::crypto::{cipher, open, random_nonce, seal};
use crate::error::{CryptoError, KekError};
use crate::kek::{Kek, KekId};

const KEK_CHECK_DOMAIN: &[u8] = b"restichub-kek-check-v1\0";

const KEK_CHECK_PLAINTEXT: &[u8] = b"restichub-kek-check-plaintext-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KekCheck {
    pub kek_id: KekId,
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

/// A random nonce comes from the operating system random number generator, so the error
/// [`CryptoError::Random`] means the generator failed.
pub fn make_kek_check(kek: &Kek) -> Result<KekCheck, CryptoError> {
    let nonce = random_nonce()?;
    let cipher = cipher(kek.bytes());
    let ciphertext = seal(&cipher, nonce, KEK_CHECK_PLAINTEXT, KEK_CHECK_DOMAIN)?;
    Ok(KekCheck {
        kek_id: kek.id().clone(),
        nonce,
        ciphertext,
    })
}

/// The error message is suitable for the start refusal of the control plane (AC-RP-02).
///
/// # Errors
///
/// - [`KekError::Mismatch`] when the key ID of the loaded KEK is not the key ID of the check;
/// - [`KekError::CheckCorrupt`] when the key IDs match but the check does not decrypt or the
///   decrypted plaintext is not the fixed plaintext.
pub fn verify_kek_check(kek: &Kek, check: &KekCheck) -> Result<(), KekError> {
    let actual_kek_id = kek.id().clone();
    if actual_kek_id != check.kek_id {
        return Err(KekError::Mismatch {
            expected_kek_id: check.kek_id.clone(),
            actual_kek_id,
        });
    }

    let corrupt = || KekError::CheckCorrupt {
        kek_id: actual_kek_id.clone(),
    };
    let cipher = cipher(kek.bytes());
    let plaintext =
        open(&cipher, check.nonce, &check.ciphertext, KEK_CHECK_DOMAIN).map_err(|_| corrupt())?;
    if plaintext != KEK_CHECK_PLAINTEXT {
        return Err(corrupt());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        KEK_CHECK_DOMAIN, KEK_CHECK_PLAINTEXT, KekCheck, make_kek_check, verify_kek_check,
    };
    use crate::error::KekError;
    use crate::kek::Kek;

    fn kek(fill: u8) -> Kek {
        let mut bytes = [fill; 32];
        Kek::from_bytes(&mut bytes)
    }

    #[test]
    fn kek_check_aad_and_plaintext_known_answer() {
        assert_eq!(KEK_CHECK_DOMAIN, b"restichub-kek-check-v1\0");
        assert_eq!(KEK_CHECK_PLAINTEXT, b"restichub-kek-check-plaintext-v1");
    }

    #[test]
    fn kek_check_round_trip() {
        let key = kek(1);
        match make_kek_check(&key) {
            Ok(check) => assert!(verify_kek_check(&key, &check).is_ok()),
            Err(error) => panic!("the check must be created: {error}"),
        }
    }

    #[test]
    fn ac_rp_02_incorrect_kek_is_refused_with_the_cause() {
        let stored = kek(1);
        let loaded = kek(2);
        match make_kek_check(&stored) {
            Ok(check) => {
                match verify_kek_check(&loaded, &check) {
                    Err(KekError::Mismatch {
                        expected_kek_id,
                        actual_kek_id,
                    }) => {
                        assert_eq!(expected_kek_id, stored.id().clone());
                        assert_eq!(actual_kek_id, loaded.id().clone());
                    }
                    other => panic!("the wrong KEK must be refused: {other:?}"),
                }
                let message = verify_kek_check(&loaded, &check)
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_default();
                assert!(message.contains(stored.id().as_str()));
                assert!(message.contains(loaded.id().as_str()));
                assert!(message.contains("does not start"));
            }
            Err(error) => panic!("the check must be created: {error}"),
        }
    }

    #[test]
    fn same_kek_id_with_a_tampered_ciphertext_is_corrupt() {
        let key = kek(1);
        match make_kek_check(&key) {
            Ok(check) => {
                let mut tampered = KekCheck {
                    kek_id: check.kek_id.clone(),
                    nonce: check.nonce,
                    ciphertext: check.ciphertext.clone(),
                };
                if let Some(byte) = tampered.ciphertext.first_mut() {
                    *byte ^= 0xff;
                }
                match verify_kek_check(&key, &tampered) {
                    Err(KekError::CheckCorrupt { kek_id }) => assert_eq!(kek_id, key.id().clone()),
                    other => panic!("the corrupt check must be refused: {other:?}"),
                }
            }
            Err(error) => panic!("the check must be created: {error}"),
        }
    }
}
