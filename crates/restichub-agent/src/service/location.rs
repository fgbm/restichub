//! The install location of the agent (FR-AG-01, threat T20).

use std::path::Path;

use anyhow::{Context, Result};

/// Refuses an agent binary that an account other than root or Administrators can change (T20).
///
/// The check covers the executable, the restic binary next to it, and every parent directory up to
/// the file system root.
///
/// # Errors
///
/// Returns an error when a path is missing or writable by another account.
pub fn check(exe: &Path) -> Result<()> {
    let restic = exe.with_file_name(crate::restic::RESTIC_FILE_NAME);
    for path in [exe, restic.as_path()] {
        check_path(path)?;
    }
    let mut current = exe.parent();
    while let Some(dir) = current {
        check_path(dir)?;
        current = dir.parent();
    }
    Ok(())
}

#[cfg(unix)]
fn check_path(path: &Path) -> Result<()> {
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
    Ok(())
}

#[cfg(windows)]
fn check_path(path: &Path) -> Result<()> {
    crate::winsec::check_admin_only(path).with_context(|| {
        "install the agent under %ProgramFiles%\\ResticHub\\Agent and keep it owned by SYSTEM and Administrators".to_owned()
    })
}

#[cfg(not(any(unix, windows)))]
fn check_path(_path: &Path) -> Result<()> {
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
