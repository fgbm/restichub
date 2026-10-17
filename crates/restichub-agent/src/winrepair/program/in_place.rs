//! The repair in place of a program tree that another process keeps open (FR-IN-07, T20).
//!
//! An open handle below a directory stops its move, so the repair gives each level from the open
//! one down to the program directory, and each entry in it, the access control of a new tree with
//! Administrators as the owner. Each program is deleted, or renamed when its image is mapped, so
//! the caller writes a new file that no old handle reaches. A name that the agent does not know
//! stops the repair: it can be a library that the service would load. A handle opened before keeps
//! its access until it closes and can still add a file, so the caller checks the tree before the
//! service starts until a clean run after a restart of the computer.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use super::super::Action;
use crate::winsec::{Entry, Exposure, Rights};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Agent,
    Restic,
    /// A file of an interrupted copy or download. The agent never runs it.
    Staged,
    /// A program that an earlier repair renamed. Windows removes it at the restart.
    SetAside,
    /// A text file of the agent package that a manual install copied.
    PackageFile,
    LegacyBin,
}

const PACKAGE_FILES: [&str; 3] = ["LICENSE", "THIRD_PARTY_NOTICES", "SHA256SUMS"];

pub(super) fn repair(open: &Path, program: &Path, log: &mut dyn FnMut(Action)) -> Result<()> {
    let mut levels: Vec<&Path> = program
        .ancestors()
        .take_while(|level| level.starts_with(open))
        .collect();
    levels.reverse();
    for level in levels {
        own_dir(level, log)?;
    }
    repair_dir(program, false, log)
}

/// Refuses each name in `program` and its old `bin` that the agent does not know, all in one
/// error; `state` tells what the refusal left as it was.
pub(super) fn refuse_unknown(program: &Path, state: &str) -> Result<()> {
    let mut unknown = Vec::new();
    collect_unknown(program, false, &mut unknown)?;
    if unknown.is_empty() {
        return Ok(());
    }
    bail!(
        "{} holds {}, which the agent does not know, so the agent does not repair {}; {state}; an administrator checks these files and removes them (the files, not the directory), then runs the command again",
        program.display(),
        list(&unknown),
        program.display()
    )
}

fn collect_unknown(dir: &Path, in_bin: bool, unknown: &mut Vec<PathBuf>) -> Result<()> {
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("cannot list {}", dir.display())),
    };
    for item in listing {
        let path = item
            .with_context(|| format!("cannot list {}", dir.display()))?
            .path();
        match kind_of(&path, in_bin) {
            None => unknown.push(path),
            Some(Kind::LegacyBin) if is_plain_dir(&path) => collect_unknown(&path, true, unknown)?,
            Some(_) => {}
        }
    }
    Ok(())
}

fn is_plain_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

fn list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The access comes before the listing, so only a handle opened before can add an entry.
fn own_dir(dir: &Path, log: &mut dyn FnMut(Action)) -> Result<()> {
    let entry = Entry::open_to_own(dir)?
        .with_context(|| format!("{} is gone during the repair", dir.display()))?;
    if !entry.is_dir() || entry.is_reparse_point() {
        bail!(
            "{} is not a plain directory, so the agent does not repair it; an administrator removes it, then runs the command again",
            dir.display()
        );
    }
    entry
        .own_and_protect()
        .with_context(|| format!("cannot protect {}", dir.display()))?;
    log(Action::Protected(dir.to_path_buf()));
    Ok(())
}

fn repair_dir(dir: &Path, in_bin: bool, log: &mut dyn FnMut(Action)) -> Result<()> {
    let mut entries = Vec::new();
    let mut unknown = Vec::new();
    for item in std::fs::read_dir(dir).with_context(|| format!("cannot list {}", dir.display()))? {
        let path = item
            .with_context(|| format!("cannot list {}", dir.display()))?
            .path();
        match kind_of(&path, in_bin) {
            Some(kind) => entries.push((path, kind)),
            None => unknown.push(path),
        }
    }
    if !unknown.is_empty() {
        bail!(
            "{} holds {}, which the agent does not know, so the agent does not repair {}; an administrator checks these files and removes them (the files, not the directory), then runs the command again",
            dir.display(),
            list(&unknown),
            dir.display()
        );
    }
    for (path, kind) in entries {
        match kind {
            Kind::Agent => replace(&path, Action::RemovedAgent, log)?,
            Kind::Restic => replace(&path, Action::RemovedRestic, log)?,
            Kind::Staged => replace(&path, Action::RemovedTemporary, log)?,
            Kind::SetAside => keep_set_aside(&path, log)?,
            Kind::PackageFile => keep_package_file(&path, log)?,
            Kind::LegacyBin => {
                own_dir(&path, log)?;
                repair_dir(&path, true, log)?;
            }
        }
    }
    Ok(())
}

/// A program image that runs can be renamed but not deleted, and a rename keeps it out of the
/// way of the new file.
fn replace(
    path: &Path,
    removed: fn(std::path::PathBuf) -> Action,
    log: &mut dyn FnMut(Action),
) -> Result<()> {
    let deleted = match std::fs::remove_file(path) {
        Ok(()) => {
            log(removed(path.to_path_buf()));
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => error,
    };
    let aside = super::aside_name(path);
    if let Err(error) = std::fs::rename(path, &aside) {
        bail!(
            "cannot replace {}: {deleted}; cannot rename it either: {error}; a process keeps it open without sharing the delete; close that process or restart the computer, then run the command again",
            path.display()
        );
    }
    // The removal is asked first, so a failed protection still leaves nothing after the restart.
    remove_at_restart(&aside);
    protect_file(&aside)?;
    log(Action::SetAside {
        from: path.to_path_buf(),
        to: aside,
    });
    Ok(())
}

/// A file that an earlier repair set aside and a process still runs stays until the restart, with
/// the access of the tree, so the check before the start accepts it.
fn keep_set_aside(path: &Path, log: &mut dyn FnMut(Action)) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {
            log(Action::RemovedMoved(path.to_path_buf()));
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {}
    }
    remove_at_restart(path);
    protect_file(path)?;
    log(Action::Protected(path.to_path_buf()));
    Ok(())
}

