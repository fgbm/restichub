//! The install location of the agent (FR-AG-01, threat T20).

use std::path::Path;

use anyhow::{Context, Result};

/// Refuses an agent binary that an account other than root or Administrators can change (T20).
///
/// The check covers the executable, the restic binary next to it, the directory that holds them and
/// every parent directory up to the file system root.
///
/// # Errors
///
/// Returns an error when a path is missing or writable by another account.
pub fn check(exe: &Path) -> Result<()> {
    let restic = exe.with_file_name(crate::restic::RESTIC_FILE_NAME);
    for path in [exe, restic.as_path()] {
        check_target(path)?;
    }
    if let Some(dir) = exe.parent() {
        check_target(dir)?;
        let mut current = dir.parent();
        while let Some(parent) = current {
            check_parent_dir(parent)?;
            current = parent.parent();
        }
    }
    Ok(())
}

#[cfg(unix)]
fn check_target(path: &Path) -> Result<()> {
    check_unix(path)
}

#[cfg(unix)]
fn check_parent_dir(path: &Path) -> Result<()> {
    check_unix(path)
}

#[cfg(unix)]
fn check_unix(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata =
        std::fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    // A group or an account other than root with write access can replace the binary.
    if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        anyhow::bail!(
            "{} is writable by an account other than root; install the agent under /opt/restichub-agent and keep it root-owned",
            path.display()
        );
    }
    // An extended POSIX ACL can grant a write that the mode does not show.
    if crate::acl::has_extended_acl(path)? {
        anyhow::bail!(
            "{} holds an extended access control list; install the agent under /opt/restichub-agent and keep it root-owned",
            path.display()
        );
    }
    Ok(())
}

#[cfg(windows)]
fn check_target(path: &Path) -> Result<()> {
    crate::winsec::check_admin_only(path).with_context(install_hint)
}

#[cfg(windows)]
fn check_parent_dir(path: &Path) -> Result<()> {
    crate::winsec::check_parent(path).with_context(install_hint)
}

#[cfg(windows)]
fn install_hint() -> String {
    "install the agent under %ProgramFiles%\\ResticHub\\Agent and keep it owned by SYSTEM and Administrators".to_owned()
}

#[cfg(not(any(unix, windows)))]
fn check_target(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn check_parent_dir(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    #[cfg(unix)]
    #[test]
    fn a_user_owned_binary_is_refused() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("restichub-agent-loc-{}", std::process::id()));
        let bin = dir.join("restichub-agent");
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        std::fs::write(&bin, b"agent").expect("cannot write the binary");
        std::fs::write(dir.join(crate::restic::RESTIC_FILE_NAME), b"restic")
            .expect("cannot write restic");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .expect("cannot set the mode");
        // The temp directory is not owned by root, so the check must refuse it.
        assert!(super::check(&bin).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
