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

/// The cache directory of the agent. The restic child writes its cache there (FR-AG-06).
///
/// Linux uses `/var/cache/restichub-agent`, which the systemd unit creates. Windows uses a
/// subdirectory of the configuration directory.
#[must_use]
pub fn cache_dir() -> PathBuf {
    #[cfg(windows)]
    {
        config_dir().join("cache")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/var/cache/restichub-agent")
    }
}

/// The directory that holds the agent executable and restic (ADR-0038).
///
/// Linux uses `/opt/restichub-agent` and Windows uses `%ProgramFiles%\ResticHub\Agent`. Only the
/// administrator account can write there; the identity, journal and cache stay in the data
/// directories.
#[must_use]
pub fn program_dir() -> PathBuf {
    #[cfg(windows)]
    {
        let program_files = std::env::var_os("ProgramFiles")
            .map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from);
        program_files.join("ResticHub").join("Agent")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/opt/restichub-agent")
    }
}

/// The directory of the downloaded restic binary (ADR-0038).
///
/// Windows keeps it next to the agent executable. Linux uses `bin` in the program directory.
#[must_use]
pub fn restic_dir() -> PathBuf {
    restic_dir_in(&program_dir())
}

/// The restic directory inside a program directory.
#[must_use]
pub fn restic_dir_in(program: &Path) -> PathBuf {
    if cfg!(windows) {
        program.to_path_buf()
    } else {
        program.join("bin")
    }
}

/// Where the agent executable lives after the self-install (FR-IN-07).
#[must_use]
pub fn agent_exe() -> PathBuf {
    agent_exe_in(&program_dir())
}

/// The agent executable path inside a program directory.
#[must_use]
pub fn agent_exe_in(program: &Path) -> PathBuf {
    let name = format!("restichub-agent{}", std::env::consts::EXE_SUFFIX);
    if cfg!(windows) {
        program.join(name)
    } else {
        program.join("bin").join(name)
    }
}

/// The restic directories of earlier installs, the newest first (ADR-0038).
#[must_use]
pub fn legacy_restic_dirs() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        vec![program_dir().join("bin"), config_dir().join("bin")]
    }
    #[cfg(not(windows))]
    {
        vec![PathBuf::from("/var/lib/restichub-agent/bin")]
    }
}

/// The directory for agent data that is not configuration (FR-AG-04).
///
/// Linux uses `/var/lib/restichub-agent`. Windows keeps data in the configuration directory.
#[must_use]
pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        config_dir()
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/var/lib/restichub-agent")
    }
}

/// The `ResticHub` directory under `%ProgramData%`. The agent owns it and every level below it; the
/// levels above it belong to Windows and stay as they are.
#[cfg(windows)]
pub(crate) fn agent_root() -> PathBuf {
    config_dir()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// Repairs the access control that an older build left on the existing agent directories
/// (FR-AG-04, threat T20).
///
/// Only the elevated commands that prepare the directories call it. The service refuses a foreign
/// access instead. A refusal that the repair does not cover (a foreign owner of a level) comes from
/// [`ensure_config_dir`]. Linux needs no repair: [`create_protected_dir`] sets the mode of an
/// existing directory and has no owner check that an old build could trip. `log` gets one line for
/// each warning and removal, and one line for each top-level directory whose access control
/// changed; `verbose` gives one line for each changed entry instead. Returns the identity entries
/// that another account could change and that [`UntrustedIdentity::Keep`] left as found: they may
/// hold replaced trust anchors.
///
/// # Errors
///
/// Returns an error when a directory holds an entry that the agent does not know, a link, a file
/// with more than one name or an entry that another account owns, or when the access control
/// cannot be set.
pub fn repair_config_dir(
    dir: &Path,
    untrusted: UntrustedIdentity,
    verbose: bool,
    log: &mut dyn FnMut(&str),
) -> Result<Vec<PathBuf>> {
    #[cfg(windows)]
    {
        let root = agent_root();
        let mut report = crate::winrepair::Report::new(&root, verbose, log);
        let result = crate::winrepair::repair_agent_tree(dir, &root, untrusted, &mut |action| {
            report.action(action);
        });
        report.finish();
        result
    }
    #[cfg(not(windows))]
    {
        let _ = (dir, untrusted, verbose, log);
        Ok(Vec::new())
    }
}

/// Repairs the access control of the program directory that a manual copy or an older build left
/// open (FR-IN-07, threat T20).
///
/// It runs before the program directory is checked, in the elevated commands only. A restic that
/// another account could change is removed with its record in `config`, so the install downloads
/// it again. Linux needs no repair: the check refuses a shared level with a remedy. `log` and
/// `verbose` work as in [`repair_config_dir`].
///
/// # Errors
///
/// Returns an error when a level holds an entry that the agent does not know, a link or an entry
/// that another account owns, or when the access control cannot be set.
pub fn repair_program_dir(config: &Path, verbose: bool, log: &mut dyn FnMut(&str)) -> Result<()> {
    #[cfg(windows)]
    {
        let program = program_dir();
        let root = program.parent().map(Path::to_path_buf).unwrap_or_default();
        let record = config.join(crate::restic_store::RECORD_FILE);
        let mut report = crate::winrepair::Report::new(&root, verbose, log);
        let result =
            crate::winrepair::repair_program_tree(&program, &root, &record, &mut |action| {
                report.action(action);
            });
        report.finish();
        result
    }
    #[cfg(not(windows))]
    {
        let _ = (config, verbose, log);
        Ok(())
    }
}

/// Refuses an identity in `dir` that another account could change (FR-AG-04, threat T20).
///
/// The service runs it before it loads the identity. It repairs nothing. Linux refuses a link and a
/// directory or an identity file that every account can write; a group write passes, because a
/// umask of 002 gives it to a directory of a test run.
///
/// # Errors
///
/// Returns an error that names the entry another account could change.
pub fn check_identity_dir(dir: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        crate::winrepair::check_identity(dir, &agent_root())
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let files = crate::identity::IDENTITY_FILES
            .iter()
            .map(|file| dir.join(file));
        for path in std::iter::once(dir.to_path_buf()).chain(files) {
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error).with_context(|| format!("cannot read {}", path.display()));
                }
            };
            if metadata.file_type().is_symlink() || metadata.mode() & 0o002 != 0 {
                anyhow::bail!("{} is a link or every account can write it", path.display());
            }
        }
        Ok(())
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = dir;
        Ok(())
    }
}

