//! The checks that run before the one-time token is spent (FR-IN-07, FR-IN-08).
//!
//! A registration that cannot save its identity would burn the token for nothing, so the agent
//! proves first that it has the rights and that every directory is ready.

use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::paths;

const PROBE_FILE: &str = ".probe";

/// Refuses a process without the rights to write the protected directories (FR-IN-07).
///
/// # Errors
///
/// Returns an error when the process is not root on Linux, or not an elevated administrator on
/// Windows.
pub fn ensure_elevated() -> Result<()> {
    if is_elevated()? {
        Ok(())
    } else {
        bail!(
            "run this command as an administrator (Windows: an elevated PowerShell) or as root (Linux); the agent cannot write its protected directories otherwise"
        )
    }
}

#[cfg(windows)]
fn is_elevated() -> Result<bool> {
    crate::winsec::is_elevated_administrator()
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn is_elevated() -> Result<bool> {
    // SAFETY: `geteuid` takes no argument and cannot fail.
    Ok(unsafe { libc::geteuid() } == 0)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn is_elevated() -> Result<bool> {
    Ok(true)
}

/// Makes the directories of the agent and proves that the agent can write there (FR-AG-04).
///
/// The check covers the configuration directory, the staging directory that the identity save uses
/// and the cache directory. It leaves no probe file behind.
///
/// # Errors
///
/// Returns an error when a directory cannot be created, is not safe, or does not take a file.
pub fn prepare_directories(config: &Path, cache: &Path) -> Result<()> {
    paths::ensure_config_dir(config)
        .with_context(|| format!("cannot prepare the directory {}", config.display()))?;
    let staging = crate::identity::sibling(config, ".staging");
    let _ = std::fs::remove_dir_all(&staging);
    paths::ensure_config_dir(&staging)
        .with_context(|| format!("cannot prepare the directory {}", staging.display()))?;
    let staged = probe(&staging);
    let _ = std::fs::remove_dir_all(&staging);
    staged?;
    prepare_cache(cache)?;
    probe(cache)
}

/// Makes the cache directory of the restic child (FR-AG-06).
///
/// # Errors
///
/// Returns an error when the directory cannot be created or is not safe.
pub fn prepare_cache(cache: &Path) -> Result<()> {
    paths::create_protected_dir(cache)
        .with_context(|| format!("cannot prepare the cache directory {}", cache.display()))
}

fn probe(dir: &Path) -> Result<()> {
    let file = dir.join(PROBE_FILE);
    paths::create_protected_file(&file, b"probe")
        .with_context(|| format!("cannot write into {}", dir.display()))?;
    std::fs::remove_file(&file).with_context(|| format!("cannot remove {}", file.display()))
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "restichub-agent-preflight-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn fr_in_07_the_directories_are_created_with_their_parents_and_hold_no_probe() {
        let base = temp("create");
        let config = base.join("one").join("two").join("Agent");
        let cache = base.join("cache").join("restic");
        prepare_directories(&config, &cache).expect("the directories must be prepared");
        assert!(config.is_dir());
        assert!(cache.is_dir());
        assert!(!cache.join(PROBE_FILE).exists());
        assert!(!crate::identity::sibling(&config, ".staging").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_in_07_a_cache_that_cannot_be_made_fails_the_preparation() {
        let base = temp("blocked");
        std::fs::create_dir_all(&base).expect("cannot create the base");
        let blocker = base.join("file");
        std::fs::write(&blocker, b"x").expect("cannot write the blocker");
        let error = prepare_directories(&base.join("Agent"), &blocker.join("cache"))
            .expect_err("a cache below a file must fail");
        assert!(error.to_string().contains("cache directory"), "{error}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
