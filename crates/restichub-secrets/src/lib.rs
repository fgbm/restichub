//! ResticHub secret handling.
//!
//! This crate holds the secret value type, the functions that read a secret from a file or from
//! the environment, the key encryption key (KEK) providers and the envelope encryption of the
//! secret values (ADR-0017, ADR-0023).
//!
//! The modules are:
//!
//! - [`kek`]: the KEK, the KEK providers and the key ID derivation;
//! - [`envelope`]: the data encryption key (DEK), its generation and its wrapping;
//! - [`ca`]: the wrapping of the certificate authority private keys;
//! - [`signing`]: the wrapping of the job signing keys;
//! - [`secret`]: the encryption of a secret value with the DEK of a tenant;
//! - [`check`]: the `kek_check` value and the verification that refuses a wrong KEK;
//! - [`error`]: the error types.
//!
//! The crate shows no secret material in `Display` or in `Debug`. The key types zeroize the
//! buffers on drop.
//!
//! # The environment variable `RESTICHUB_KEK`
//!
//! The environment provider reads the KEK from `RESTICHUB_KEK`. On Unix, any process of the same
//! user can read the environment of another process through `/proc`. A child process inherits the
//! environment of the control plane by default. The control plane MUST NOT spawn a child process
//! that inherits the environment while `RESTICHUB_KEK` is set. The control plane clears the
//! environment or passes an explicit, empty environment to each child process.

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use zeroize::{Zeroize, Zeroizing};

pub mod ca;
pub mod check;
pub mod envelope;
pub mod error;
pub mod kek;
pub mod mac;
pub mod secret;
pub mod signing;

mod crypto;

pub use ca::{WrappedCaKey, unwrap_ca_key, wrap_ca_key};
pub use check::{KekCheck, make_kek_check, verify_kek_check};
pub use crypto::NONCE_LEN;
pub use envelope::{DEK_LEN, Dek, WrappedDek, generate_dek, unwrap_dek, wrap_dek};
pub use error::{CryptoError, KekError};
pub use kek::{
    EnvKekProvider, FileKekProvider, KEK_ENV, KEK_FILE_PARAM, KEK_LEN, Kek, KekConfig, KekId,
    KekProvider, generate_kek_encoded, load_kek_provider,
};
pub use mac::{derive_key, hmac_sha256};
pub use secret::{
    EncryptedSecret, SecretPurpose, decrypt_job_blob, decrypt_secret, encrypt_job_blob,
    encrypt_secret,
};
pub use signing::{WrappedSigningKey, unwrap_signing_key, wrap_signing_key};

/// Hides the value in `Debug` and zeroizes it on drop.
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Deserialises a [`SecretString`] from a JSON string.
///
/// The feature `serde` enables this implementation. The type zeroizes the value on drop, so the
/// decoded value does not stay in a plain `String`. The request body bytes that `hyper` buffered
/// before the deserialiser are not zeroized; ADR-0023 records this as an accepted residual risk.
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for SecretString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self(value))
    }
}

/// The control plane reads configuration values and secrets through this trait; a test uses a
/// map instead of the process environment.
pub trait EnvReader {
    /// `None` if the variable is not set.
    fn get(&self, name: &str) -> Option<String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemEnv;

impl EnvReader for SystemEnv {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("both the secret file and the environment variable are set")]
    BothSourcesSet,
    #[error("the secret source is not set")]
    Missing,
    #[error("cannot read the secret file {path}")]
    FileRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the secret file {path} is a symbolic link")]
    Symlink { path: PathBuf },
    #[error("the secret file {path} is not a regular file")]
    NotRegularFile { path: PathBuf },
    #[error(
        "the secret file {path} is readable, writable or executable by the group or by other users"
    )]
    FilePermissions { path: PathBuf },
    #[error("the secret file {path} does not hold valid UTF-8")]
    NotUtf8 { path: PathBuf },
}

