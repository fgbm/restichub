//! The self-install of the agent into the program directory and the move of an old install
//! (FR-IN-07, FR-IN-08, T20, ADR-0038).
//!
//! The location check of the service passes by construction: the agent copies itself into a
//! directory that only the administrator account can write, and the service starts from there.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest as _, Sha256};

use crate::restic::RESTIC_FILE_NAME;
use crate::service::ServiceManager;

/// Where the agent and restic belong, and where an older install left them.
#[derive(Debug, Clone)]
pub struct Placement {
    pub program_dir: PathBuf,
    pub exe: PathBuf,
    pub restic_dir: PathBuf,
    pub legacy_restic_dirs: Vec<PathBuf>,
}

impl Placement {
    #[must_use]
    pub fn host() -> Self {
        Self {
            program_dir: crate::paths::program_dir(),
            exe: crate::paths::agent_exe(),
            restic_dir: crate::paths::restic_dir(),
            legacy_restic_dirs: crate::paths::legacy_restic_dirs(),
        }
    }

    /// A placement below `program_dir`, so a test never touches the real directories.
    #[must_use]
    pub fn under(program_dir: &Path, legacy_restic_dirs: Vec<PathBuf>) -> Self {
        Self {
            program_dir: program_dir.to_path_buf(),
            exe: crate::paths::agent_exe_in(program_dir),
            restic_dir: crate::paths::restic_dir_in(program_dir),
            legacy_restic_dirs,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub copied_agent: bool,
    pub moved_restic: bool,
    pub stopped_service: bool,
}

/// Puts the agent and an old restic into the program directory.
///
/// A running service is stopped before a file in use is replaced; the caller starts it again.
///
/// # Errors
///
/// Returns an error when a directory cannot be made or protected, when a file cannot be copied, or
/// when the service does not stop.
pub fn prepare(
    placement: &Placement,
    source: &Path,
    manager: &dyn ServiceManager,
) -> Result<Outcome> {
    crate::paths::ensure_program_dir(&placement.restic_dir)?;
    crate::paths::ensure_program_dir(&placement.program_dir)?;

    let legacy = legacy_files(placement);
    let copy_agent = !same_content(source, &placement.exe)?;
    let mut outcome = Outcome::default();
    if (copy_agent || !legacy.is_empty()) && manager.state()?.running {
        manager.stop()?;
        outcome.stopped_service = true;
    }
    if copy_agent {
        copy_file(source, &placement.exe)?;
        outcome.copied_agent = true;
    }
    if !legacy.is_empty() {
        move_legacy(placement, &legacy)?;
        outcome.moved_restic = true;
    }
    Ok(outcome)
}

/// Moves restic from an earlier install location into the restic directory.
///
/// The service start calls it too, so an agent that an update replaced finds restic without a new
/// `register`. Returns whether a file moved.
///
/// # Errors
///
/// Returns an error when a file cannot be copied or its copy differs from the original.
pub fn migrate_restic(placement: &Placement) -> Result<bool> {
    let legacy = legacy_files(placement);
    if legacy.is_empty() {
        return Ok(false);
    }
    crate::paths::ensure_program_dir(&placement.restic_dir)?;
    move_legacy(placement, &legacy)?;
    Ok(true)
}

fn move_legacy(placement: &Placement, legacy: &[(PathBuf, PathBuf)]) -> Result<()> {
    for (old, new) in legacy {
        move_file(old, new)?;
    }
    for dir in &placement.legacy_restic_dirs {
        // A directory stays when something else lies in it, and the restic directory itself stays.
        if *dir != placement.restic_dir {
            let _ = std::fs::remove_dir(dir);
        }
    }
    Ok(())
}

/// The old restic files and where each one goes. A file whose target exists is only removed.
fn legacy_files(placement: &Placement) -> Vec<(PathBuf, PathBuf)> {
    let previous = format!("restic-previous{}", std::env::consts::EXE_SUFFIX);
    let mut files = Vec::new();
    for dir in &placement.legacy_restic_dirs {
        if *dir == placement.restic_dir {
            continue;
        }
        for name in [RESTIC_FILE_NAME, previous.as_str()] {
            let old = dir.join(name);
            if old.is_file() {
                files.push((old, placement.restic_dir.join(name)));
            }
        }
    }
    files
}

fn same_content(source: &Path, target: &Path) -> Result<bool> {
    if !target.is_file() {
        return Ok(false);
    }
    if let (Ok(a), Ok(b)) = (source.canonicalize(), target.canonicalize())
        && a == b
    {
        return Ok(true);
    }
    Ok(digest(source)? == digest(target)?)
}

/// A path in a user folder can be swapped by another process between the start and the copy, and
/// the service would run the swapped file as an administrator (T20). On Linux this path names the
/// running image itself.
///
/// # Errors
///
/// Returns an error when the platform does not tell the path of the executable.
pub fn running_image() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        Ok(PathBuf::from("/proc/self/exe"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        std::env::current_exe().context("cannot find the agent executable")
    }
}

fn digest(path: &Path) -> Result<Vec<u8>> {
    Ok(Sha256::digest(read_source(path)?).to_vec())
}

/// Reads a file. On Windows the file stays open without write and delete sharing while it is read,
/// so no other process changes it during the copy.
fn read_source(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read as _;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        const FILE_SHARE_READ: u32 = 1;
        options.share_mode(FILE_SHARE_READ);
    }
    let mut bytes = Vec::new();
    options
        .open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .with_context(|| format!("cannot read {}", path.display()))?;
    Ok(bytes)
}

/// Writes `target` with one rename, so a reader sees the old or the new file (FR-IN-07).
fn copy_file(source: &Path, target: &Path) -> Result<()> {
    let bytes = read_source(source)?;
    let staging = staging_path(target);
    // A killed earlier run leaves its staging file behind, and `create_new` refuses it.
    let _ = std::fs::remove_file(&staging);
    crate::paths::create_protected_file(&staging, &bytes)?;
    make_executable(&staging)
        .and_then(|()| rename_with_retry(&staging, target))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&staging);
        })
}

