//! The repair of `%ProgramFiles%\ResticHub` that a manual copy of the agent left open (threat T20).
//!
//! A folder made by hand under `%ProgramFiles%` keeps the inherited entries, and on a client
//! edition of Windows the entry of the creator owner becomes a full control for the account of the
//! administrator. A process of that account without elevation can then replace a program that the
//! service runs as LocalSystem. A new access control does not stop it: a handle that the process
//! opened before keeps its access, and the directory of the executable is the first place where
//! Windows looks for a library. So the repair moves the open tree aside and removes it, and the
//! caller makes the directories again for SYSTEM and Administrators only, copies the running agent
//! into them and downloads and verifies restic. An old handle then points to the moved tree, which
//! the agent never uses. When another process keeps a file in the tree open, the tree cannot move
//! and the repair falls back to a repair in place, so the host never stays without its service.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use super::{Action, remove_file};
use crate::paths::ProgramRebuild;
use crate::winsec::{Entry, Exposure, Rights};

mod in_place;

pub(crate) use in_place::{
    check_tree as check_program_tree, check_tree_without as check_program_tree_without,
};

/// The Restart Manager takes a list of files; a tree larger or deeper than this is not a manual
/// copy of the agent, and the first files name the holder well enough. The depth bound keeps a
/// tree that another account made from exhausting the stack.
const MAX_HOLDER_FILES: usize = 4096;
const MAX_HOLDER_DEPTH: usize = 32;

/// Moves the program tree aside when another account could change a level or an entry of it, or
/// repairs it in place when another process keeps it open (FR-IN-07).
///
/// The open level nearest to `root` goes, or `program` when only an entry in it is open. A level
/// of another owner is open too. `stop_service` runs before the move: a running agent or restic
/// keeps the tree in use.
/// The restic record in `record` goes first. `running` is the image of this process and
/// `current_dir` its current directory; neither lets the tree move.
pub(crate) fn rebuild_open_program_tree(
    program: &Path,
    root: &Path,
    record: &Path,
    running: &Path,
    current_dir: Option<&Path>,
    stop_service: &mut dyn FnMut() -> Result<()>,
    log: &mut dyn FnMut(Action),
) -> Result<ProgramRebuild> {
    let Some(open) = open_level(program, root)? else {
        return Ok(ProgramRebuild::Unchanged);
    };
    if is_inside(running, &open) {
        bail!(
            "the agent runs from {}, which another account could change, so the agent cannot replace that directory; copy restichub-agent.exe from the release package to another folder and run the command from there",
            open.display()
        );
    }
    if let Some(current) = current_dir.filter(|current| is_inside(current, &open)) {
        bail!(
            "the current directory {} is in {}, which the command must replace, and an open directory cannot move; change to another directory, for example `cd $env:TEMP`, and run the command again",
            current.display(),
            open.display()
        );
    }
    stop_service().context("cannot stop the service before the program directory is replaced")?;
    // The record goes first, so a failure below never leaves it beside a removed restic.
    if record.exists() {
        remove_file(record)?;
        log(Action::RemovedRestic(record.to_path_buf()));
    }
    let aside = aside_name(&open);
    if let Err(error) = move_with_retry(&open, &aside) {
        let holders = name_holders(&open);
        log(Action::RepairingInPlace {
            dir: open.clone(),
            cause: error.to_string(),
            holders: holders.clone(),
        });
        in_place::repair(&open, program, log).with_context(|| {
            format!(
                "cannot move {} ({error}; open in: {holders}), and the repair in place stopped; the service stays stopped until the command succeeds",
                open.display()
            )
        })?;
        return Ok(ProgramRebuild::RepairedInPlace { dir: open, holders });
    }
    log(Action::MovedProgram {
        from: open,
        to: aside.clone(),
    });
    match std::fs::remove_dir_all(&aside) {
        Ok(()) => log(Action::RemovedMoved(aside)),
        Err(error) => log(Action::KeptMoved {
            path: aside,
            cause: error.to_string(),
        }),
    }
    Ok(ProgramRebuild::Moved)
}

fn aside_name(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stamp = time::OffsetDateTime::now_utc().unix_timestamp();
    path.with_file_name(format!("{name}.untrusted-{stamp}"))
}

