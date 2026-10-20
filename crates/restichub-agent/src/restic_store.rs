//! The installed restic binary and its protected record (ADR-0038, FR-AG-11, threat T20).
//!
//! The binary lives in a protected directory. A record in the protected configuration directory
//! holds its version and SHA-256. The agent compares the binary with the record at every start and
//! before every spawn, so a changed binary needs a changed record, and both need administrator
//! rights.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::restic_release::{VerifiedRelease, check_supported, hex};

pub(crate) const RECORD_FILE: &str = "restic.json";

// Windows refuses to replace an executable that a process runs.
#[cfg(windows)]
const REPLACE_HINT: &str =
    "; stop the restichub-agent service, run the update again and start the service";

#[cfg(not(windows))]
const REPLACE_HINT: &str = "";

/// Where the restic binary and its record live.
#[derive(Debug, Clone)]
pub struct Layout {
    bin_dir: PathBuf,
    config_dir: PathBuf,
}

/// What the agent knows about the installed binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub version: String,
    pub sha256: String,
}

impl Layout {
    #[must_use]
    pub fn new(bin_dir: PathBuf, config_dir: PathBuf) -> Self {
        Self {
            bin_dir,
            config_dir,
        }
    }

    /// The `bin` and configuration directories that ADR-0038 sets for this host.
    #[must_use]
    pub fn host() -> Self {
        Self::new(crate::paths::restic_dir(), crate::paths::config_dir())
    }

    #[must_use]
    pub fn binary_path(&self) -> PathBuf {
        self.bin_dir.join(crate::restic::RESTIC_FILE_NAME)
    }

    #[must_use]
    pub fn previous_path(&self) -> PathBuf {
        self.bin_dir
            .join(format!("restic-previous{}", std::env::consts::EXE_SUFFIX))
    }

    fn staging_path(&self) -> PathBuf {
        self.bin_dir
            .join(format!("restic-new{}", std::env::consts::EXE_SUFFIX))
    }

    #[must_use]
    pub fn record_path(&self) -> PathBuf {
        self.config_dir.join(RECORD_FILE)
    }

    #[must_use]
    pub fn bin_dir(&self) -> &Path {
        &self.bin_dir
    }

    /// Tells whether a record exists. A damaged record counts as present, so a start reports it.
    #[must_use]
    pub fn has_record(&self) -> bool {
        self.record_path().exists()
    }

