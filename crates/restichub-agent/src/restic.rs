//! The installed restic binary and the SHA-256 that the agent recorded for it (ADR-0038, FR-AG-11).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::restic_release::hex;
use crate::restic_store::Layout;

#[cfg(windows)]
pub const RESTIC_FILE_NAME: &str = "restic.exe";

#[cfg(not(windows))]
pub const RESTIC_FILE_NAME: &str = "restic";

/// Integration point for the backup engine (ADR-0011).
pub trait Restic {
    /// Verifies the binary before any use of it (FR-AG-11).
    ///
    /// # Errors
    ///
    /// Returns an error when the hash differs or when the binary is missing.
    fn verify(&self) -> Result<()>;

    /// Asks restic for its build identifier (FR-AG-11).
    ///
    /// The child runs the checked inode, so the caller needs no earlier hash check.
    ///
    /// # Errors
    ///
    /// Returns an error when the hash differs, when restic does not run, or when it prints no
    /// version.
    fn version(&self) -> Result<String>;
}

/// The restic that the agent downloaded, with the hash from its protected record (ADR-0038).
#[derive(Debug, Clone)]
pub struct ResticBinary {
    path: PathBuf,
    expected_sha256: String,
}

impl ResticBinary {
    #[must_use]
    pub fn new(path: PathBuf, expected_sha256: String) -> Self {
        Self {
            path,
            expected_sha256,
        }
    }

    /// Takes the hash from the protected record and the path from the protected directory (ADR-0038).
    ///
    /// Neither comes from the control plane or from a user file (FR-AG-11).
    ///
    /// # Errors
    ///
    /// Returns an error when the record is absent or damaged, when it names an unsupported
    /// version, or when the binary is missing.
    pub fn installed(layout: &Layout) -> Result<Self> {
        let record = layout.read_record()?;
        crate::restic_release::check_supported(&record.version)
            .context("the recorded restic version is not supported")?;
        let path = layout.binary_path();
        if !path.is_file() {
            bail!(
                "the restic binary {} is missing; run `restichub-agent restic update`",
                path.display()
            );
        }
        Ok(Self::new(path, record.sha256))
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn expected_sha256(&self) -> &str {
        &self.expected_sha256
    }
}

impl Restic for ResticBinary {
    fn verify(&self) -> Result<()> {
        verify_hash(&self.path, &self.expected_sha256)
    }

    fn version(&self) -> Result<String> {
        // The query runs the checked inode through the same handle pin and environment as a job
        // (FR-AG-06, threat T20).
        let output = crate::exec::version_output(self)?;
        if !output.status.success() {
            bail!("`restic version` failed with {}", output.status);
        }
        parse_version(&String::from_utf8_lossy(&output.stdout))
    }
}

/// Checks the SHA-256 of the file at `path` against `expected_hex` (AC-AG-11).
///
/// # Errors
///
/// Returns an error when the binary is missing or its hash differs from `expected_hex`.
pub fn verify_hash(path: &Path, expected_hex: &str) -> Result<()> {
    verify_digest(&digest_at(path)?, expected_hex)
}

/// Like [`verify_hash`], but the error gives no advice, because the caller gives its own.
pub(crate) fn check_hash(path: &Path, expected_hex: &str) -> Result<()> {
    if let Some(mismatch) = mismatch(&digest_at(path)?, expected_hex) {
        bail!(mismatch);
    }
    Ok(())
}

fn digest_at(path: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("cannot read the restic binary at {}", path.display()))?;
    Ok(Sha256::digest(&bytes).to_vec())
}

fn mismatch(actual: &[u8], expected_hex: &str) -> Option<String> {
    let actual = hex(actual);
    (!actual.eq_ignore_ascii_case(expected_hex)).then(|| {
        format!(
            "the restic binary does not match the recorded SHA-256: expected {expected_hex}, got {actual}"
        )
    })
}

/// Checks a SHA-256 digest against the expected hexadecimal value (AC-AG-11).
///
/// The caller hashes the bytes it will run, so the check and the use share one reading of the
/// binary (threat T20).
///
/// # Errors
///
/// Returns an error when the digest differs from `expected_hex`.
pub fn verify_digest(actual: &[u8], expected_hex: &str) -> Result<()> {
    if let Some(mismatch) = mismatch(actual, expected_hex) {
        bail!("{mismatch}; run `restichub-agent restic update` again or restart the service");
    }
    Ok(())
}

/// Parses the version token of the first line, for example `0.19.1` in `restic 0.19.1 compiled ...`.
pub(crate) fn parse_version(text: &str) -> Result<String> {
    let line = text.lines().next().unwrap_or_default();
    let token = line
        .split_whitespace()
        .nth(1)
        .context("`restic version` printed no version")?;
    Ok(token.to_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn temp_file(name: &str, contents: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "restichub-agent-restic-{name}-{}",
            std::process::id()
        ));
        std::fs::write(&path, contents).expect("cannot write the test file");
        path
    }

    #[test]
    fn ac_ag_11_accepts_the_pinned_binary() {
        let path = temp_file("good", b"restic bytes");
        let expected = hex(&Sha256::digest(b"restic bytes"));
        verify_hash(&path, &expected).expect("the pinned binary must pass");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ac_ag_11_refuses_a_different_binary() {
        let path = temp_file("bad", b"restic bytes");
        let expected = hex(&Sha256::digest(b"another binary"));
        let error = verify_hash(&path, &expected).expect_err("a different binary must fail");
        assert!(
            error
                .to_string()
                .contains("does not match the recorded SHA-256")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ac_ag_11_refuses_a_missing_binary() {
        let path = std::env::temp_dir().join("restichub-agent-restic-missing-does-not-exist");
        let error = verify_hash(&path, &"0".repeat(64)).expect_err("a missing binary must fail");
        assert!(error.to_string().contains("cannot read the restic binary"));
    }

    #[cfg(unix)]
    #[test]
    fn t20_start_refuses_an_absent_record_and_a_changed_binary() {
        use crate::restic_release::VerifiedRelease;

        let base = std::env::temp_dir().join(format!(
            "restichub-agent-restic-start-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let layout = Layout::new(base.join("bin"), base.join("config"));
        let error = ResticBinary::installed(&layout).expect_err("nothing is installed");
        assert!(error.to_string().contains("restic update"), "{error:#}");

        let release = VerifiedRelease {
            version: "0.19.1".to_owned(),
            binary: b"restic bytes".to_vec(),
            sha256: hex(&Sha256::digest(b"restic bytes")),
        };
        layout
            .install(&release, &|_| Ok("0.19.1".to_owned()))
            .expect("install");
        let restic = ResticBinary::installed(&layout).expect("installed");
        restic.verify().expect("the installed binary matches");

        std::fs::write(layout.binary_path(), b"another binary").expect("tamper");
        let restic = ResticBinary::installed(&layout).expect("the record still loads");
        let error = restic.verify().expect_err("a changed binary");
        assert!(error.to_string().contains("recorded SHA-256"), "{error:#}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_version_token_is_read() {
        assert_eq!(
            parse_version("restic 0.19.1 compiled with go1.24 on linux/amd64").expect("version"),
            "0.19.1"
        );
    }
}
