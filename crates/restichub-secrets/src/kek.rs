//! The key encryption key (KEK) and the KEK providers (ADR-0017, ADR-0023).
//!
//! A KEK is 32 random bytes from the operating system random number generator. The control plane
//! stores the KEK outside the database, in a file with mode 0600 or in the environment variable
//! [`KEK_ENV`]. The KEK encrypts the data encryption key (DEK) of each tenant.
//!
//! Each KEK has a key ID. The crate derives the key ID from the KEK. The derivation is the first
//! 16 hexadecimal characters of the SHA-256 digest of a fixed domain string followed by the KEK
//! bytes. A change of the KEK changes the key ID. The derivation does not disclose the KEK,
//! because SHA-256 is a one-way function.
//!
//! The file provider opens the file one time with `O_NOFOLLOW` on Unix, refuses a symbolic link,
//! refuses a path that is not a regular file and refuses a file with the group or the other
//! permission bits set (`mode & 0o077 != 0`).

use std::fmt;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::error::{CryptoError, KekError};
use crate::{EnvReader, SecretString, read_secret_file};

/// The name of the environment variable that holds the KEK.
pub const KEK_ENV: &str = "RESTICHUB_KEK";

/// The name of the configuration parameter that holds the path of the KEK file.
pub const KEK_FILE_PARAM: &str = "kek_file";

/// The length of a KEK in bytes.
pub const KEK_LEN: usize = 32;

/// The domain string of the key ID derivation.
const KEK_ID_DOMAIN: &[u8] = b"restichub-kek-id-v1";

/// The key ID of a KEK.
///
/// The key ID is 16 lowercase hexadecimal characters. The type shows the value in `Debug`, because
/// the key ID is not secret material.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KekId(String);

impl KekId {
    /// Parses a key ID from 16 lowercase hexadecimal characters.
    ///
    /// The function returns [`KekError::BadKekId`] for another value.
    pub fn parse(value: &str) -> Result<Self, KekError> {
        let valid = value.len() == 16
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(KekError::BadKekId {
                value: value.to_owned(),
            })
        }
    }

    /// Returns the key ID as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KekId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A key encryption key.
///
/// The type is 32 bytes. The type zeroizes the value on drop and shows `<redacted>` in `Debug`.
pub struct Kek {
    /// The KEK bytes.
    bytes: [u8; KEK_LEN],
    /// The derived key ID.
    id: KekId,
}

impl Kek {
    /// Creates a KEK from 32 bytes and derives the key ID.
    ///
    /// The function copies the bytes into the KEK and zeroizes the source array.
    pub fn from_bytes(bytes: &mut [u8; KEK_LEN]) -> Self {
        let id = derive_kek_id(bytes);
        let stored = *bytes;
        bytes.zeroize();
        Self { bytes: stored, id }
    }

    /// Returns the key ID.
    pub fn id(&self) -> &KekId {
        &self.id
    }

    /// Returns the KEK bytes.
    ///
    /// The function is crate-internal, so the exact key never leaves the crate.
    pub(crate) fn bytes(&self) -> &[u8; KEK_LEN] {
        &self.bytes
    }
}

impl fmt::Debug for Kek {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

impl Drop for Kek {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}

/// Derives the key ID of a KEK.
///
/// The key ID is the first 16 hexadecimal characters of the SHA-256 digest of [`KEK_ID_DOMAIN`]
/// followed by the KEK bytes.
fn derive_kek_id(key: &[u8; KEK_LEN]) -> KekId {
    let mut hasher = Sha256::new();
    hasher.update(KEK_ID_DOMAIN);
    hasher.update(key);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(16);
    for byte in &digest[..8] {
        // A write to a `String` cannot fail.
        let _ = write!(hex, "{byte:02x}");
    }
    KekId(hex)
}

/// A source of key encryption keys.
///
/// The trait is not asynchronous, so a provider can be used from a synchronous or an asynchronous
/// caller. The method [`KekProvider::by_id`] prepares the rotation of the KEK: a provider can hold
/// the current KEK and the previous KEKs. The trait requires `Debug`, so a log line can show the
/// key ID without the key.
pub trait KekProvider: Send + Sync + fmt::Debug {
    /// Returns the current KEK.
    fn current(&self) -> &Kek;