/// What the repair does with an identity entry that another account could change (T20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UntrustedIdentity {
    /// Leaves the entry and its directory as found, so each later check refuses them again.
    Keep,
    /// Removes the entry, because a new registration replaces the identity.
    Remove,
}

/// Makes the directory and every missing parent, and restricts them to the service account
/// (FR-AG-04).
///
/// # Errors
///
/// Returns an error when the directory cannot be created or its access cannot be restricted.
pub fn ensure_config_dir(dir: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        crate::winsec::create_protected_tree(dir, &agent_root())
    }
    #[cfg(not(windows))]
    {
        create_protected_dir(dir)
    }
}

/// Makes a directory and every missing parent that only the service account can use (FR-AG-04).
///
/// # Errors
///
/// Returns an error when the directory cannot be created or its access cannot be restricted.
pub fn create_protected_dir(dir: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        crate::winsec::create_protected_tree(dir, &agent_root())
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

/// Makes a program directory and every missing parent that only the administrator account can
/// write (FR-IN-07, T20).
///
/// Linux levels are `0755`; the caller runs as root, so root owns them. Windows levels get the
/// protected DACL and refuse a junction, with the `ResticHub` level as the top of the owned tree.
///
/// # Errors
///
/// Returns an error when a level cannot be created or protected, or when an existing Windows level
/// is a reparse point or writable by another account.
pub fn ensure_program_dir(dir: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let host = program_dir();
        let root = match host.parent() {
            Some(root) if dir.starts_with(root) => root.to_path_buf(),
            _ => dir.parent().map(Path::to_path_buf).unwrap_or_default(),
        };
        crate::winsec::create_protected_tree(dir, &root)
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let mut missing = Vec::new();
        let mut level = Some(dir);
        while let Some(current) = level {
            match std::fs::symlink_metadata(current) {
                Ok(meta) => {
                    // An existing level is checked before anything is written below it. The
                    // check follows no link, so a link cannot stand in for a root-owned level.
                    // Only a root-owned level is judged by its mode: a level of another owner is
                    // already refused, and a non-root run exists only in tests.
                    let shared =
                        meta.uid() == 0 && meta.mode() & 0o022 != 0 && meta.mode() & 0o1000 == 0;
                    let foreign_owner = meta.uid() != 0 && meta.uid() != current_uid();
                    if !meta.is_dir() || foreign_owner || shared {
                        anyhow::bail!(
                            "{} must be a directory that root owns and that no other account can write; remove it and run the command again",
                            current.display()
                        );
                    }
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("cannot inspect {}", current.display()));
                }
            }
            missing.push(current);
            level = current.parent();
        }
        for level in missing.into_iter().rev() {
            std::fs::create_dir(level)
                .with_context(|| format!("cannot create the directory {}", level.display()))?;
            std::fs::set_permissions(level, std::fs::Permissions::from_mode(0o755))
                .with_context(|| format!("cannot set the mode of {}", level.display()))?;
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn current_uid() -> u32 {
    // SAFETY: `geteuid` takes no argument and cannot fail.
    unsafe { libc::geteuid() }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn current_uid() -> u32 {
    0
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
    // A crash between the write and the rename leaves a stale temporary file. Remove it first, so a
    // create that refuses an existing file cannot fail the replacement.
    let _ = std::fs::remove_file(&temporary);
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
    use std::os::unix::fs::OpenOptionsExt as _;
    // `create_new` never follows an existing file, so a pre-created path cannot redirect the write.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    file.write_all(contents)
        .with_context(|| format!("cannot write {}", path.display()))?;
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
    fn fr_in_07_a_linked_program_level_is_refused() {
        let base = temp_dir("program-level");
        std::fs::create_dir_all(&base).expect("base");
        let real = base.join("real");
        std::fs::create_dir(&real).expect("real");
        let link = base.join("link");
        std::os::unix::fs::symlink(&real, &link).expect("link");
        let error = ensure_program_dir(&link.join("bin")).expect_err("a linked level");
        assert!(
            format!("{error:#}").contains("must be a directory"),
            "{error:#}"
        );
        assert!(!real.join("bin").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn fr_ag_04_an_identity_that_another_account_could_change_is_refused() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp_dir("identity-check");
        ensure_config_dir(&dir).expect("cannot create the configuration directory");
        let key = dir.join(crate::identity::KEY_FILE);
        create_protected_file(&key, b"key").expect("cannot write the key");
        check_identity_dir(&dir).expect("an owner-only identity passes");
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o666)).expect("mode");
        let error = format!("{:#}", check_identity_dir(&dir).expect_err("a shared key"));
        assert!(error.contains("agent-key.pem"), "{error}");
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).expect("mode");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).expect("mode");
        assert!(check_identity_dir(&dir).is_err(), "a shared directory");
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