/// A handle to a directory, such as the current directory of a shell or an Explorer window, is
/// not in the list, so the text says so when the list is empty.
fn name_holders(dir: &Path) -> String {
    let mut files = Vec::new();
    let complete = collect_files(dir, 0, &mut files);
    let holders = match crate::winsec::holders(&files) {
        Ok(holders) => holders,
        Err(error) => return format!("the process is not known: {error:#}"),
    };
    let mut text = if !holders.is_empty() {
        holders
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    } else if complete {
        "no process keeps a file in it open, so a process keeps a directory in it open, for example a shell or an Explorer window".to_owned()
    } else {
        "no process keeps one of the checked files open".to_owned()
    };
    if !complete {
        text.push_str(&format!(
            "; not all files were checked, only the first {}",
            files.len()
        ));
    }
    text
}

/// Never follows a link, so the list stays inside the tree. Returns whether it saw each file.
fn collect_files(dir: &Path, depth: usize, files: &mut Vec<PathBuf>) -> bool {
    if depth >= MAX_HOLDER_DEPTH {
        return false;
    }
    let Ok(listing) = std::fs::read_dir(dir) else {
        return true;
    };
    let mut complete = true;
    for item in listing.flatten() {
        if files.len() >= MAX_HOLDER_FILES {
            return false;
        }
        let path = item.path();
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {
                complete &= collect_files(&path, depth + 1, files);
            }
            Ok(metadata) if metadata.is_file() => files.push(path),
            _ => {}
        }
    }
    complete
}

/// A scanner, or the image of a service that just stopped, can keep a file open for a moment.
fn move_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    const ATTEMPTS: u32 = 20;
    let mut attempt = 1;
    loop {
        match std::fs::rename(from, to) {
            Err(_) if attempt < ATTEMPTS => {
                std::thread::sleep(std::time::Duration::from_millis(250));
                attempt += 1;
            }
            result => return result,
        }
    }
}

fn open_level(program: &Path, root: &Path) -> Result<Option<PathBuf>> {
    if !program.starts_with(root) {
        return Ok(None);
    }
    let mut levels: Vec<&Path> = program
        .ancestors()
        .take_while(|level| level.starts_with(root))
        .collect();
    levels.reverse();
    for level in levels {
        let Some(entry) = Entry::open(level)? else {
            return Ok(None);
        };
        // The normal check refuses a level that is not a plain directory.
        if !entry.is_dir() || entry.is_reparse_point() {
            return Ok(None);
        }
        // A foreign owner can change the access at any time, also of a level below it. The tree
        // holds no data of the host, so it goes or takes Administrators as the owner.
        if entry
            .exposure(Rights::Change)
            .with_context(|| format!("cannot check {}", level.display()))?
            != Exposure::Clean
        {
            return Ok(Some(level.to_path_buf()));
        }
    }
    Ok(has_open_entry(program, 0)?.then(|| program.to_path_buf()))
}