    /// Returns the KEK with the key ID `id`.
    ///
    /// The function returns `None` if this provider does not hold this KEK.
    fn by_id(&self, id: &KekId) -> Option<&Kek>;
}

/// The file or environment source of the KEK.
///
/// The path is optional. If the path is set, the KEK comes from the file. If the path is not set,
/// the KEK comes from the environment variable [`KEK_ENV`].
#[derive(Debug, Default, Clone)]
pub struct KekConfig {
    /// The path of the KEK file.
    pub file_path: Option<PathBuf>,
}

/// Loads the KEK provider from the configuration.
///
/// The function selects the file provider if the configuration sets a path. The function selects
/// the environment provider if the configuration sets no path.
///
/// The function returns an error in these cases:
///
/// - the configuration sets a path and the environment variable [`KEK_ENV`] is also set, with
///   [`KekError::BothSourcesSet`];
/// - the configuration sets no path and the environment variable [`KEK_ENV`] is not set, with
///   [`KekError::MissingSource`];
/// - the value is not valid base64 or it is not 32 bytes.
///
/// The error names the parameter. The error never holds the value.
pub fn load_kek_provider(
    config: &KekConfig,
    env: &dyn EnvReader,
) -> Result<Box<dyn KekProvider>, KekError> {
    match &config.file_path {
        Some(path) => Ok(Box::new(FileKekProvider::load(path, env)?)),
        None => Ok(Box::new(EnvKekProvider::load(env)?)),
    }
}

/// The KEK provider that reads a file.
///
/// The provider obeys the same file rules as the database password: the file is a regular file,
/// it is not a symbolic link, and its permissions are no wider than the owner on Unix. The
/// provider also refuses a file when the environment variable [`KEK_ENV`] is set, because the
/// operator must set one source only.
pub struct FileKekProvider {
    /// The KEK.
    kek: Kek,
}

impl FileKekProvider {
    /// Loads the KEK from the file at `file_path`.
    pub fn load(file_path: &Path, env: &dyn EnvReader) -> Result<Self, KekError> {
        if env.get(KEK_ENV).is_some() {
            return Err(KekError::BothSourcesSet);
        }
        let text = read_secret_file(file_path)?;
        let kek = parse_kek(text.expose_secret(), KEK_FILE_PARAM)?;
        Ok(Self { kek })
    }
}

impl KekProvider for FileKekProvider {
    fn current(&self) -> &Kek {
        &self.kek
    }

    fn by_id(&self, id: &KekId) -> Option<&Kek> {
        (self.kek.id() == id).then_some(&self.kek)
    }
}

impl fmt::Debug for FileKekProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileKekProvider")
            .field("kek", &self.kek)
            .finish()
    }
}

/// The KEK provider that reads the environment variable [`KEK_ENV`].
pub struct EnvKekProvider {
    /// The KEK.
    kek: Kek,
}

impl EnvKekProvider {
    /// Loads the KEK from the environment variable [`KEK_ENV`].
    pub fn load(env: &dyn EnvReader) -> Result<Self, KekError> {
        let value = env.get(KEK_ENV).ok_or(KekError::MissingSource)?;
        Self::from_value(&value)
    }

    /// Builds the provider from a base64 value.
    fn from_value(value: &str) -> Result<Self, KekError> {
        let kek = parse_kek(value, KEK_ENV)?;
        Ok(Self { kek })
    }
}

impl KekProvider for EnvKekProvider {
    fn current(&self) -> &Kek {
        &self.kek
    }

    fn by_id(&self, id: &KekId) -> Option<&Kek> {
        (self.kek.id() == id).then_some(&self.kek)
    }
}

impl fmt::Debug for EnvKekProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnvKekProvider")
            .field("kek", &self.kek)
            .finish()
    }
}

/// Decodes a base64 KEK value and checks the length.
///
/// The function names the parameter `name` in the error. The function never puts the value in the
/// error. The decoded bytes are zeroized after the copy into the KEK.
fn parse_kek(encoded: &str, name: &'static str) -> Result<Kek, KekError> {
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| KekError::BadEncoding { name })?;
    let decoded = Zeroizing::new(decoded);
    if decoded.len() != KEK_LEN {
        return Err(KekError::BadLength {
            name,
            actual: decoded.len(),
        });
    }
    let mut bytes = [0_u8; KEK_LEN];
    bytes.copy_from_slice(decoded.as_slice());
    Ok(Kek::from_bytes(&mut bytes))
}

