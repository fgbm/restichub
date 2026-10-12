//! The errors of the cryptographic operations.
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
    /// A secret read failed. The error covers the file rules and the environment.
    #[error(transparent)]
    Secret(#[from] SecretError),
    /// The parameter `kek_file` and the environment variable `RESTICHUB_KEK` are both set.
    #[error(
        "the parameter `kek_file` and the environment variable `RESTICHUB_KEK` are both set; set only one"
    )]
    BothSourcesSet,
    /// Neither the parameter `kek_file` nor the environment variable `RESTICHUB_KEK` is set.
    #[error(
        "the parameter `kek_file` and the environment variable `RESTICHUB_KEK` are not set; set one"
    )]
    MissingSource,
    /// A KEK identifier is not 16 lowercase hexadecimal characters.
    #[error("the key ID `{value}` is not 16 lowercase hexadecimal characters")]
    BadKekId {
        /// The refused value. A key ID is not secret material.
        value: String,
    },
    /// The KEK value is not valid base64.
    #[error("the parameter `{name}` does not hold valid base64")]
    BadEncoding {
        /// The name of the parameter that holds the value.
        name: &'static str,
    },
    /// The KEK is not 32 bytes after the base64 decoding.
    #[error(
        "the parameter `{name}` must hold 32 bytes after the base64 decoding, but it holds {actual}"
    )]
    BadLength {
        /// The name of the parameter that holds the value.
        name: &'static str,
        /// The number of bytes after the base64 decoding.
        actual: usize,
    },
    /// The loaded KEK is not the KEK of the `kek_check` row.
    ///
    /// The control plane does not start with an incorrect KEK and shows this cause (AC-RP-02).
    #[error(
        "the loaded key encryption key `{actual_kek_id}` does not match the key encryption key of the database `{expected_kek_id}`; the control plane does not start with an incorrect key encryption key"
    )]
    Mismatch {
        /// The key ID of the KEK in the `kek_check` row.
        expected_kek_id: KekId,
        /// The key ID of the loaded KEK.
        actual_kek_id: KekId,
    },
    /// The `kek_check` value is corrupt. The key IDs match, but the value does not decrypt.
    #[error(
        "the kek_check value of the key encryption key `{kek_id}` is corrupt; the control plane does not start"
    )]
    CheckCorrupt {
        /// The key ID of the KEK and of the `kek_check` row.
        kek_id: KekId,
    },
}

/// The error of an authenticated encryption operation.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// The operating system random number generator failed.
    #[error("the operating system random number generator failed")]
    Random(#[source] getrandom::Error),
    /// The authenticated encryption failed. The value is too long in practice.
    #[error("the authenticated encryption failed")]
    Encrypt(#[source] aes_gcm::Error),
    /// The authenticated decryption failed. The ciphertext or the associated data is not valid.
    #[error(
        "the authenticated decryption failed: the ciphertext or the associated data is not valid"
    )]
    Decrypt(#[source] aes_gcm::Error),
    /// The decrypted value does not have the expected length.
    #[error("the decrypted value does not have the expected length")]
    Length,
    /// The decrypted secret value is not valid UTF-8.
    #[error("the decrypted secret value is not valid UTF-8")]
    Utf8,
    /// The provider does not hold the KEK with the key ID of the wrapped DEK.
    #[error("the key encryption key `{0}` is not available")]
    UnknownKek(KekId),
    /// The purpose of a secret value is too long for its length prefix.
    #[error("the purpose of the secret value is too long")]
    PurposeTooLong,
}