/// A link or a second name can carry content that another account chose. A tree deeper than the
/// bound is not a copy of the agent, so it counts as open.
fn has_open_entry(dir: &Path, depth: usize) -> Result<bool> {
    if depth >= MAX_HOLDER_DEPTH {
        return Ok(true);
    }
    let listing =
        std::fs::read_dir(dir).with_context(|| format!("cannot list {}", dir.display()))?;
    for item in listing {
        let path = item
            .with_context(|| format!("cannot list {}", dir.display()))?
            .path();
        let Some(entry) = Entry::open(&path)? else {
            continue;
        };
        if entry.is_reparse_point()
            || (!entry.is_dir() && entry.links() > 1)
            || entry
                .exposure(Rights::Change)
                .with_context(|| format!("cannot check {}", path.display()))?
                != Exposure::Clean
        {
            return Ok(true);
        }
        let is_dir = entry.is_dir();
        drop(entry);
        if is_dir && has_open_entry(&path, depth + 1)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_inside(path: &Path, dir: &Path) -> bool {
    match (path.canonicalize(), dir.canonicalize()) {
        (Ok(path), Ok(dir)) => path.starts_with(dir),
        _ => path.starts_with(dir),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::os::windows::fs::OpenOptionsExt as _;
    use std::os::windows::io::AsRawHandle as _;

    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ADD_FILE, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
    };

    use super::*;
    use crate::winsec::check_admin_only;
    use crate::winsec::tests::{current_user_sid, grant, grant_sid, set_owner, temp};

    struct Tree {
        base: PathBuf,
        root: PathBuf,
        program: PathBuf,
        record: PathBuf,
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    /// A tree that a manual copy made: the agent, restic, the package files and the record.
    fn tree(name: &str) -> Tree {
        let base = temp(name);
        crate::winsec::create_protected_dir(&base).expect("cannot create the base");
        let root = base.join("ResticHub");
        let program = root.join("Agent");
        crate::winsec::create_protected_tree(&program, &root).expect("cannot create the tree");
        for file in ["restichub-agent.exe", "restic.exe", "LICENSE", "SHA256SUMS"] {
            crate::winsec::create_protected_file(&program.join(file), b"x").expect("cannot write");
        }
        let config = base.join("config");
        crate::winsec::create_protected_dir(&config).expect("cannot create the config");
        let record = config.join("restic.json");
        std::fs::write(&record, b"{}").expect("cannot write the record");
        Tree {
            base,
            root,
            program,
            record,
        }
    }

    struct Run {
        result: Result<ProgramRebuild>,
        actions: Vec<Action>,
        stops: u32,
    }

    fn rebuild(tree: &Tree, running: &Path) -> Run {
        rebuild_from(tree, running, None)
    }

    fn rebuild_from(tree: &Tree, running: &Path, current_dir: Option<&Path>) -> Run {
        let mut actions = Vec::new();
        let mut stops = 0;
        let result = rebuild_open_program_tree(
            &tree.program,
            &tree.root,
            &tree.record,
            running,
            current_dir,
            &mut || {
                stops += 1;
                Ok(())
            },
            &mut |action| actions.push(action),
        );
        Run {
            result,
            actions,
            stops,
        }
    }

    fn elsewhere(tree: &Tree) -> PathBuf {
        tree.base.join("package").join("restichub-agent.exe")
    }

    fn open_dir(path: &Path, share: u32) -> std::fs::File {
        std::fs::OpenOptions::new()
            .access_mode(FILE_ADD_FILE)
            .share_mode(share)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .expect("cannot open the directory")
    }

    #[allow(unsafe_code)]
    fn file_id(file: &std::fs::File) -> Option<(u32, u32, u32)> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: the handle is open and the out parameter is a local.
        let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) };
        (ok != 0).then_some((
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ))
    }

    #[test]
    fn fr_in_07_an_install_directory_that_the_admin_account_could_change_is_made_again() {
        let tree = tree("program-open");
        // The entry of the creator owner on a client edition: the account of the administrator.
        let user = current_user_sid().expect("cannot read the user SID");
        grant_sid(&tree.root, &user, "(OI)(CI)(F)").expect("cannot add the entry");
        grant_sid(&tree.program, &user, "(OI)(CI)(F)").expect("cannot add the entry");
        std::fs::write(tree.program.join("version.dll"), b"x").expect("cannot plant");
        // A process of that account keeps a handle that may add a file after any repair. A handle
        // below the moved level would stop the move, see the next tests.
        let held = open_dir(
            &tree.root,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        );

        let run = rebuild(&tree, &elsewhere(&tree));
        assert_eq!(
            run.result.expect("the repair must succeed"),
            ProgramRebuild::Moved
        );
        assert_eq!(run.stops, 1, "the service stops before the move");
        let [removed_record, Action::MovedProgram { from, to }, ..] = run.actions.as_slice() else {
            panic!("unexpected actions: {:?}", run.actions);
        };
        assert_eq!(from, &tree.root);
        assert!(
            to.to_string_lossy().contains("ResticHub.untrusted-"),
            "{}",
            to.display()
        );
        assert_eq!(removed_record, &Action::RemovedRestic(tree.record.clone()));
        assert!(!tree.root.exists(), "the open tree must go");
        assert!(!tree.record.exists(), "the record of restic must go");

        crate::winsec::create_protected_tree(&tree.program, &tree.root)
            .expect("the new tree is made for SYSTEM and Administrators");
        check_admin_only(&tree.program).expect("the new directory is protected");
        let new = open_dir(&tree.root, FILE_SHARE_READ | FILE_SHARE_WRITE);
        assert_ne!(
            file_id(&held),
            file_id(&new),
            "the old handle must not reach the new directory"
        );
        let run = rebuild(&tree, &elsewhere(&tree));
        assert_eq!(
            run.result.expect("a second run must succeed"),
            ProgramRebuild::Unchanged
        );
        assert!(run.actions.is_empty(), "{:?}", run.actions);
        assert_eq!(run.stops, 0);
    }

    #[test]
    fn fr_in_07_a_protected_install_directory_is_not_changed() {
        let tree = tree("program-clean");
        let run = rebuild(&tree, &elsewhere(&tree));
        assert_eq!(
            run.result.expect("the repair must succeed"),
            ProgramRebuild::Unchanged
        );
        assert!(run.actions.is_empty(), "{:?}", run.actions);
        assert_eq!(run.stops, 0, "a protected tree keeps the service running");
        assert!(tree.program.join("restic.exe").exists());
        assert!(tree.record.exists());
    }

    #[test]
    fn fr_in_07_a_writable_restic_in_a_protected_directory_makes_the_directory_again() {
        let tree = tree("program-restic");
        grant(&tree.program.join("restic.exe"), "(R,W)").expect("cannot add the entry");
        let run = rebuild(&tree, &elsewhere(&tree));
        assert_eq!(
            run.result.expect("the repair must succeed"),
            ProgramRebuild::Moved
        );
        assert!(
            run.actions
                .iter()
                .any(|action| matches!(action, Action::MovedProgram { from, .. } if *from == tree.program)),
            "{:?}",
            run.actions
        );
        assert!(tree.root.exists(), "a protected root stays");
        assert!(!tree.program.exists());
        assert!(!tree.record.exists());
    }

    #[test]
    fn fr_in_07_the_agent_does_not_replace_the_directory_it_runs_from() {
        let tree = tree("program-running");
        grant(&tree.program, "(OI)(CI)(M)").expect("cannot add the entry");
        let run = rebuild(&tree, &tree.program.join("restichub-agent.exe"));
        let error = format!("{:#}", run.result.expect_err("the running image is inside"));
        assert!(error.contains("from the release package"), "{error}");
        assert_eq!(run.stops, 0);
        assert!(tree.program.join("restic.exe").exists());
    }

    #[test]
    fn fr_in_07_a_current_directory_in_the_tree_stops_before_the_service_stops() {
        let tree = tree("program-current");
        grant(&tree.root, "(OI)(CI)(M)").expect("cannot add the entry");
        let run = rebuild_from(&tree, &elsewhere(&tree), Some(&tree.program));
        let error = format!(
            "{:#}",
            run.result.expect_err("the current directory is inside")
        );
        assert!(error.contains("change to another directory"), "{error}");
        assert_eq!(run.stops, 0, "the service keeps running");
        assert!(run.actions.is_empty(), "{:?}", run.actions);
        assert!(tree.program.join("restic.exe").exists());
        assert!(tree.record.exists());

        let run = rebuild_from(&tree, &elsewhere(&tree), Some(&tree.base));
        assert_eq!(
            run.result
                .expect("a directory above the tree does not stop the move"),
            ProgramRebuild::Moved
        );
    }

    #[test]
    fn fr_in_07_a_program_that_cannot_be_replaced_stops_with_its_holder() {
        let tree = tree("program-in-use");
        grant(&tree.program, "(OI)(CI)(M)").expect("cannot add the entry");
        // Without the delete sharing, restic can be neither removed nor renamed.
        let restic = hold(&tree.program.join("restic.exe"));
        let run = rebuild(&tree, &elsewhere(&tree));
        let error = format!("{:#}", run.result.expect_err("a program in use"));
        assert!(error.contains(&this_process()), "{error}");
        assert!(error.contains("restart the computer"), "{error}");
        assert!(error.contains("the service stays stopped"), "{error}");
        assert_eq!(run.stops, 1, "the service stops first");
        assert!(
            !tree.record.exists(),
            "the record goes first, so the next run installs restic again"
        );
        drop(restic);
    }

    #[test]
    fn fr_in_07_a_record_that_cannot_be_removed_leaves_restic_in_place() {
        let tree = tree("program-record-held");
        grant(&tree.program, "(OI)(CI)(M)").expect("cannot add the entry");
        let record = hold(&tree.record);
        let run = rebuild(&tree, &elsewhere(&tree));
        let error = format!("{:#}", run.result.expect_err("a held record"));
        assert!(error.contains("restic.json"), "{error}");
        assert_eq!(run.stops, 1);
        assert!(tree.record.exists());
        assert!(
            tree.program.join("restic.exe").exists(),
            "the record never outlives restic"
        );
        drop(record);
    }

    /// Opens a file as a FileStream with FileShare.Read does.
    fn hold(path: &Path) -> std::fs::File {
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)
            .expect("cannot hold the file")
    }

    fn this_process() -> String {
        format!("PID {}", std::process::id())
    }

    #[test]
    fn fr_in_07_a_tree_that_another_process_keeps_open_is_repaired_in_place() {
        let tree = tree("program-held");
        let user = current_user_sid().expect("cannot read the user SID");
        grant_sid(&tree.root, &user, "(OI)(CI)(F)").expect("cannot add the entry");
        let license = tree.program.join("LICENSE");
        grant_sid(&license, &user, "(F)").expect("cannot add the entry");
        let held = hold(&license);

        let run = rebuild(&tree, &elsewhere(&tree));
        let rebuilt = run.result.expect("the repair in place must succeed");
        let ProgramRebuild::RepairedInPlace { dir, holders } = &rebuilt else {
            panic!("unexpected result: {rebuilt:?}");
        };
        assert_eq!(dir, &tree.root);
        assert!(holders.contains(&this_process()), "{holders}");
        assert!(
            rebuilt
                .warning()
                .is_some_and(|warning| warning.contains(&this_process())
                    && warning.contains("restart the computer and run install-service again")
                    && warning.contains("starts only by hand")),
            "{rebuilt:?}"
        );
        assert_eq!(run.stops, 1);
        assert!(
            matches!(
                run.actions.as_slice(),
                [
                    Action::RemovedRestic(_),
                    Action::RepairingInPlace { .. },
                    ..
                ]
            ),
            "{:?}",
            run.actions
        );
        for level in [&tree.root, &tree.program] {
            check_admin_only(level).expect("each level is protected");
        }
        check_admin_only(&license).expect("the held file is protected");
        assert!(license.exists(), "the held file stays");
        for gone in ["restichub-agent.exe", "restic.exe", "SHA256SUMS"] {
            assert!(!tree.program.join(gone).exists(), "{gone} must go");
        }
        assert!(!tree.record.exists(), "the record of restic must go");

        // The caller copies the agent; a file that appears after the repair stops the start.
        crate::winsec::create_protected_file(&tree.program.join("restichub-agent.exe"), b"x")
            .expect("cannot copy the agent");
        check_program_tree(&tree.program, &tree.root).expect("the repaired tree passes");
        std::fs::write(tree.program.join("version.dll"), b"x").expect("cannot plant");
        std::fs::write(tree.program.join("dbghelp.dll"), b"x").expect("cannot plant");
        let error = format!(
            "{:#}",
            check_program_tree(&tree.program, &tree.root)
                .expect_err("a new library must stop the start")
        );
        assert!(error.contains("version.dll"), "{error}");
        assert!(error.contains("dbghelp.dll"), "{error}");
        drop(held);
    }

    #[test]
    fn fr_in_07_the_repair_in_place_removes_each_name_that_the_agent_does_not_know() {
        let tree = tree("program-held-unknown");
        grant(&tree.program, "(OI)(CI)(M)").expect("cannot add the entry");
        let planted = [
            tree.program.join("version.dll"),
            tree.program.join("restichub-agent-1.0"),
            tree.program.join("bin").join("restic.cmd"),
        ];
        std::fs::write(&planted[0], b"x").expect("cannot plant");
        std::fs::create_dir(&planted[1]).expect("cannot plant");
        std::fs::write(planted[1].join("restichub-agent.exe"), b"x").expect("cannot plant");
        std::fs::create_dir(tree.program.join("bin")).expect("cannot make bin");
        std::fs::write(&planted[2], b"x").expect("cannot plant");
        let held = hold(&tree.program.join("LICENSE"));
        let run = rebuild(&tree, &elsewhere(&tree));
        assert!(
            matches!(
                run.result.expect("the repair in place must succeed"),
                ProgramRebuild::RepairedInPlace { .. }
            ),
            "{:?}",
            run.actions
        );
        for path in &planted {
            assert!(!path.exists(), "{} must go", path.display());
            assert!(
                run.actions.contains(&Action::RemovedUnused(path.clone())),
                "{}: {:?}",
                path.display(),
                run.actions
            );
        }
        crate::winsec::create_protected_file(&tree.program.join("restichub-agent.exe"), b"x")
            .expect("cannot copy the agent");
        check_program_tree(&tree.program, &tree.root).expect("the repaired tree passes");
        drop(held);
    }

    #[test]
    fn fr_in_07_an_unknown_file_in_a_protected_tree_is_left_alone() {
        let tree = tree("program-clean-unknown");
        crate::winsec::create_protected_file(&tree.program.join("Thumbs.db"), b"x")
            .expect("cannot write");
        let run = rebuild(&tree, &elsewhere(&tree));
        assert_eq!(
            run.result.expect("a protected tree passes"),
            ProgramRebuild::Unchanged
        );
        assert_eq!(run.stops, 0);
    }

    #[test]
    fn fr_in_07_a_set_aside_program_that_still_runs_is_protected() {
        let tree = tree("program-set-aside");
        let user = current_user_sid().expect("cannot read the user SID");
        grant_sid(&tree.program, &user, "(OI)(CI)(M)").expect("cannot add the entry");
        let aside = tree.program.join("restichub-agent.exe.untrusted-1");
        std::fs::write(&aside, b"x").expect("cannot write");
        grant_sid(&aside, &user, "(M)").expect("cannot add the entry");
        let aside_held = hold(&aside);
        let license_held = hold(&tree.program.join("LICENSE"));
        let run = rebuild(&tree, &elsewhere(&tree));
        assert!(
            matches!(
                run.result.expect("the repair in place must succeed"),
                ProgramRebuild::RepairedInPlace { .. }
            ),
            "{:?}",
            run.actions
        );
        assert!(aside.exists(), "a running program stays until the restart");
        check_admin_only(&aside).expect("the set-aside program is protected");
        crate::winsec::create_protected_file(&tree.program.join("restichub-agent.exe"), b"x")
            .expect("cannot copy the agent");
        check_program_tree(&tree.program, &tree.root).expect("the repaired tree passes");
        drop(aside_held);
        drop(license_held);
    }

    #[test]
    fn fr_in_07_a_tree_deeper_than_the_bound_is_not_listed_to_the_end() {
        let tree = tree("program-deep");
        let mut deep = tree.program.clone();
        for _ in 0..=MAX_HOLDER_DEPTH {
            deep = deep.join("d");
        }
        std::fs::create_dir_all(&deep).expect("cannot make the deep tree");
        std::fs::write(deep.join("held.txt"), b"x").expect("cannot write");
        let mut files = Vec::new();
        assert!(!collect_files(&tree.root, 0, &mut files));
        assert!(!files.iter().any(|file| file.ends_with("held.txt")));
        let text = name_holders(&tree.root);
        assert!(text.contains("not all files were checked"), "{text}");
        assert!(
            has_open_entry(&tree.program, 0).expect("the check must pass"),
            "a deep tree counts as open"
        );
    }

    #[test]
    fn fr_in_07_the_checks_of_the_agent_leave_no_handle_in_the_tree() {
        let tree = tree("program-own-handles");
        grant(&tree.program.join("restic.exe"), "(R,W)").expect("cannot add the entry");
        let mut files = Vec::new();
        collect_files(&tree.root, 0, &mut files);
        assert_eq!(
            open_level(&tree.program, &tree.root).expect("the check must pass"),
            Some(tree.program.clone())
        );
        let holders = crate::winsec::holders(&files).expect("the list must be read");
        assert!(holders.is_empty(), "{holders:?}");
        let aside = aside_name(&tree.root);
        std::fs::rename(&tree.root, &aside).expect("the tree moves at the first attempt");
    }

    #[test]
    fn fr_in_07_a_level_of_a_foreign_owner_is_made_again() {
        let tree = tree("program-owner");
        let user = current_user_sid().expect("cannot read the user SID");
        set_owner(&tree.root, &user).expect("cannot set the owner");
        let run = rebuild(&tree, &elsewhere(&tree));
        assert_eq!(
            run.result.expect("a foreign owner is repaired"),
            ProgramRebuild::Moved
        );
        assert!(!tree.root.exists());
        assert!(!tree.record.exists());
    }

    #[test]
    fn fr_in_07_a_level_of_a_foreign_owner_that_a_process_keeps_open_takes_a_new_owner() {
        let tree = tree("program-owner-held");
        let user = current_user_sid().expect("cannot read the user SID");
        set_owner(&tree.program, &user).expect("cannot set the owner");
        let held = hold(&tree.program.join("LICENSE"));
        let run = rebuild(&tree, &elsewhere(&tree));
        assert!(
            matches!(
                run.result.expect("a foreign owner is repaired in place"),
                ProgramRebuild::RepairedInPlace { .. }
            ),
            "{:?}",
            run.actions
        );
        check_admin_only(&tree.program).expect("Administrators own the level now");
        drop(held);
    }
}
