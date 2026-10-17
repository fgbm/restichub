//! The stored agent identity and the key pair (ADR-0026, FR-AG-04).

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use rcgen::{CertificateParams, KeyPair, PKCS_ECDSA_P256_SHA256};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use x509_parser::prelude::FromDer as _;

use crate::{paths, pem};

pub const KEY_FILE: &str = "agent-key.pem";
pub const CERT_FILE: &str = "agent-cert.pem";
pub const CA_CHAIN_FILE: &str = "ca-chain.pem";
pub const METADATA_FILE: &str = "identity.json";
pub const SIGNING_KEYS_FILE: &str = "signing-keys.json";

pub(crate) const IDENTITY_FILES: [&str; 5] = [
    KEY_FILE,
    CERT_FILE,
    CA_CHAIN_FILE,
    METADATA_FILE,
    SIGNING_KEYS_FILE,
];
/// The suffix of a file that a certificate rotation staged and did not yet move into place.
pub(crate) const STAGED_SUFFIX: &str = ".next";
/// The staged marker. Its presence says that the staged key, certificate and chain are complete.
pub(crate) const ROTATION_MARKER: &str = "rotation.next";
/// The files of a rotation that a load moves into place.
const ROTATED_FILES: [&str; 3] = [KEY_FILE, CERT_FILE, CA_CHAIN_FILE];

/// The names in an identity directory whose content the service trusts, with whether the name
/// holds the private key: what a load reads or moves into place, the key updates that widen the
/// key ring, the restic record and the job journal.
pub(crate) fn trusted_names() -> Vec<(String, bool)> {
    let mut names: Vec<(String, bool)> = IDENTITY_FILES
        .iter()
        .map(|file| ((*file).to_owned(), *file == KEY_FILE))
        .collect();
    names.extend(
        ROTATED_FILES
            .iter()
            .map(|file| (format!("{file}{STAGED_SUFFIX}"), *file == KEY_FILE)),
    );
    names.push((ROTATION_MARKER.to_owned(), false));
    names.extend(
        [
            crate::keys::KEY_UPDATES_FILE,
            crate::restic_store::RECORD_FILE,
            crate::journal::JOURNAL_FILE,
        ]
        .map(|file| (file.to_owned(), false)),
    );
    names
}

/// A rotation holds the lock for milliseconds. A holder past this time is not the agent, and a
/// hang would keep the service from starting.
const LOCK_WAIT: Duration = Duration::from_secs(60);

/// The public job signing key set that the identity keeps (ADR-0026).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSigningKeys {
    /// Current entry first, then the next entry.
    pub keys: Vec<crate::enroll::EnrollmentKeyBody>,
    /// Base64 of the Ed25519 value over the set (standard alphabet).
    pub signature: String,
}

/// The identity that the agent keeps between runs (FR-AG-04).
#[derive(Debug)]
pub struct StoredIdentity {
    pub agent_id: String,
    pub tenant_id: String,
    pub cp_url: String,
    pub grpc_endpoint: Option<String>,
    pub key_pem: String,
    pub certificate_der: Vec<u8>,
    pub ca_chain_der: Vec<Vec<u8>>,
    pub signing_keys: StoredSigningKeys,
}

#[derive(Debug, Serialize, Deserialize)]
struct Metadata {
    agent_id: String,
    tenant_id: String,
    cp_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    grpc_endpoint: Option<String>,
}

