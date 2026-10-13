//! The internal authenticated encryption helpers (ADR-0017, ADR-0023).
//!
//! The module holds the cipher construction, the nonce generation and the associated data of the
//! three encrypted values of the crate: the wrapped data encryption key, the secret value and the
//! `kek_check` value. The module is crate-internal.

use aes_gcm::aead::{Aead, Nonce, Payload};
use aes_gcm::{Aes256Gcm, Key, KeyInit};
use zeroize::{ZeroizeOnDrop, Zeroizing};

use crate::error::CryptoError;

/// AES-GCM nonce length in bytes.
pub const NONCE_LEN: usize = 12;

/// AES-256 key length in bytes.
pub(crate) const KEY_LEN: usize = 32;

/// Builds the AES-256-GCM cipher for a 32-byte key.
///
/// The function copies the key into a zeroizing buffer, so the working copy is zeroized when it
/// leaves the scope. The `aes-gcm` crate with the `zeroize` feature zeroizes the state of the
/// cipher on drop.
pub(crate) fn cipher(key: &[u8; KEY_LEN]) -> Aes256Gcm {
    let key = Zeroizing::new(*key);
    Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key))
}

/// Asserts at compile time that the cipher zeroizes its state on drop.
///
/// The assertion holds while the crate enables the `zeroize` feature of `aes-gcm`.
fn assert_zeroizes_on_drop<T: ZeroizeOnDrop>() {}

/// Empty and never called; it exists for the compile-time assertion.
#[allow(dead_code)]
fn cipher_zeroizes_on_drop() {
    assert_zeroizes_on_drop::<Aes256Gcm>();
}

pub(crate) fn random_nonce() -> Result<[u8; NONCE_LEN], CryptoError> {
    let mut nonce = [0_u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(CryptoError::Random)?;
    Ok(nonce)
}

pub(crate) fn seal(
    cipher: &Aes256Gcm,
    nonce: [u8; NONCE_LEN],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let nonce = Nonce::<Aes256Gcm>::from(nonce);
    cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(CryptoError::Encrypt)
}

pub(crate) fn open(
    cipher: &Aes256Gcm,
    nonce: [u8; NONCE_LEN],
    ciphertext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let nonce = Nonce::<Aes256Gcm>::from(nonce);
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(CryptoError::Decrypt)
}