fn protect_file(path: &Path) -> Result<()> {
    let Some(entry) = Entry::open_to_own(path)? else {
        return Ok(());
    };
    if entry.is_dir() || entry.is_reparse_point() || entry.links() > 1 {
        bail!(
            "{} is a directory or a link, and a process keeps it open; close that process or restart the computer, then run the command again",
            path.display()
        );
    }
    entry
        .own_and_protect()
        .with_context(|| format!("cannot protect {}", path.display()))
}

#[allow(unsafe_code)]
fn remove_at_restart(path: &Path) {
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
    let wide = crate::winsec::wide_path(path);
    // SAFETY: the path is a null-terminated UTF-16 string, and a null target asks for a removal.
    let ok = unsafe { MoveFileExW(wide.as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) };
    if ok == 0 {
        tracing::warn!(
            path = %path.display(),
            error = %std::io::Error::last_os_error(),
            "cannot ask Windows to remove the file at the restart"
        );
    }
}

/// The agent does not need these files, so they go; one that a process keeps open stays with the
/// protected access.
fn keep_package_file(path: &Path, log: &mut dyn FnMut(Action)) -> Result<()> {
    if std::fs::remove_file(path).is_ok() {
        log(Action::RemovedUnused(path.to_path_buf()));
        return Ok(());
    }
    protect_file(path)?;
    log(Action::Protected(path.to_path_buf()));
    Ok(())
}

/// Refuses a program tree with a name that the agent does not know, a link or an open access,
/// before the service starts from it. Each problem is in the one error.
pub(crate) fn check_tree(program: &Path, root: &Path) -> Result<()> {
    let mut problems = Vec::new();
    let mut levels: Vec<&Path> = program
        .ancestors()
        .take_while(|level| level.starts_with(root))
        .collect();
    levels.reverse();
    for level in levels {
        let Some(entry) = Entry::inspect(level)? else {
            problems.push(format!("{} is gone", level.display()));
            break;
        };
        if !entry.is_dir() || entry.is_reparse_point() {
            problems.push(format!("{} is not a plain directory", level.display()));
            break;
        }
        if entry.exposure(Rights::Change)? != Exposure::Clean {
            problems.push(format!(
                "{} grants another account a change",
                level.display()
            ));
        }
    }
    if problems.is_empty() {
        check_entries(program, false, &mut problems)?;
    }
    if problems.is_empty() {
        return Ok(());
    }
    bail!(
        "{}, so the command does not start the service from {}; a process that kept the directory open during a repair in place can add or change a file there until the computer restarts; an administrator checks these files and removes them, restarts the computer and runs the command again",
        problems.join("; "),
        program.display()
    )
}

fn check_entries(dir: &Path, in_bin: bool, problems: &mut Vec<String>) -> Result<()> {
    for item in std::fs::read_dir(dir).with_context(|| format!("cannot list {}", dir.display()))? {
        let path = item
            .with_context(|| format!("cannot list {}", dir.display()))?
            .path();
        let Some(kind) = kind_of(&path, in_bin) else {
            problems.push(format!("{} is not a file of the agent", path.display()));
            continue;
        };
        let Some(entry) = Entry::inspect(&path)? else {
            continue;
        };
        if entry.is_reparse_point() || (!entry.is_dir() && entry.links() > 1) {
            problems.push(format!("{} is a link", path.display()));
            continue;
        }
        if entry.exposure(Rights::Change)? != Exposure::Clean {
            problems.push(format!(
                "{} grants another account a change",
                path.display()
            ));
        }
        if kind == Kind::LegacyBin && entry.is_dir() {
            drop(entry);
            check_entries(&path, true, problems)?;
        }
    }
    Ok(())
}

fn kind_of(path: &Path, in_bin: bool) -> Option<Kind> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if let Some((original, stamp)) = name.rsplit_once(".untrusted-")
        && !original.is_empty()
        && !stamp.is_empty()
        && stamp.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Some(Kind::SetAside);
    }
    let suffix = std::env::consts::EXE_SUFFIX;
    let restic = format!("restic{suffix}");
    if name == restic
        || name == format!("restic-previous{suffix}")
        || name == format!("restic-new{suffix}")
    {
        return Some(Kind::Restic);
    }
    if is_staged_copy(&name, &restic) {
        return Some(Kind::Staged);
    }
    if in_bin {
        return None;
    }
    let agent = format!("restichub-agent{suffix}");
    if name == agent {
        Some(Kind::Agent)
    } else if is_staged_copy(&name, &agent) {
        Some(Kind::Staged)
    } else if name == "bin" {
        Some(Kind::LegacyBin)
    } else if PACKAGE_FILES
        .iter()
        .any(|file| file.eq_ignore_ascii_case(&name))
    {
        Some(Kind::PackageFile)
    } else {
        None
    }
}

/// The self-install copies `<file>` through `<file>.<process id>.new`.
fn is_staged_copy(name: &str, file: &str) -> bool {
    name.strip_prefix(file)
        .and_then(|rest| rest.strip_prefix('.'))
        .and_then(|rest| rest.strip_suffix(".new"))
        .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit()))
}