impl StoredIdentity {
    /// Writes the identity files into `dir` with the owner-only permissions (FR-AG-04).
    ///
    /// The files are staged in a sibling directory and the whole directory then replaces the old one
    /// with one rename. A failure before the rename leaves the old identity intact. The last file is
    /// `identity.json`, so its presence marks a complete identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the staging directory cannot be written or the swap fails.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let staging = sibling(dir, ".staging");
        let _ = std::fs::remove_dir_all(&staging);
        paths::ensure_config_dir(&staging)?;
        // The first registration has no directory to lock yet.
        let _lock = if lock_path(dir).parent().is_some_and(Path::is_dir) {
            Some(lock(dir)?)
        } else {
            None
        };
        if let Err(error) = self.stage(&staging) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
        swap(dir, &staging)
    }

    fn stage(&self, staging: &Path) -> Result<()> {
        paths::create_protected_file(&staging.join(KEY_FILE), self.key_pem.as_bytes())?;
        paths::create_protected_file(
            &staging.join(CERT_FILE),
            pem::encode("CERTIFICATE", &self.certificate_der).as_bytes(),
        )?;
        let chain: String = self
            .ca_chain_der
            .iter()
            .map(|der| pem::encode("CERTIFICATE", der))
            .collect();
        paths::create_protected_file(&staging.join(CA_CHAIN_FILE), chain.as_bytes())?;
        let keys =
            serde_json::to_vec(&self.signing_keys).context("cannot serialize the key set")?;
        paths::create_protected_file(&staging.join(SIGNING_KEYS_FILE), &keys)?;
        let metadata = Metadata {
            agent_id: self.agent_id.clone(),
            tenant_id: self.tenant_id.clone(),
            cp_url: self.cp_url.clone(),
            grpc_endpoint: self.grpc_endpoint.clone(),
        };
        let json = serde_json::to_vec_pretty(&metadata).context("cannot serialize the identity")?;
        // The last file marks a complete identity for the next start.
        paths::create_protected_file(&staging.join(METADATA_FILE), &json)?;
        paths::sync_dir(staging)
    }

    /// Reads the identity from `dir` (FR-AG-04).
    ///
    /// # Errors
    ///
    /// Returns an error when a file is missing or malformed.
    pub fn load(dir: &Path) -> Result<Self> {
        let _lock = lock(dir)?;
        let dir = recover(dir);
        finish_rotation(&dir)?;
        let metadata: Metadata =
            serde_json::from_slice(&read(dir.join(METADATA_FILE))?).context("bad identity.json")?;
        let key_pem =
            String::from_utf8(read(dir.join(KEY_FILE))?).context("the key file is not text")?;
        let certificate_der = first_block(&read(dir.join(CERT_FILE))?)?;
        let ca_chain_der = pem::decode_all(
            std::str::from_utf8(&read(dir.join(CA_CHAIN_FILE))?)
                .context("the CA chain file is not text")?,
        )?;
        let signing_keys = serde_json::from_slice(&read(dir.join(SIGNING_KEYS_FILE))?)
            .context("bad signing-keys.json")?;
        Ok(Self {
            agent_id: metadata.agent_id,
            tenant_id: metadata.tenant_id,
            cp_url: metadata.cp_url,
            grpc_endpoint: metadata.grpc_endpoint,
            key_pem,
            certificate_der,
            ca_chain_der,
            signing_keys,
        })
    }

    /// Stores a rotated key pair and certificate in place (FR-ID-03, FR-AG-04).
    ///
    /// The files belong to the same directory as the rest of the agent state, so the rotation
    /// never swaps the directory. It stages the new key, certificate and chain under the
    /// owner-only rules, writes the marker last and moves the files into place. A crash before
    /// the marker keeps the old identity. A crash after it is finished by the next [`Self::load`],
    /// so the key and the certificate never stay apart.
    ///
    /// `agent_id` and `replaced_der` name the identity that the rotation started from.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityReplaced`] when the directory holds another identity now, and an error
    /// when a staged file cannot be written or moved.
    pub fn install_rotation(
        dir: &Path,
        agent_id: &str,
        replaced_der: &[u8],
        rotated: &Rotated,
    ) -> Result<()> {
        let _lock = lock(dir)?;
        let dir = recover(dir);
        // An interrupted earlier rotation with a complete staged set finishes first. Dropping the
        // set after some files moved would leave a new key with an old certificate.
        finish_rotation(&dir)?;
        // A register that saved while the request was open put a new identity here. The rotated
        // certificate belongs to the old one and would break the new identity.
        let metadata: Metadata =
            serde_json::from_slice(&read(dir.join(METADATA_FILE))?).context("bad identity.json")?;
        if metadata.agent_id != agent_id {
            return Err(IdentityReplaced::Agent.into());
        }
        if first_block(&read(dir.join(CERT_FILE))?)? != replaced_der {
            return Err(IdentityReplaced::Certificate.into());
        }
        discard_staged(&dir);
        paths::create_protected_file(&staged(&dir, KEY_FILE), rotated.key_pem.as_bytes())?;
        paths::create_protected_file(
            &staged(&dir, CERT_FILE),
            pem::encode("CERTIFICATE", &rotated.certificate_der).as_bytes(),
        )?;
        let chain: String = rotated
            .ca_chain_der
            .iter()
            .map(|der| pem::encode("CERTIFICATE", der))
            .collect();
        paths::create_protected_file(&staged(&dir, CA_CHAIN_FILE), chain.as_bytes())?;
        paths::create_protected_file(&dir.join(ROTATION_MARKER), b"1")?;
        paths::sync_dir(&dir)?;
        finish_rotation(&dir)
    }

    /// Reports whether the directory holds a stored identity.
    #[must_use]
    pub fn exists(dir: &Path) -> bool {
        current(dir).join(METADATA_FILE).exists()
    }

    /// The time from which and until which the stored leaf certificate is valid (FR-AG-04).
    ///
    /// # Errors
    ///
    /// Returns an error when the certificate is not a valid DER certificate.
    pub fn certificate_validity(&self) -> Result<(OffsetDateTime, OffsetDateTime)> {
        let (_, certificate) =
            x509_parser::certificate::X509Certificate::from_der(&self.certificate_der)
                .map_err(|error| anyhow::anyhow!("cannot parse the agent certificate: {error}"))?;
        let validity = certificate.validity();
        Ok((
            validity.not_before.to_datetime(),
            validity.not_after.to_datetime(),
        ))
    }

    /// The time when the stored leaf certificate stops being valid (FR-AG-04).
    ///
    /// # Errors
    ///
    /// Returns an error when the certificate is not a valid DER certificate.
    pub fn certificate_expiry(&self) -> Result<OffsetDateTime> {
        let (_, certificate) =
            x509_parser::certificate::X509Certificate::from_der(&self.certificate_der)
                .map_err(|error| anyhow::anyhow!("cannot parse the agent certificate: {error}"))?;
        Ok(certificate.validity().not_after.to_datetime())
    }
}

