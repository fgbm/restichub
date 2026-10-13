//! The protected configuration directory of the agent (FR-AG-04).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

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
    std::fs::create_dir_all(dir).with_context(|| {
        format!(
            "cannot create the configuration directory {}",
            dir.display()
        )
    })?;
    restrict(dir)
}

/// Writes a file that only the service account can read (FR-AG-04).
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub fn create_protected_file(path: &Path, contents: &[u8]) -> Result<()> {
    write_owner_only(path, contents)
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
    Ok(())
}

#[cfg(windows)]
fn restrict(dir: &Path) -> Result<()> {
    restrict_acl(dir)
}

#[cfg(windows)]
fn write_owner_only(path: &Path, contents: &[u8]) -> Result<()> {
    std::fs::write(path, contents).with_context(|| format!("cannot write {}", path.display()))
}

/// Grants full control to SYSTEM and Administrators only, and removes the inherited entries.
///
/// The well-known security identifiers avoid the localized account names; `icacls` accepts the
/// `*SID` form. The `(OI)(CI)` flags apply the entry to the directory and to its children.
///
/// # Errors
///
/// Fails when `icacls` fails.
#[cfg(windows)]
pub fn restrict_acl(dir: &Path) -> Result<()> {
    let status = std::process::Command::new("icacls")
        .arg(dir)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "*S-1-5-32-544:(OI)(CI)F",
        ])
        .status()
        .with_context(|| format!("cannot run icacls on {}", dir.display()))?;
    if !status.success() {
        anyhow::bail!("icacls failed on {} with {status}", dir.display());
    }
    Ok(())
}

#[cfg(test)]
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

    #[cfg(windows)]
    #[test]
    fn the_acl_grants_only_system_and_administrators() {
        let dir = std::env::temp_dir().join(format!("restichub-agent-acl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        restrict_acl(&dir).expect("cannot set the ACL");
        let output = std::process::Command::new("icacls")
            .arg(&dir)
            .output()
            .expect("cannot run icacls");
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains("SYSTEM"), "the ACL must grant SYSTEM: {text}");
        assert!(
            text.contains("Administrators") || text.contains("S-1-5-32-544"),
            "the ACL must grant Administrators: {text}"
        );
        for unexpected in ["Everyone", "BUILTIN\\Users", "Authenticated Users"] {
            assert!(
                !text.contains(unexpected),
                "the ACL must not grant {unexpected}: {text}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