/// The image of a service that just stopped can stay mapped for a moment, so a rename on Windows
/// can fail with an access error.
fn rename_with_retry(from: &Path, to: &Path) -> Result<()> {
    const ATTEMPTS: u32 = 20;
    let mut attempt = 1;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) if cfg!(windows) && attempt < ATTEMPTS => {
                tracing::debug!(%error, attempt, "the rename failed; trying again");
                std::thread::sleep(std::time::Duration::from_millis(250));
                attempt += 1;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("cannot replace {}", to.display()));
            }
        }
    }
}

/// Copies a file into the program directory, checks the copy, then removes the old one.
///
/// The old file stays when the copy fails, so a failed move never loses the binary.
fn move_file(old: &Path, new: &Path) -> Result<()> {
    if !new.exists() {
        copy_file(old, new)?;
        // The record names the hash of the binary, so a copy that differs must not replace it (T20).
        if digest(old)? != digest(new)? {
            let _ = std::fs::remove_file(new);
            anyhow::bail!("the copy of {} differs from the original", old.display());
        }
    }
    std::fs::remove_file(old).with_context(|| format!("cannot remove {}", old.display()))
}

fn staging_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".new");
    target.with_file_name(name)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .with_context(|| format!("cannot set the mode of {}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::cell::{Cell, RefCell};

    use super::*;
    use crate::service::{ServiceState, StartType};

    struct Fake {
        running: bool,
        events: RefCell<Vec<&'static str>>,
        stopped: Cell<bool>,
    }

    impl Fake {
        fn new(running: bool) -> Self {
            Self {
                running,
                events: RefCell::new(Vec::new()),
                stopped: Cell::new(false),
            }
        }
    }

    impl ServiceManager for Fake {
        fn install_and_start(&self, _restic: &Path) -> Result<()> {
            self.events.borrow_mut().push("start");
            Ok(())
        }

        fn state(&self) -> Result<ServiceState> {
            Ok(ServiceState {
                installed: true,
                running: self.running && !self.stopped.get(),
                start_type: StartType::Automatic,
            })
        }

        fn stop(&self) -> Result<()> {
            self.events.borrow_mut().push("stop");
            self.stopped.set(true);
            Ok(())
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "restichub-agent-selfinstall-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        // The program directory check refuses a parent that another account can write.
        #[cfg(windows)]
        crate::paths::create_protected_dir(&dir).expect("cannot create the protected directory");
        #[cfg(not(windows))]
        std::fs::create_dir_all(&dir).expect("cannot create the temp directory");
        dir
    }

    #[test]
    fn fr_in_07_the_agent_copies_itself_into_the_program_dir() {
        let base = temp_dir("copy");
        let source = base.join("Downloads").join("restichub-agent-download");
        std::fs::create_dir_all(source.parent().expect("parent")).expect("downloads");
        std::fs::write(&source, b"agent v1").expect("source");
        let placement = Placement::under(&base.join("program"), vec![base.join("old-bin")]);
        let fake = Fake::new(false);

        let outcome = prepare(&placement, &source, &fake).expect("prepare");

        assert!(outcome.copied_agent);
        assert!(!outcome.stopped_service);
        assert_eq!(std::fs::read(&placement.exe).expect("copy"), b"agent v1");
        assert!(!staging_path(&placement.exe).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = |path: &Path| {
                std::fs::metadata(path)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777
            };
            assert_eq!(mode(&placement.exe), 0o755);
            assert_eq!(mode(&placement.program_dir), 0o755);
            assert_eq!(mode(&placement.restic_dir), 0o755);
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_in_07_a_stale_staging_file_does_not_block_the_install() {
        let base = temp_dir("stale");
        let source = base.join("agent-download");
        std::fs::write(&source, b"agent v2").expect("source");
        let placement = Placement::under(&base.join("program"), vec![base.join("old-bin")]);
        crate::paths::ensure_program_dir(&placement.restic_dir).expect("tree");
        std::fs::write(staging_path(&placement.exe), b"half").expect("stale file");

        prepare(&placement, &source, &Fake::new(false)).expect("prepare");

        assert_eq!(std::fs::read(&placement.exe).expect("copy"), b"agent v2");
        assert!(!staging_path(&placement.exe).exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_in_07_a_second_run_changes_nothing() {
        let base = temp_dir("again");
        let source = base.join("agent-download");
        std::fs::write(&source, b"agent v1").expect("source");
        let placement = Placement::under(&base.join("program"), vec![base.join("old-bin")]);
        prepare(&placement, &source, &Fake::new(true)).expect("first");
        let fake = Fake::new(true);

        let outcome = prepare(&placement, &source, &fake).expect("second");

        assert_eq!(outcome, Outcome::default());
        assert!(fake.events.borrow().is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_in_08_a_running_service_stops_before_the_executable_is_replaced() {
        let base = temp_dir("replace");
        let source = base.join("agent-download");
        let placement = Placement::under(&base.join("program"), vec![base.join("old-bin")]);
        std::fs::write(&source, b"agent v1").expect("source");
        prepare(&placement, &source, &Fake::new(false)).expect("first");
        std::fs::write(&source, b"agent v2").expect("newer source");
        let fake = Fake::new(true);

        let outcome = prepare(&placement, &source, &fake).expect("replace");

        assert!(outcome.copied_agent && outcome.stopped_service);
        assert_eq!(*fake.events.borrow(), vec!["stop"]);
        assert_eq!(std::fs::read(&placement.exe).expect("exe"), b"agent v2");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_in_07_an_old_restic_moves_into_the_program_dir() {
        let base = temp_dir("migrate");
        let source = base.join("agent-download");
        std::fs::write(&source, b"agent").expect("source");
        let legacy = base.join("old-bin");
        std::fs::create_dir_all(&legacy).expect("legacy");
        std::fs::write(legacy.join(RESTIC_FILE_NAME), b"restic 1").expect("old restic");
        let placement = Placement::under(&base.join("program"), vec![legacy.clone()]);
        let fake = Fake::new(true);

        let outcome = prepare(&placement, &source, &fake).expect("migrate");

        assert!(outcome.moved_restic && outcome.stopped_service);
        assert_eq!(
            std::fs::read(placement.restic_dir.join(RESTIC_FILE_NAME)).expect("moved"),
            b"restic 1"
        );
        assert!(!legacy.join(RESTIC_FILE_NAME).exists());
        assert!(!legacy.exists(), "the empty old directory is removed");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_in_07_a_current_restic_wins_over_an_old_copy() {
        let base = temp_dir("both");
        let source = base.join("agent-download");
        std::fs::write(&source, b"agent").expect("source");
        let legacy = base.join("old-bin");
        std::fs::create_dir_all(&legacy).expect("legacy");
        std::fs::write(legacy.join(RESTIC_FILE_NAME), b"old").expect("old restic");
        let placement = Placement::under(&base.join("program"), vec![legacy.clone()]);
        std::fs::create_dir_all(&placement.restic_dir).expect("bin");
        std::fs::write(placement.restic_dir.join(RESTIC_FILE_NAME), b"current").expect("current");

        prepare(&placement, &source, &Fake::new(false)).expect("migrate");

        assert_eq!(
            std::fs::read(placement.restic_dir.join(RESTIC_FILE_NAME)).expect("kept"),
            b"current"
        );
        assert!(!legacy.join(RESTIC_FILE_NAME).exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_in_07_the_executable_already_in_place_is_not_copied() {
        let base = temp_dir("inplace");
        let placement = Placement::under(&base.join("program"), vec![base.join("old-bin")]);
        crate::paths::ensure_program_dir(&placement.restic_dir).expect("tree");
        std::fs::write(&placement.exe, b"agent").expect("exe");
        let fake = Fake::new(true);

        let outcome = prepare(&placement, &placement.exe, &fake).expect("prepare");

        assert_eq!(outcome, Outcome::default());
        assert!(fake.events.borrow().is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_in_07_restic_moves_from_the_newest_old_location_and_the_older_copy_goes() {
        let base = temp_dir("two-old");
        let source = base.join("agent-download");
        std::fs::write(&source, b"agent").expect("source");
        let newer = base.join("program-bin");
        let older = base.join("data-bin");
        for (dir, bytes) in [(&newer, b"restic newer"), (&older, b"restic older")] {
            std::fs::create_dir_all(dir).expect("legacy");
            std::fs::write(dir.join(RESTIC_FILE_NAME), bytes).expect("old restic");
        }
        let placement = Placement::under(&base.join("program"), vec![newer.clone(), older.clone()]);

        let outcome = prepare(&placement, &source, &Fake::new(false)).expect("migrate");

        assert!(outcome.moved_restic);
        assert_eq!(
            std::fs::read(placement.restic_dir.join(RESTIC_FILE_NAME)).expect("moved"),
            b"restic newer"
        );
        assert!(
            !newer.exists() && !older.exists(),
            "both old directories go"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_in_07_the_service_start_migrates_restic_without_the_agent_copy() {
        let base = temp_dir("start");
        let legacy = base.join("old-bin");
        std::fs::create_dir_all(&legacy).expect("legacy");
        std::fs::write(legacy.join(RESTIC_FILE_NAME), b"restic 1").expect("old restic");
        let placement = Placement::under(&base.join("program"), vec![legacy.clone()]);

        assert!(migrate_restic(&placement).expect("migrate"));

        assert_eq!(
            std::fs::read(placement.restic_dir.join(RESTIC_FILE_NAME)).expect("moved"),
            b"restic 1"
        );
        assert!(!legacy.exists());
        assert!(
            !migrate_restic(&placement).expect("again"),
            "nothing left to move"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