/// The key, the certificate and the chain that a rotation installs (FR-ID-03).
#[derive(Debug)]
pub struct Rotated {
    pub key_pem: String,
    pub certificate_der: Vec<u8>,
    pub ca_chain_der: Vec<Vec<u8>>,
}

/// The stored identity is no longer the one that a rotation started from (FR-ID-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityReplaced {
    /// A registration stored a new certificate of the same agent.
    Certificate,
    /// A registration stored the identity of another agent.
    Agent,
}

impl std::fmt::Display for IdentityReplaced {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Certificate => {
                "a registration stored a new certificate after the rotation started"
            }
            Self::Agent => {
                "a registration stored another agent identity after the rotation started"
            }
        })
    }
}

impl std::error::Error for IdentityReplaced {}

fn staged(dir: &Path, name: &str) -> std::path::PathBuf {
    dir.join(format!("{name}{STAGED_SUFFIX}"))
}

fn discard_staged(dir: &Path) {
    for name in ROTATED_FILES {
        let _ = std::fs::remove_file(staged(dir, name));
    }
    let _ = std::fs::remove_file(dir.join(ROTATION_MARKER));
}

/// Moves a complete staged rotation into place, or drops an incomplete one (FR-ID-03).
///
/// The marker proves that the three staged files are complete. Each move is atomic and the
/// function skips a file that an earlier, interrupted run already moved, so it is safe to repeat.
fn finish_rotation(dir: &Path) -> Result<()> {
    let marker = dir.join(ROTATION_MARKER);
    if !marker.exists() {
        discard_staged(dir);
        return Ok(());
    }
    for name in ROTATED_FILES {
        let source = staged(dir, name);
        if source.exists() {
            std::fs::rename(&source, dir.join(name))
                .with_context(|| format!("cannot install the rotated {name}"))?;
        }
    }
    std::fs::remove_file(&marker).context("cannot remove the rotation marker")?;
    paths::sync_dir(dir)
}

