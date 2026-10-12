//! ResticHub secret handling.
//!
//! This crate holds the secret value type and the functions that read a secret from a file or
//! from the environment (ADR-0023). The later task T3 adds the key encryption key (KEK) logic.

use std::fmt;
use std::path::{Path, PathBuf};

use zeroize::Zeroize;

/// A secret string.
///
/// The type hides the value in `Debug` and zeroizes the value on drop.
pub struct SecretString(String);

impl SecretString {
    /// Wraps a value.
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Returns the secret value.
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

/// Reads environment variables.
///
/// The control plane uses this trait to read configuration values and secrets. A test uses a
/// map instead of the process environment.
pub trait EnvReader {
    /// Returns the value of the variable `name`.
    ///
    /// The function returns `None` if the variable is not set.
    fn get(&self, name: &str) -> Option<String>;
}

/// The process environment.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemEnv;

impl EnvReader for SystemEnv {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// The error of a secret read.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// The file path and the environment variable are both set.
    #[error("both the secret file and the environment variable are set")]
    BothSourcesSet,
    /// No source is set.
    #[error("the secret source is not set")]
    Missing,
    /// The file cannot be read.
    #[error("cannot read the secret file {path}")]
    FileRead {
        /// The path of the file.
        path: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
    /// The file is readable by the group or by other users.
    #[error("the secret file {path} is readable by the group or by other users")]
    FilePermissions {
        /// The path of the file.
        path: PathBuf,
    },
}

/// Reads a secret value from one source.
///
/// The function reads the file if `file_path` is set. The function reads the environment variable
/// `env_var` if `file_path` is not set. If `file_path` is set and the environment variable is also
/// set, the function returns [`SecretError::BothSourcesSet`].
///
/// On Unix, the function rejects a file that is readable by the group or by other users. The
/// function removes one trailing line break from the file value.
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
            read_file(path)
        }
        None => match env.get(env_var) {
            Some(value) => Ok(SecretString::new(value)),
            None => Err(SecretError::Missing),
        },
    }
}

fn read_file(path: &Path) -> Result<SecretString, SecretError> {
    check_permissions(path)?;
    let content = std::fs::read_to_string(path).map_err(|source| SecretError::FileRead {
        path: path.to_path_buf(),
        source,
    })?;
    let value = content.strip_suffix('\n').unwrap_or(&content);
    let value = value.strip_suffix('\r').unwrap_or(value);
    Ok(SecretString::new(value.to_owned()))
}

/// Checks that no other user can read the file.
#[cfg(unix)]
fn check_permissions(path: &Path) -> Result<(), SecretError> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path).map_err(|source| SecretError::FileRead {
        path: path.to_path_buf(),
        source,
    })?;
    let mode = metadata.permissions().mode();
    // The group read bit is 0o040. The other read bit is 0o004.
    if mode & 0o044 != 0 {
        return Err(SecretError::FilePermissions {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

/// Checks the file permissions. The function does nothing on a non-Unix platform.
#[cfg(not(unix))]
fn check_permissions(_path: &Path) -> Result<(), SecretError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::{EnvReader, SecretError, SecretString, read_secret};

    /// A test environment.
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
