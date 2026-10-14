//! The bundled restic binary and its embedded SHA-256 (ADR-0031, FR-AG-11).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// The SHA-256 of the bundled restic binary, embedded at build time (ADR-0031).
///
/// The packaging CI sets `RESTICHUB_RESTIC_SHA256` after it verifies the restic release against the
/// signed `SHA256SUMS`. An empty value is a development build without a pinned binary.
pub const EXPECTED_SHA256: &str = env!("RESTICHUB_RESTIC_SHA256");

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
    /// Call the hash check before this call.
    ///
    /// # Errors
    ///
    /// Returns an error when restic does not run or prints no version.
    fn version(&self) -> Result<String>;
}

/// The bundled restic that sits next to the agent executable (ADR-0031).
#[derive(Debug, Clone)]
pub struct ResticBinary {
    path: PathBuf,
}

impl ResticBinary {
    /// Resolves the restic binary next to the running agent executable (ADR-0031).
    ///
    /// The path never comes from the control plane or from a configuration file (ADR-0031).
    ///
    /// # Errors
    ///
    /// Returns an error when the path of the running executable is not readable.
    pub fn locate() -> Result<Self> {
        let exe = std::env::current_exe().context("cannot find the agent executable")?;
        let dir = exe
            .parent()
            .context("the agent executable has no directory")?;
        Ok(Self {
            path: dir.join(RESTIC_FILE_NAME),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Restic for ResticBinary {
    fn verify(&self) -> Result<()> {
        verify_hash(&self.path, EXPECTED_SHA256)
    }

    fn version(&self) -> Result<String> {
        let output = Command::new(&self.path)
            .arg("version")
            // A restic child never inherits the agent environment (FR-AG-06, ADR-0021).
            .env_clear()
            .output()
            .with_context(|| format!("cannot run {}", self.path.display()))?;
        if !output.status.success() {
            bail!("`restic version` failed with {}", output.status);
        }
        parse_version(&String::from_utf8_lossy(&output.stdout))
    }
}

/// Checks the SHA-256 of the file at `path` against `expected_hex` (AC-AG-11).
///
/// An empty `expected_hex` is a development build. Without the `dev-unpinned-restic` feature the
/// agent refuses the binary, so a release never runs an unverified restic.
///
/// # Errors
///
/// Returns an error when the binary is missing or its hash differs from `expected_hex`.
pub fn verify_hash(path: &Path, expected_hex: &str) -> Result<()> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("cannot read the bundled restic at {}", path.display()))?;
    verify_digest(&Sha256::digest(&bytes), expected_hex)
}

/// Checks a SHA-256 digest against the expected hexadecimal value (AC-AG-11).
///
/// The caller hashes the bytes it will run, so the check and the use share one reading of the
/// binary (threat T20).
///
/// # Errors
///
/// Returns an error when the digest differs from `expected_hex`, or when the build holds no pinned
/// hash and the `dev-unpinned-restic` feature is absent.
pub fn verify_digest(actual: &[u8], expected_hex: &str) -> Result<()> {
    let actual = hex(actual);
    if expected_hex.is_empty() {
        if cfg!(feature = "dev-unpinned-restic") {
            tracing::warn!("the restic hash is not pinned in this build");
            return Ok(());
        }
        bail!(
            "the restic binary is not verified: this build holds no pinned SHA-256; install a released agent package"
        );
    }
    if !actual.eq_ignore_ascii_case(expected_hex) {
        bail!(
            "the restic binary does not match the pinned SHA-256: expected {expected_hex}, got {actual}"
        );
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Parses the version token of the first line, for example `0.19.1` in `restic 0.19.1 compiled ...`.
fn parse_version(text: &str) -> Result<String> {
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
                .contains("does not match the pinned SHA-256")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ac_ag_11_refuses_a_missing_binary() {
        let path = std::env::temp_dir().join("restichub-agent-restic-missing-does-not-exist");
        let error = verify_hash(&path, &"0".repeat(64)).expect_err("a missing binary must fail");
        assert!(error.to_string().contains("cannot read the bundled restic"));
    }

    #[test]
    fn ac_ag_11_refuses_an_unpinned_build_without_the_feature() {
        // A release build holds no pinned hash and must refuse the binary.
        if cfg!(feature = "dev-unpinned-restic") {
            return;
        }
        let path = temp_file("unpinned", b"restic bytes");
        assert!(verify_hash(&path, "").is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_version_token_is_read() {
        assert_eq!(
            parse_version("restic 0.19.1 compiled with go1.24 on linux/amd64").expect("version"),
            "0.19.1"
        );
    }
}