/// Returns a new random KEK as base64 text in a zeroizing string.
///
/// The future setup command uses this function. The string zeroizes on drop, so the operator can
/// write the value to a file with mode 0600 or to the environment.
pub fn generate_kek_encoded() -> Result<SecretString, CryptoError> {
    let mut bytes = Zeroizing::new([0_u8; KEK_LEN]);
    getrandom::fill(&mut *bytes).map_err(CryptoError::Random)?;
    Ok(SecretString::new(STANDARD.encode(bytes.as_slice())))
}

/// Encodes a KEK as base64. A test uses this function.
#[cfg(test)]
pub(crate) fn encode_kek(kek: &Kek) -> String {
    STANDARD.encode(kek.bytes())
}

/// Parses a base64 KEK value. A test uses this function.
#[cfg(test)]
pub(crate) fn decode_kek(encoded: &str) -> Result<Kek, KekError> {
    parse_kek(encoded, KEK_FILE_PARAM)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::{
        EnvKekProvider, FileKekProvider, KEK_ENV, KEK_FILE_PARAM, Kek, KekConfig, KekError, KekId,
        KekProvider, decode_kek, encode_kek, generate_kek_encoded, load_kek_provider,
    };
    use crate::{EnvReader, SecretError};

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
        path.push(format!(
            "restichub-secrets-kek-{}-{name}",
            std::process::id()
        ));
        path
    }

    fn kek(fill: u8) -> Kek {
        let mut bytes = [fill; 32];
        Kek::from_bytes(&mut bytes)
    }

    #[test]
    fn kek_id_is_stable_and_16_hex_characters() {
        let first = kek(7);
        let again = kek(7);
        assert_eq!(first.id(), again.id());
        let id = first.id().as_str().to_owned();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(id.chars().all(|c| !c.is_ascii_uppercase()));
    }

    #[test]
    fn kek_id_changes_with_the_key() {
        assert_ne!(kek(1).id(), kek(2).id());
    }

    #[test]
    fn kek_id_parse_accepts_16_lowercase_hex() {
        let id = kek(7).id().clone();
        let parsed = KekId::parse(id.as_str());
        assert_eq!(parsed.ok().as_ref(), Some(&id));
    }

    #[test]
    fn kek_id_parse_rejects_bad_values() {
        assert!(KekId::parse("0123456789ABCDEF").is_err());
        assert!(KekId::parse("0123456789abcde").is_err());
        assert!(KekId::parse("0123456789abcdg").is_err());
        assert!(KekId::parse("").is_err());
    }

    #[test]
    fn debug_hides_the_kek() {
        assert_eq!(format!("{:?}", kek(9)), "<redacted>");
    }

    #[test]
    fn base64_round_trip() {
        let key = kek(3);
        let encoded = encode_kek(&key);
        let decoded = decode_kek(&encoded);
        assert_eq!(
            decoded.ok().as_ref().map(Kek::id),
            Some(key.id()),
            "the decoded key has the same key ID"
        );
    }

    #[test]
    fn generate_kek_encoded_is_32_bytes_and_unique() {
        let first = generate_kek_encoded();
        let second = generate_kek_encoded();
        let (Some(first), Some(second)) = (first.ok(), second.ok()) else {
            panic!("the generator must work");
        };
        assert_ne!(first.expose_secret(), second.expose_secret());
        assert!(decode_kek(first.expose_secret()).is_ok());
    }

    #[test]
    fn env_provider_reads_the_value() {
        let key = kek(4);
        let env = MapEnv::with(KEK_ENV, &encode_kek(&key));
        let provider = EnvKekProvider::load(&env);
        assert_eq!(
            provider.ok().as_ref().map(|p| p.current().id().clone()),
            Some(key.id().clone())
        );
    }

    #[test]
    fn file_provider_reads_the_value() {
        let key = kek(5);
        let path = temp_path("file");
        std::fs::write(&path, format!("{}\n", encode_kek(&key))).ok();
        permission(path.clone(), 0o600);
        let env = MapEnv::empty();
        let provider = FileKekProvider::load(&path, &env);
        std::fs::remove_file(&path).ok();
        assert_eq!(
            provider.ok().as_ref().map(|p| p.current().id().clone()),
            Some(key.id().clone())
        );
    }

    #[test]
    fn load_selects_the_environment_provider() {
        let key = kek(6);
        let env = MapEnv::with(KEK_ENV, &encode_kek(&key));
        let config = KekConfig::default();
        let provider = load_kek_provider(&config, &env);
        assert_eq!(
            provider.ok().as_ref().map(|p| p.current().id().clone()),
            Some(key.id().clone())
        );
    }

    #[test]
    fn by_id_returns_the_current_key_only_for_its_id() {
        let key = kek(8);
        let other = kek(9);
        let env = MapEnv::with(KEK_ENV, &encode_kek(&key));
        match EnvKekProvider::load(&env) {
            Ok(provider) => {
                assert!(provider.by_id(key.id()).is_some());
                assert!(provider.by_id(other.id()).is_none());
            }
            Err(error) => panic!("the provider must load: {error}"),
        }
    }

    #[test]
    fn both_sources_are_an_error_and_name_the_parameters() {
        let key = kek(10);
        let path = temp_path("both");
        std::fs::write(&path, encode_kek(&key)).ok();
        permission(path.clone(), 0o600);
        let env = MapEnv::with(KEK_ENV, &encode_kek(&key));
        let config = KekConfig {
            file_path: Some(path.clone()),
        };
        let error = load_kek_provider(&config, &env);
        std::fs::remove_file(&path).ok();
        match error {
            Err(KekError::BothSourcesSet) => {}
            other => panic!("unexpected result: {other:?}"),
        }
        let message = KekError::BothSourcesSet.to_string();
        assert!(message.contains(KEK_FILE_PARAM));
        assert!(message.contains(KEK_ENV));
    }

    #[test]
    fn no_source_is_an_error_and_names_the_parameters() {
        let env = MapEnv::empty();
        let config = KekConfig::default();
        let error = load_kek_provider(&config, &env);
        match error {
            Err(KekError::MissingSource) => {}
            other => panic!("unexpected result: {other:?}"),
        }
        let message = KekError::MissingSource.to_string();
        assert!(message.contains(KEK_FILE_PARAM));
        assert!(message.contains(KEK_ENV));
    }

    #[test]
    fn bad_length_names_the_parameter_and_hides_the_value() {
        // A 16-byte value is valid base64 but it is not a 32-byte KEK.
        let value = "AAECAwQFBgcICQoLDA0ODw==";
        match decode_kek(value) {
            Err(KekError::BadLength { name, actual }) => {
                assert_eq!(name, KEK_FILE_PARAM);
                assert_eq!(actual, 16);
            }
            other => panic!("unexpected result: {other:?}"),
        }
        let message = decode_kek(value)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(!message.contains(value));
    }

    #[test]
    fn bad_base64_names_the_parameter_and_hides_the_value() {
        let value = "not-base64!!!";
        match decode_kek(value) {
            Err(KekError::BadEncoding { name }) => assert_eq!(name, KEK_FILE_PARAM),
            other => panic!("unexpected result: {other:?}"),
        }
        let message = decode_kek(value)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default();
        assert!(!message.contains(value));
    }

    #[cfg(unix)]
    #[test]
    fn group_readable_file_is_rejected_and_hides_the_value() {
        let key = kek(11);
        let encoded = encode_kek(&key);
        let path = temp_path("wide");
        std::fs::write(&path, &encoded).ok();
        permission(path.clone(), 0o644);
        let env = MapEnv::empty();
        let error = FileKekProvider::load(&path, &env);
        std::fs::remove_file(&path).ok();
        match error {
            Err(KekError::Secret(SecretError::FilePermissions { .. })) => {}
            other => panic!("unexpected result: {other:?}"),
        }
        if let Err(error) = FileKekProvider::load(&temp_path("missing"), &env) {
            assert!(!format!("{error}").contains(&encoded));
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let key = kek(12);
        let target = temp_path("symlink-target");
        let link = temp_path("symlink-link");
        std::fs::write(&target, encode_kek(&key)).ok();
        permission(target.clone(), 0o600);
        symlink(&target, &link).ok();
        let env = MapEnv::empty();
        let error = FileKekProvider::load(&link, &env);
        std::fs::remove_file(&link).ok();
        std::fs::remove_file(&target).ok();
        assert!(matches!(
            error,
            Err(KekError::Secret(SecretError::Symlink { .. }))
        ));
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
