//! The checks that run before the one-time token is spent (FR-IN-07, FR-IN-08).
//!
//! A registration that cannot save its identity would burn the token for nothing, so the agent
//! proves first that it has the rights and that every directory is ready.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use time::OffsetDateTime;

use crate::paths;
use crate::verify::{CLOCK_HINT, CLOCK_TOLERANCE};

pub(crate) const PROBE_FILE: &str = ".probe";

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
/// An existing directory of an older build that grants a foreign account a right is repaired when it
/// holds only entries of the agent (threat T20); see [`paths::repair_config_dir`].
///
/// The check covers the configuration directory, the staging directory that the identity save uses
/// and the cache directory. It leaves no probe file behind. `untrusted` says what happens to an
/// identity that another account could change. `verbose` prints each repaired entry.
///
/// # Errors
///
/// Returns [`ChangeableIdentity`] when such an identity stays as found, and an error when a
/// directory cannot be created, is not safe, or does not take a file.
pub fn prepare_directories(
    config: &Path,
    cache: &Path,
    untrusted: paths::UntrustedIdentity,
    verbose: bool,
) -> Result<()> {
    let print = &mut |line: &str| println!("{line}");
    // The first pass removes nothing of the identity, so every refusal that the tree can give comes
    // while the old identity is still in place (T20, FR-IN-07).
    let changeable =
        paths::repair_config_dir(config, paths::UntrustedIdentity::Keep, verbose, print)?;
    if !changeable.is_empty() {
        if untrusted == paths::UntrustedIdentity::Keep {
            return Err(ChangeableIdentity { paths: changeable }.into());
        }
        paths::repair_config_dir(config, untrusted, verbose, print)?;
    }
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
    probe(cache)?;
    Ok(())
}

/// Another account could change these identity entries, so they may hold a replaced CA chain,
/// signing keys or endpoint (T20).
#[derive(Debug)]
pub struct ChangeableIdentity {
    pub paths: Vec<PathBuf>,
}

pub(crate) const REGISTER_AGAIN: &str = "register the host again: create a token with \"Re-register\" on the host page and run `restichub-agent register --force --url <URL> --token <TOKEN>`";

impl ChangeableIdentity {
    #[must_use]
    pub fn files(&self) -> String {
        self.paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl std::fmt::Display for ChangeableIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the identity of this host could be changed by another account ({}), so the agent does not use it; {REGISTER_AGAIN}",
            self.files()
        )
    }
}

impl std::error::Error for ChangeableIdentity {}

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

/// Refuses a host clock that is far from the control plane clock (FR-IN-07, FR-AG-04).
///
/// The issued certificate starts a few minutes before the control plane time. A host that is
/// behind by more than the tolerance would refuse it after the token is spent, so the check runs
/// before the enrollment. An absent control plane time passes.
///
/// # Errors
///
/// Returns an error with both times and the offset when they differ by more than the tolerance.
pub fn check_clock(local: OffsetDateTime, server: Option<OffsetDateTime>) -> Result<()> {
    let Some(server) = server else {
        return Ok(());
    };
    let offset = local - server;
    if offset.abs() > CLOCK_TOLERANCE {
        bail!(
            "the host clock is off by {} s: the local time is {local} and the control plane time is {server}; the token is not used; {CLOCK_HINT}",
            offset.whole_seconds()
        );
    }
    Ok(())
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
        prepare_directories(&config, &cache, paths::UntrustedIdentity::Keep, false)
            .expect("the directories must be prepared");
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
        let error = prepare_directories(
            &base.join("Agent"),
            &blocker.join("cache"),
            paths::UntrustedIdentity::Keep,
            false,
        )
        .expect_err("a cache below a file must fail");
        assert!(error.to_string().contains("cache directory"), "{error}");
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod clock_tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn ac_in_02_a_clock_within_the_tolerance_passes() {
        let server = OffsetDateTime::now_utc();
        check_clock(server - time::Duration::minutes(4), Some(server)).expect("4 minutes behind");
        check_clock(server + time::Duration::minutes(4), Some(server)).expect("4 minutes ahead");
        check_clock(server, None).expect("no control plane time");
    }

    #[test]
    fn ac_in_02_a_clock_beyond_the_tolerance_is_refused_with_both_times() {
        let server = OffsetDateTime::now_utc();
        let local = server - time::Duration::minutes(20);
        let error = check_clock(local, Some(server))
            .expect_err("a clock 20 minutes behind must fail")
            .to_string();
        assert!(error.contains("-1200 s"), "{error}");
        assert!(error.contains(&local.to_string()), "{error}");
        assert!(error.contains(&server.to_string()), "{error}");
        assert!(error.contains("w32tm /resync"), "{error}");
        assert!(error.contains("timedatectl"), "{error}");
    }
}
