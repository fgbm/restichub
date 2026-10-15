//! Who may change the allow-list file and the hook executables (FR-HK-01, AC-HK-01, threat T2).
//!
//! The agent runs a hook as root or LocalSystem. A user who can change the allow-list, an
//! executable or a directory above them would gain that power, so the agent trusts a path only
//! when root or Administrators own every element of the chain and no other account can write it.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

/// Names the account that must own the files the agent trusts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// Root on Unix and Administrators or SYSTEM on Windows. The agent always uses this value.
    Host,
    /// The account that runs the process, and only the file and its directory. A test uses it to
    /// build a fixture without administrator rights; it is never the setting of a running agent.
    CurrentUser,
}

/// Checks the allow-list file: a regular file, not a link, that only the trusted account can
/// change, in directories that only the trusted account can change.
///
/// # Errors
///
/// Returns the reason in words when the file is not trusted.
pub fn check_file(path: &Path, trust: Trust) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("{} is not a regular file", path.display());
    }
    check_chain(path, trust)
}

/// Resolves a program path and checks the result. A link passes only when the link, the target and
/// every directory above both are trusted.
///
/// # Errors
///
/// Returns the reason in words when the executable is not trusted.
pub fn check_executable(path: &Path, trust: Trust) -> Result<PathBuf> {
    if !path.is_absolute() {
        anyhow::bail!("{} is not an absolute path", path.display());
    }
    let resolved = path
        .canonicalize()
        .with_context(|| format!("cannot resolve {}", path.display()))?;
    let metadata = std::fs::metadata(&resolved)
        .with_context(|| format!("cannot read {}", resolved.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("{} is not a regular file", resolved.display());
    }
    check_chain(path, trust)?;
    check_chain(&resolved, trust)?;
    Ok(resolved)
}

/// Walks from the path up to the root and checks each element that exists.
fn check_chain(path: &Path, trust: Trust) -> Result<()> {
    let mut checked = 0_usize;
    for element in path.ancestors() {
        if element.as_os_str().is_empty() {
            break;
        }
        if trust == Trust::CurrentUser && checked > 1 {
            break;
        }
        // A link in the chain is not followed here: the resolved path is checked by the caller.
        if std::fs::symlink_metadata(element).is_err() {
            continue;
        }
        check_element(element, checked == 0, trust)?;
        checked += 1;
    }
    Ok(())
}

#[cfg(unix)]
fn check_element(path: &Path, _is_leaf: bool, trust: Trust) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata =
        std::fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    let owner = match trust {
        Trust::Host => 0,
        Trust::CurrentUser => current_uid(),
    };
    if metadata.uid() != owner || metadata.mode() & 0o022 != 0 {
        anyhow::bail!(
            "{} must belong to the trusted account and must not be writable by a group or other accounts",
            path.display()
        );
    }
    if crate::acl::has_extended_acl(path)? {
        anyhow::bail!("{} holds an extended access control list", path.display());
    }
    Ok(())
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn current_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

#[cfg(windows)]
fn check_element(path: &Path, is_leaf: bool, trust: Trust) -> Result<()> {
    if trust == Trust::CurrentUser {
        return Ok(());
    }
    if is_leaf {
        crate::winsec::check_admin_only(path)
    } else {
        crate::winsec::check_parent(path)
    }
}

#[cfg(not(any(unix, windows)))]
fn check_element(_path: &Path, _is_leaf: bool, _trust: Trust) -> Result<()> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::expect_used)]

    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "restichub-agent-trust-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .expect("cannot set the mode");
        dir.canonicalize().expect("cannot resolve the directory")
    }

    fn file(dir: &Path, name: &str, mode: u32) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "x").expect("cannot write the file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
            .expect("cannot set the mode");
        path
    }

    #[test]
    fn ac_hk_01_a_file_of_the_current_user_without_write_bits_is_trusted_for_that_user() {
        let dir = temp_dir("ok");
        let path = file(&dir, "hooks.toml", 0o644);
        assert!(check_file(&path, Trust::CurrentUser).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ac_hk_01_a_file_that_others_can_write_is_refused() {
        let dir = temp_dir("writable");
        for mode in [0o666, 0o664, 0o646] {
            let path = file(&dir, "hooks.toml", mode);
            assert!(check_file(&path, Trust::CurrentUser).is_err(), "{mode:o}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ac_hk_01_a_directory_that_others_can_write_is_refused() {
        let dir = temp_dir("dir");
        let path = file(&dir, "hooks.toml", 0o644);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777))
            .expect("cannot set the mode");
        assert!(check_file(&path, Trust::CurrentUser).is_err());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755))
            .expect("cannot set the mode");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ac_hk_01_the_host_trust_refuses_a_file_of_a_user_account() {
        let dir = temp_dir("host");
        let path = file(&dir, "hooks.toml", 0o644);
        // The test account is not root, so the host trust refuses its file.
        if current_uid() != 0 {
            assert!(check_file(&path, Trust::Host).is_err());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ac_hk_01_a_link_is_not_an_allow_list_file() {
        let dir = temp_dir("link");
        let target = file(&dir, "real.toml", 0o644);
        let link = dir.join("link.toml");
        std::os::unix::fs::symlink(&target, &link).expect("cannot create the link");
        assert!(check_file(&link, Trust::CurrentUser).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ac_hk_01_an_executable_that_others_can_write_is_refused() {
        let dir = temp_dir("exe");
        let good = file(&dir, "good", 0o755);
        let bad = file(&dir, "bad", 0o757);
        assert!(check_executable(&good, Trust::CurrentUser).is_ok());
        assert!(check_executable(&bad, Trust::CurrentUser).is_err());
        assert!(check_executable(Path::new("relative"), Trust::CurrentUser).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
