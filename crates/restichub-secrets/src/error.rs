//! Error types of the cryptographic operations.
//!
//! The crate has two error types. [`KekError`] covers the key encryption key (KEK) provider and
//! the KEK check. [`CryptoError`] covers the authenticated encryption of the data encryption keys
//! (DEK) and of the secret values.
//!
//! Neither type holds secret material. A `Debug` or a `Display` of an error never shows a key, a
//! plaintext or a base64 value. The error types show a path, a parameter name or a key ID, because
//! the operator needs the cause of a start refusal.

use crate::SecretError;
use crate::kek::KekId;

/// The error of a key encryption key (KEK) operation.
///
/// The variant [`KekError::Mismatch`] is the cause of the start refusal of the control plane
/// (AC-RP-02).
#[derive(Debug, thiserror::Error)]
pub enum KekError {
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error(
        "the parameter `kek_file` and the environment variable `RESTICHUB_KEK` are both set; set only one"
    )]
    BothSourcesSet,
    #[error(
        "the parameter `kek_file` and the environment variable `RESTICHUB_KEK` are not set; set one"
    )]
    MissingSource,
    #[error("the key ID `{value}` is not 16 lowercase hexadecimal characters")]
    BadKekId {
        /// Key IDs are not secret material.
        value: String,
    },
    #[error("the parameter `{name}` does not hold valid base64")]
    BadEncoding { name: &'static str },
    #[error(
        "the parameter `{name}` must hold 32 bytes after the base64 decoding, but it holds {actual}"
    )]
    BadLength { name: &'static str, actual: usize },
    /// The loaded KEK is not the KEK of the `kek_check` row.
    ///
    /// The control plane does not start with an incorrect KEK and shows this cause (AC-RP-02).
    #[error(
        "the loaded key encryption key `{actual_kek_id}` does not match the key encryption key of the database `{expected_kek_id}`; the control plane does not start with an incorrect key encryption key"
    )]
    Mismatch {
        expected_kek_id: KekId,
        actual_kek_id: KekId,
    },
    #[error(
        "the kek_check value of the key encryption key `{kek_id}` is corrupt; the control plane does not start"
    )]
    CheckCorrupt { kek_id: KekId },
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("the operating system random number generator failed")]
    Random(#[source] getrandom::Error),
    /// Too long in practice.
    #[error("the authenticated encryption failed")]
    Encrypt(#[source] aes_gcm::Error),
    #[error(
        "the authenticated decryption failed: the ciphertext or the associated data is not valid"
    )]
    Decrypt(#[source] aes_gcm::Error),
    #[error("the decrypted value does not have the expected length")]
    Length,
    #[error("the decrypted secret value is not valid UTF-8")]
    Utf8,
    #[error("the key encryption key `{0}` is not available")]
    UnknownKek(KekId),
    #[error("the purpose of the secret value is too long")]
    PurposeTooLong,
}