/// Takes the exclusive lock of the identity files; dropping the file releases it (FR-ID-03).
///
/// A load finishes or drops a staged rotation. Without the lock, a second process such as `status`
/// that loads while the service stages a rotation deletes the staged key, and the service then
/// installs the new certificate next to the old key. The lock also keeps a reader from seeing the
/// key of one rotation with the certificate of the next, and a save from swapping the directory
/// under a rotation.
fn lock(dir: &Path) -> Result<std::fs::File> {
    lock_within(dir, LOCK_WAIT)
}

/// Windows cannot rename a directory that holds an open file, so the lock file is next to `dir`.
/// The Linux service may write only inside `dir` (`ProtectSystem=strict`), so the lock file is
/// inside it, and a waiter checks that a swap did not replace it.
pub(crate) fn lock_file(dir: &Path) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        sibling(dir, ".lock")
    }
    #[cfg(not(windows))]
    {
        dir.join("identity.lock")
    }
}

fn lock_within(dir: &Path, wait: Duration) -> Result<std::fs::File> {
    let deadline = Instant::now() + wait;
    loop {
        let path = lock_path(dir);
        match open_lock_file(&path) {
            Ok(file) => match file.try_lock() {
                Ok(()) if still_at(&file, &path) => return Ok(file),
                Ok(()) | Err(std::fs::TryLockError::WouldBlock) => {}
                Err(std::fs::TryLockError::Error(error)) => {
                    return Err(error).with_context(|| format!("cannot lock {}", path.display()));
                }
            },
            // A save renamed the directory between `lock_path` and the open.
            Err(error) if is_not_found(&error) && swap_in_progress(dir) => {}
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            // TimedOut is not a damaged identity, so the service restarts instead of exiting 78.
            return Err(std::io::Error::from(std::io::ErrorKind::TimedOut)).with_context(|| {
                format!(
                    "another process holds the lock {} for more than {wait:?}; stop that process and run the command again",
                    path.display()
                )
            });
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
        .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
}

fn swap_in_progress(dir: &Path) -> bool {
    dir.exists() || sibling(dir, ".old").exists()
}

#[cfg(windows)]
fn lock_path(dir: &Path) -> std::path::PathBuf {
    lock_file(dir)
}

/// Between the two renames of a save, and after a crash there, the lock file is in the backup.
#[cfg(not(windows))]
fn lock_path(dir: &Path) -> std::path::PathBuf {
    let backup = sibling(dir, ".old");
    if !dir.exists() && backup.exists() {
        lock_file(&backup)
    } else {
        lock_file(dir)
    }
}

#[cfg(windows)]
fn open_lock_file(path: &Path) -> Result<std::fs::File> {
    crate::winsec::open_or_create_protected(path)
}

#[cfg(not(windows))]
fn open_lock_file(path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("cannot open {}", path.display()))
}

#[cfg(windows)]
fn still_at(_file: &std::fs::File, _path: &Path) -> bool {
    true
}

/// A save that swapped the directory while this process waited leaves it with the lock of the old
/// directory.
#[cfg(not(windows))]
fn still_at(file: &std::fs::File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    match (file.metadata(), std::fs::metadata(path)) {
        (Ok(held), Ok(current)) => held.dev() == current.dev() && held.ino() == current.ino(),
        _ => false,
    }
}

/// An ECDSA P-256 key pair and its raw public key (ADR-0026).
pub struct GeneratedKey {
    pub key_pem: String,
    /// The raw key bytes, for the response check (ADR-0026).
    pub public_key: Vec<u8>,
}

/// Makes an ECDSA P-256 key pair and the certificate signing request (ADR-0026).
///
/// The private key stays in memory and reaches the configuration directory only through
/// [`StoredIdentity::save`].
///
/// # Errors
///
/// Returns an error when the key or the request cannot be generated.
pub fn generate_key_and_csr() -> Result<(GeneratedKey, Vec<u8>)> {
    let key =
        KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).context("cannot generate the agent key")?;
    let request = CertificateParams::default()
        .serialize_request(&key)
        .context("cannot build the certificate signing request")?;
    let generated = GeneratedKey {
        key_pem: key.serialize_pem(),
        public_key: key.public_key_raw().to_vec(),
    };
    Ok((generated, request.der().to_vec()))
}