/// The source is the file when `file_path` is set, otherwise the environment variable `env_var`.
/// When both are set, the function returns [`SecretError::BothSourcesSet`].
///
/// One trailing carriage return and one trailing line feed are removed from a file value.
pub fn read_secret(
    file_path: Option<&Path>,
    env_var: &str,
    env: &dyn EnvReader,
) -> Result<SecretString, SecretError> {
    match file_path {
        Some(path) => {
            if env.get(env_var).is_some() {
                return Err(SecretError::BothSourcesSet);
            }
            read_secret_file(path)
        }
        None => match env.get(env_var) {
            Some(value) => Ok(SecretString::new(value)),
            None => Err(SecretError::Missing),
        },
    }
}

/// The file is opened one time and read from the same handle. A symbolic link, a path that is not a
/// regular file and a file whose permissions are wider than the owner (`mode & 0o077 != 0`) on Unix
/// are rejected.
///
/// Crate-internal; the KEK provider uses it.
pub(crate) fn read_secret_file(path: &Path) -> Result<SecretString, SecretError> {
    check_not_symlink(path)?;
    let file = open_readonly(path)?;
    let metadata = file.metadata().map_err(|source| SecretError::FileRead {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(SecretError::NotRegularFile {
            path: path.to_path_buf(),
        });
    }
    check_mode(&metadata, path)?;
    read_all(file, path)
}

/// Refuses to follow a symbolic link on Unix (`O_NOFOLLOW`).
#[cfg(unix)]
fn open_readonly(path: &Path) -> Result<File, SecretError> {
    use std::os::unix::fs::OpenOptionsExt;

    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|source| SecretError::FileRead {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(not(unix))]
fn open_readonly(path: &Path) -> Result<File, SecretError> {
    File::open(path).map_err(|source| SecretError::FileRead {
        path: path.to_path_buf(),
        source,
    })
}

fn check_not_symlink(path: &Path) -> Result<(), SecretError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| SecretError::FileRead {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(SecretError::Symlink {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn check_mode(metadata: &std::fs::Metadata, path: &Path) -> Result<(), SecretError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode();
    // Group and other bits are 0o077.
    if mode & 0o077 != 0 {
        return Err(SecretError::FilePermissions {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

/// Does nothing on a non-Unix platform.
#[cfg(not(unix))]
fn check_mode(_metadata: &std::fs::Metadata, _path: &Path) -> Result<(), SecretError> {
    Ok(())
}

fn read_all(mut file: File, path: &Path) -> Result<SecretString, SecretError> {
    let mut buffer = Zeroizing::new(Vec::new());
    file.read_to_end(&mut buffer)
        .map_err(|source| SecretError::FileRead {
            path: path.to_path_buf(),
            source,
        })?;
    if buffer.last() == Some(&b'\n') {
        buffer.pop();
    }
    if buffer.last() == Some(&b'\r') {
        buffer.pop();
    }
    let bytes = std::mem::take(&mut *buffer);
    match String::from_utf8(bytes) {
        Ok(value) => Ok(SecretString::new(value)),
        Err(error) => {
            let mut bytes = error.into_bytes();
            bytes.zeroize();
            Err(SecretError::NotUtf8 {
                path: path.to_path_buf(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::{EnvReader, SecretError, SecretString, read_secret};

    struct MapEnv(HashMap<String, String>);

    impl MapEnv {
        fn empty() -> Self {
            Self(HashMap::new())
        }

        fn with(name: &str, value: &str) -> Self {
            let mut map = HashMap::new();
            map.insert(name.to_owned(), value.to_owned());
            Self(map)
        }
    }

    impl EnvReader for MapEnv {
        fn get(&self, name: &str) -> Option<String> {
            self.0.get(name).cloned()
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("restichub-secrets-{}-{name}", std::process::id()));
        path
    }

    #[test]
    fn debug_hides_the_value() {
        let secret = SecretString::new("hunter2".to_owned());
        assert_eq!(format!("{secret:?}"), "<redacted>");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn a_secret_string_deserialises_from_a_json_string() {
        let secret: SecretString =
            serde_json::from_str("\"hunter2\"").unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(secret.expose_secret(), "hunter2");
    }

    #[test]
    fn env_source_reads_the_value() {
        let env = MapEnv::with("SECRET", "from-env");
        let secret = read_secret(None, "SECRET", &env);
        assert_eq!(
            secret.ok().as_ref().map(SecretString::expose_secret),
            Some("from-env")
        );
    }

    #[test]
    fn missing_source_is_an_error() {
        let env = MapEnv::empty();
        let error = read_secret(None, "SECRET", &env);
        assert!(matches!(error, Err(SecretError::Missing)));
    }

    #[test]
    fn both_sources_are_an_error() {
        let path = temp_path("both");
        std::fs::write(&path, "from-file").ok();
        let env = MapEnv::with("SECRET", "from-env");
        let error = read_secret(Some(&path), "SECRET", &env);
        std::fs::remove_file(&path).ok();
        assert!(matches!(error, Err(SecretError::BothSourcesSet)));
    }

    #[test]
    fn file_source_reads_the_value() {
        let env = MapEnv::empty();
        let path = temp_path("file");
        std::fs::write(&path, "from-file\n").ok();
        permission(path.clone(), 0o600);
        let secret = read_secret(Some(&path), "SECRET", &env);
        std::fs::remove_file(&path).ok();
        assert_eq!(
            secret.ok().as_ref().map(SecretString::expose_secret),
            Some("from-file")
        );
    }

    #[cfg(unix)]
    #[test]
    fn group_readable_file_is_rejected() {
        let env = MapEnv::empty();
        let path = temp_path("wide");
        std::fs::write(&path, "from-file").ok();
        permission(path.clone(), 0o644);
        let secret = read_secret(Some(&path), "SECRET", &env);
        std::fs::remove_file(&path).ok();
        assert!(matches!(secret, Err(SecretError::FilePermissions { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn mode_0620_is_rejected() {
        let path = temp_path("mode-620");
        std::fs::write(&path, "from-file").ok();
        permission(path.clone(), 0o620);
        let secret = super::read_secret_file(&path);
        std::fs::remove_file(&path).ok();
        assert!(matches!(secret, Err(SecretError::FilePermissions { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn mode_0604_is_rejected() {
        let path = temp_path("mode-604");
        std::fs::write(&path, "from-file").ok();
        permission(path.clone(), 0o604);
        let secret = super::read_secret_file(&path);
        std::fs::remove_file(&path).ok();
        assert!(matches!(secret, Err(SecretError::FilePermissions { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn mode_0600_is_accepted() {
        let path = temp_path("mode-600");
        std::fs::write(&path, "from-file").ok();
        permission(path.clone(), 0o600);
        let secret = super::read_secret_file(&path);
        std::fs::remove_file(&path).ok();
        assert_eq!(
            secret.ok().as_ref().map(SecretString::expose_secret),
            Some("from-file")
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let target = temp_path("symlink-target");
        let link = temp_path("symlink-link");
        std::fs::write(&target, "from-file").ok();
        permission(target.clone(), 0o600);
        symlink(&target, &link).ok();
        let secret = super::read_secret_file(&link);
        std::fs::remove_file(&link).ok();
        std::fs::remove_file(&target).ok();
        assert!(matches!(secret, Err(SecretError::Symlink { .. })));
    }

    #[cfg(unix)]
    fn permission(path: PathBuf, mode: u32) {
        use std::os::unix::fs::PermissionsExt;

        if let Ok(metadata) = std::fs::metadata(&path) {
            let mut permissions = metadata.permissions();
            permissions.set_mode(mode);
            std::fs::set_permissions(&path, permissions).ok();
        }
    }

    #[cfg(not(unix))]
    fn permission(_path: PathBuf, _mode: u32) {}
}
