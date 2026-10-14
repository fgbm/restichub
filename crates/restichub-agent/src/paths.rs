//! The protected configuration directory of the agent (FR-AG-04).

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

/// Path of the directory that holds the agent identity (FR-AG-04).
///
/// Linux uses `/etc/restichub-agent` and Windows uses `%ProgramData%\ResticHub\Agent`. macOS
/// compiles with the Linux path.
#[must_use]
pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    {
        let program_data = std::env::var_os("ProgramData")
            .map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
        program_data.join("ResticHub").join("Agent")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/etc/restichub-agent")
    }
}

/// Makes the directory and restricts it to the service account (FR-AG-04).
///
/// # Errors
///
/// Returns an error when the directory cannot be created or its access cannot be restricted.
pub fn ensure_config_dir(dir: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        crate::winsec::ensure_config_dir(dir)
    }
    #[cfg(not(windows))]
    {
        create_protected_dir(dir)
    }
}

/// Makes one directory that only the service account can use (FR-AG-04).
///
/// # Errors
///
/// Returns an error when the directory cannot be created or its access cannot be restricted.
pub fn create_protected_dir(dir: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        crate::winsec::create_protected_dir(dir)
    }
    #[cfg(not(windows))]
    {
        std::fs::create_dir_all(dir).with_context(|| {
            format!(
                "cannot create the configuration directory {}",
                dir.display()
            )
        })?;
        restrict(dir)
    }
}

/// Writes a file that only the service account can read (FR-AG-04).
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub fn create_protected_file(path: &Path, contents: &[u8]) -> Result<()> {
    #[cfg(windows)]
    {
        crate::winsec::create_protected_file(path, contents)
    }
    #[cfg(not(windows))]
    {
        write_owner_only(path, contents)
    }
}

/// Replaces a file with one rename, so a reader sees the old or the new content (FR-AG-04).
///
/// # Errors
///
/// Returns an error when the temporary file cannot be written or the rename fails.
pub fn replace_file(path: &Path, contents: &[u8]) -> Result<()> {
    let temporary = path.with_extension("tmp");
    create_protected_file(&temporary, contents)?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("cannot replace {}", path.display()))?;
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

#[cfg(unix)]
fn restrict(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("cannot set the mode of {}", dir.display()))
}

#[cfg(unix)]
fn write_owner_only(path: &Path, contents: &[u8]) -> Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    file.write_all(contents)
        .with_context(|| format!("cannot write {}", path.display()))?;
    // The mode above applies at creation only; an overwritten file keeps its old mode.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("cannot set the mode of {}", path.display()))?;
    // The identity must survive a power loss, so the data and the metadata are flushed.
    file.sync_all()
        .with_context(|| format!("cannot flush {}", path.display()))
}

/// Flushes a directory entry change, so a rename of the identity survives a power loss.
///
/// Windows has no directory handle to flush; the move is durable enough there.
///
/// # Errors
///
/// Returns an error when the directory cannot be opened or flushed.
#[cfg(unix)]
pub fn sync_dir(dir: &Path) -> Result<()> {
    let file = std::fs::File::open(dir)
        .with_context(|| format!("cannot open the directory {}", dir.display()))?;
    file.sync_all()
        .with_context(|| format!("cannot flush the directory {}", dir.display()))
}

/// Flushes a directory entry change, so a rename of the identity survives a power loss.
#[cfg(not(unix))]
pub fn sync_dir(_dir: &Path) -> Result<()> {
    Ok(())
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("restichub-agent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[cfg(not(windows))]
    #[test]
    fn the_config_dir_is_under_etc() {
        assert_eq!(config_dir(), PathBuf::from("/etc/restichub-agent"));
    }

    #[cfg(unix)]
    #[test]
    fn the_config_dir_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp_dir("dir-mode");
        ensure_config_dir(&dir).expect("cannot create the configuration directory");
        let mode = std::fs::metadata(&dir)
            .expect("cannot read the directory")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_stored_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp_dir("file-mode");
        ensure_config_dir(&dir).expect("cannot create the configuration directory");
        let path = dir.join("secret");
        create_protected_file(&path, b"value").expect("cannot write the file");
        let mode = std::fs::metadata(&path)
            .expect("cannot read the file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