    /// Loads and validates the record file.
    ///
    /// # Errors
    ///
    /// Returns an error when the record is absent, is not valid JSON, or holds a bad hash.
    pub fn read_record(&self) -> Result<Record> {
        let path = self.record_path();
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "restic is not installed: cannot read {}; run `restichub-agent restic update`",
                path.display()
            )
        })?;
        let record: Record = serde_json::from_str(&text)
            .with_context(|| format!("{} is damaged", path.display()))?;
        let valid = record.sha256.len() == 64
            && record
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !valid {
            bail!("{} holds a bad SHA-256", path.display());
        }
        Ok(record)
    }

    /// Hashes the binary and compares the hash with the record.
    ///
    /// # Errors
    ///
    /// Returns an error when the record or the binary is missing or they differ.
    pub fn verify_installed(&self) -> Result<Record> {
        let record = self.read_record()?;
        crate::restic::verify_hash(&self.binary_path(), &record.sha256)?;
        Ok(record)
    }

    /// Installs a verified release and swaps it in with one rename (ADR-0038).
    ///
    /// The new binary is written beside the old one, hashed again and run with `probe`, which
    /// returns the version that the binary prints. Only then does it replace the old binary, which
    /// stays as the previous binary. Any failure leaves the old binary and the old record in place.
    ///
    /// # Errors
    ///
    /// Returns an error when a directory or file cannot be written, when the staged binary differs
    /// from the verified one, when `probe` fails or reports another version, or when the swap fails.
    pub fn install(
        &self,
        release: &VerifiedRelease,
        probe: &dyn Fn(&Path) -> Result<String>,
    ) -> Result<Record> {
        crate::paths::ensure_program_dir(&self.bin_dir)?;
        crate::paths::create_protected_dir(&self.config_dir)?;
        let staging = self.staging_path();
        let _ = std::fs::remove_file(&staging);
        let staged = self.stage_and_check(&staging, release, probe);
        if let Err(error) = staged {
            let _ = std::fs::remove_file(&staging);
            return Err(error);
        }
        let record = Record {
            version: release.version.clone(),
            sha256: release.sha256.clone(),
        };
        self.swap(&staging, &record)?;
        Ok(record)
    }

    fn stage_and_check(
        &self,
        staging: &Path,
        release: &VerifiedRelease,
        probe: &dyn Fn(&Path) -> Result<String>,
    ) -> Result<()> {
        crate::paths::create_program_file(staging, &release.binary)?;
        make_executable(staging)?;
        let written = std::fs::read(staging)
            .with_context(|| format!("cannot read back {}", staging.display()))?;
        if hex(&Sha256::digest(&written)) != release.sha256 {
            bail!("the staged restic differs from the verified one");
        }
        let printed = probe(staging).context("the downloaded restic does not run")?;
        if printed != release.version {
            bail!(
                "the downloaded restic reports version {printed}, but the release is {}",
                release.version
            );
        }
        check_supported(&printed)
    }

    fn swap(&self, staging: &Path, record: &Record) -> Result<()> {
        let binary = self.binary_path();
        let previous = self.previous_path();
        let old_record = std::fs::read(self.record_path()).ok();
        let had_binary = binary.is_file();
        if had_binary {
            let _ = std::fs::remove_file(&previous);
            std::fs::copy(&binary, &previous).with_context(|| {
                format!("cannot keep the previous restic at {}", previous.display())
            })?;
            make_executable(&previous)?;
        }
        if let Err(error) = std::fs::rename(staging, &binary) {
            let _ = std::fs::remove_file(staging);
            return Err(anyhow::Error::new(error)
                .context(format!("cannot replace {}{REPLACE_HINT}", binary.display())));
        }
        let text = serde_json::to_vec_pretty(record).context("cannot write the record")?;
        let recorded = crate::paths::replace_file(&self.record_path(), &text)
            .and_then(|()| self.verify_installed().map(|_| ()));
        if let Err(error) = recorded {
            if had_binary {
                let _ = std::fs::rename(&previous, &binary);
            } else {
                let _ = std::fs::remove_file(&binary);
            }
            match old_record {
                Some(bytes) => {
                    let _ = crate::paths::replace_file(&self.record_path(), &bytes);
                }
                None => {
                    let _ = std::fs::remove_file(self.record_path());
                }
            }
            return Err(error.context("the new restic was rolled back"));
        }
        Ok(())
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .with_context(|| format!("cannot set the mode of {}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// How long a staged restic may take to print its version.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Asks the binary for its version with a plain child process.
///
/// This runs only on the staged file of an install, which lies in the protected directory and was
/// just hashed. A job never takes this path (FR-AG-11). A binary that does not answer within
/// 30 seconds is killed, so a faulty release cannot hang the update.
///
/// # Errors
///
/// Returns an error when the binary does not run, does not answer in time or prints no version.
pub fn probe_version(path: &Path) -> Result<String> {
    probe_version_within(path, PROBE_TIMEOUT)
}

fn probe_version_within(path: &Path, timeout: Duration) -> Result<String> {
    let mut command = std::process::Command::new(path);
    command
        .arg("version")
        .env_clear()
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    // A Go program on Windows needs the system root to start.
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("cannot run {}", path.display()))?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().context("cannot wait for restic")? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "`restic version` did not answer within {} seconds",
                timeout.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if !status.success() {
        bail!("`restic version` failed with {status}");
    }
    let mut stdout = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_end(&mut stdout)
            .context("cannot read the restic version")?;
    }
    crate::restic::parse_version(&String::from_utf8_lossy(&stdout))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "restichub-agent-store-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn release(version: &str, bytes: &[u8]) -> VerifiedRelease {
        VerifiedRelease {
            version: version.to_owned(),
            binary: bytes.to_vec(),
            sha256: hex(&Sha256::digest(bytes)),
        }
    }

    fn layout(name: &str) -> (Layout, PathBuf) {
        let base = temp(name);
        (Layout::new(base.join("bin"), base.join("config")), base)
    }

    fn prints(version: &'static str) -> impl Fn(&Path) -> Result<String> {
        move |_| Ok(version.to_owned())
    }

    #[test]
    fn fr_ag_11_an_install_writes_the_binary_and_the_record() {
        let (layout, base) = layout("fresh");
        let record = layout
            .install(&release("0.19.1", b"one"), &prints("0.19.1"))
            .expect("install");
        assert_eq!(std::fs::read(layout.binary_path()).expect("binary"), b"one");
        assert_eq!(layout.verify_installed().expect("verify"), record);
        assert!(!layout.staging_path().exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn t20_start_refuses_a_binary_that_differs_from_the_record() {
        let (layout, base) = layout("tamper");
        layout
            .install(&release("0.19.1", b"one"), &prints("0.19.1"))
            .expect("install");
        std::fs::write(layout.binary_path(), b"evil").expect("tamper");
        let error = layout.verify_installed().expect_err("tampered");
        assert!(error.to_string().contains("does not match the recorded"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn t20_a_damaged_record_is_refused() {
        let (layout, base) = layout("record");
        layout
            .install(&release("0.19.1", b"one"), &prints("0.19.1"))
            .expect("install");
        std::fs::write(
            layout.record_path(),
            br#"{"version":"0.19.1","sha256":"zz"}"#,
        )
        .expect("write");
        assert!(layout.read_record().is_err());
        std::fs::write(layout.record_path(), b"not json").expect("write");
        assert!(layout.read_record().is_err());
        let _ = std::fs::remove_file(layout.record_path());
        let error = layout.read_record().expect_err("absent");
        assert!(error.to_string().contains("restic update"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_11_an_update_keeps_the_previous_binary() {
        let (layout, base) = layout("update");
        layout
            .install(&release("0.18.1", b"old"), &prints("0.18.1"))
            .expect("first");
        layout
            .install(&release("0.19.1", b"new"), &prints("0.19.1"))
            .expect("second");
        assert_eq!(std::fs::read(layout.binary_path()).expect("binary"), b"new");
        assert_eq!(
            std::fs::read(layout.previous_path()).expect("previous"),
            b"old"
        );
        assert_eq!(layout.read_record().expect("record").version, "0.19.1");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_11_a_failed_probe_keeps_the_old_binary_and_record() {
        let (layout, base) = layout("probe");
        let first = layout
            .install(&release("0.18.1", b"old"), &prints("0.18.1"))
            .expect("first");
        let error = layout
            .install(&release("0.19.1", b"new"), &|_| bail!("does not run"))
            .expect_err("a broken binary");
        assert!(format!("{error:#}").contains("does not run"));
        assert_eq!(std::fs::read(layout.binary_path()).expect("binary"), b"old");
        assert_eq!(layout.read_record().expect("record"), first);
        assert!(!layout.staging_path().exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_11_a_version_mismatch_keeps_the_old_binary() {
        let (layout, base) = layout("mismatch");
        layout
            .install(&release("0.18.1", b"old"), &prints("0.18.1"))
            .expect("first");
        let error = layout
            .install(&release("0.19.1", b"new"), &prints("0.18.1"))
            .expect_err("a version lie");
        assert!(error.to_string().contains("reports version"));
        assert_eq!(std::fs::read(layout.binary_path()).expect("binary"), b"old");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_11_an_unsupported_version_is_not_installed() {
        let (layout, base) = layout("unsupported");
        let error = layout
            .install(&release("0.17.0", b"old"), &prints("0.17.0"))
            .expect_err("below the minimum");
        assert!(error.to_string().contains("older than the minimum"));
        assert!(!layout.binary_path().exists());
        assert!(!layout.has_record());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn fr_ag_11_a_binary_that_does_not_answer_is_killed() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp("hang");
        std::fs::create_dir_all(&dir).expect("dir");
        let script = dir.join("restic");
        std::fs::write(&script, "#!/bin/sh\nexec sleep 30\n").expect("script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).expect("mode");
        let started = Instant::now();
        let error =
            probe_version_within(&script, Duration::from_millis(300)).expect_err("a hung binary");
        assert!(error.to_string().contains("did not answer"), "{error:#}");
        assert!(started.elapsed() < Duration::from_secs(10));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn fr_ag_11_a_failed_swap_removes_the_staging_file() {
        let (layout, base) = layout("swap");
        // A directory with content at the binary path makes the rename fail.
        std::fs::create_dir_all(layout.binary_path()).expect("dir");
        std::fs::write(layout.binary_path().join("keep"), b"x").expect("file");
        let error = layout
            .install(&release("0.19.1", b"new"), &prints("0.19.1"))
            .expect_err("the swap fails");
        assert!(error.to_string().contains("cannot replace"), "{error:#}");
        assert!(!layout.staging_path().exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