/// A path next to `dir` with the given suffix, so both stay on one file system.
pub(crate) fn sibling(dir: &Path, suffix: &str) -> std::path::PathBuf {
    let name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("restichub-agent");
    dir.with_file_name(format!("{name}{suffix}"))
}

/// The directory that holds the identity, with the backup of an interrupted swap (FR-AG-04).
///
/// A swap that a crash interrupted leaves the old identity in the backup and no identity in the
/// main directory. The function moves the backup back, so the agent keeps its identity. The caller
/// holds the lock: a save that is between its two renames looks the same.
pub(crate) fn recover(dir: &Path) -> std::path::PathBuf {
    if dir.join(METADATA_FILE).exists() {
        return dir.to_path_buf();
    }
    let backup = sibling(dir, ".old");
    if backup.join(METADATA_FILE).exists() {
        if std::fs::rename(&backup, dir).is_ok() {
            return dir.to_path_buf();
        }
        return backup;
    }
    dir.to_path_buf()
}

/// The directory that holds the identity, without the move of [`recover`], for a reader that does
/// not take the lock (FR-AG-04).
pub(crate) fn current(dir: &Path) -> std::path::PathBuf {
    if !dir.join(METADATA_FILE).exists() {
        let backup = sibling(dir, ".old");
        if backup.join(METADATA_FILE).exists() {
            return backup;
        }
    }
    dir.to_path_buf()
}

/// Replaces `dir` with `staging` in one rename, keeping the old directory until the swap succeeds.
fn swap(dir: &Path, staging: &Path) -> Result<()> {
    let backup = sibling(dir, ".old");
    let _ = std::fs::remove_dir_all(&backup);
    let had_old = dir.exists();
    if had_old {
        std::fs::rename(dir, &backup)
            .with_context(|| format!("cannot move the old identity to {}", backup.display()))?;
    }
    if let Err(error) = std::fs::rename(staging, dir) {
        if had_old {
            // Put the old identity back, so a failed swap never loses it.
            let _ = std::fs::rename(&backup, dir);
        }
        let _ = std::fs::remove_dir_all(staging);
        return Err(error).context("cannot install the new identity");
    }
    if had_old {
        keep_other_entries(&backup, dir);
        let _ = std::fs::remove_dir_all(&backup);
    }
    if let Some(parent) = dir.parent() {
        paths::sync_dir(parent)?;
    }
    Ok(())
}

/// The entries of the old directory that survive a new registration. Everything else is tied to the
/// old identity (key updates, job journal, status) and would break or mislead the new one.
const KEPT_ENTRIES: [&str; 1] = ["restic.json"];

/// Moves the entries of [`KEPT_ENTRIES`] into the new directory, so a registration does not lose
/// the restic record (FR-IN-07).
fn keep_other_entries(old: &Path, new: &Path) {
    for name in KEPT_ENTRIES {
        let target = new.join(name);
        if !target.exists() {
            let _ = std::fs::rename(old.join(name), target);
        }
    }
}

fn read(path: std::path::PathBuf) -> Result<Vec<u8>> {
    std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))
}

fn first_block(text: &[u8]) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(text).context("the certificate file is not text")?;
    let mut blocks = pem::decode_all(text)?;
    if blocks.is_empty() {
        bail!("the certificate file holds no PEM block");
    }
    Ok(blocks.swap_remove(0))
}

#[cfg(test)]
pub(crate) mod tests;
