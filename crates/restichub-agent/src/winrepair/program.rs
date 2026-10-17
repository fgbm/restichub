//! The repair of `%ProgramFiles%\ResticHub` that a manual copy of the agent left open (threat T20).
//!
//! A folder made by hand under `%ProgramFiles%` keeps the inherited entries, and on a client
//! edition of Windows the entry of the creator owner becomes a full control for the account of the
//! administrator. A process of that account without elevation can then replace a program that the
//! service runs as LocalSystem. The repair gives each open entry the descriptor of a new tree:
//! SYSTEM and Administrators only, with Administrators as the owner. Users get no read: the service
//! runs as LocalSystem and every command of the agent runs elevated, so nothing without elevation
//! needs the files, and a repaired tree equals a new one.
//!
//! A restic in an open directory is removed with its record, so the install downloads and verifies
//! it again. The agent executable is replaced by the running image when it differs, so it only gets
//! the protection. A name that the agent does not know stops the repair of an open directory: it
//! can be a planted library that the agent would load.

use std::path::Path;

use anyhow::{Context as _, Result, bail};

use super::{Action, Kind as ConfigKind, check_entry, remove_file};
use crate::winsec::{Entry, Exposure, Rights};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Program,
    LegacyBin,
    Agent,
    Restic,
    /// A file of an interrupted copy or download. The agent never runs it.
    Staged,
    /// A text file of the agent package that a manual install copied.
    PackageFile,
}

const PACKAGE_FILES: [&str; 3] = ["LICENSE", "THIRD_PARTY_NOTICES", "SHA256SUMS"];

/// Repairs the levels from `root` down to `program` and the files of the agent in them (FR-IN-07).
///
/// A level that another account owns or that is a reparse point is left to the normal check, which
/// refuses it. When a restic goes, `record` goes too. `log` gets each change.
pub(crate) fn repair_program_tree(
    program: &Path,
    root: &Path,
    record: &Path,
    log: &mut dyn FnMut(Action),
) -> Result<()> {
    if !program.starts_with(root) {
        return Ok(());
    }
    let mut levels: Vec<&Path> = program
        .ancestors()
        .take_while(|level| level.starts_with(root))
        .collect();
    levels.reverse();
    let mut repair = Repair {
        program,
        log,
        restic_removed: false,
    };
    let mut in_open = false;
    let mut result = Ok(());
    for level in levels {
        match repair.level(level, in_open) {
            Ok(open) => in_open = open,
            Err(error) => {
                result = Err(error);
                break;
            }
        }
    }
    if repair.restic_removed && record.exists() {
        remove_file(record)?;
        (repair.log)(Action::RemovedRestic(record.to_path_buf()));
    }
    result
}

struct Repair<'a> {
    program: &'a Path,
    log: &'a mut dyn FnMut(Action),
    restic_removed: bool,
}

impl Repair<'_> {
    /// `in_open` says that another account could change the directory that holds `level`. Returns
    /// whether another account could change `level` before the repair.
    fn level(&mut self, level: &Path, in_open: bool) -> Result<bool> {
        let Some(entry) = Entry::open(level)? else {
            return Ok(in_open);
        };
        if !entry.is_dir() || entry.is_reparse_point() {
            return Ok(in_open);
        }
        let granted = match entry
            .exposure(Rights::Change)
            .with_context(|| format!("cannot check {}", level.display()))?
        {
            Exposure::ForeignOwner if in_open => bail!(
                "{} is owned by an account that is not SYSTEM or Administrators, so the agent does not use it; an administrator checks it and removes it, then runs the command again",
                level.display()
            ),
            Exposure::ForeignOwner => return Ok(in_open),
            Exposure::ForeignGrant => true,
            Exposure::Clean => false,
        };
        if granted {
            // The DACL comes before the listing, so no entry can appear after the check.
            entry
                .protect()
                .with_context(|| format!("cannot protect {}", level.display()))?;
            drop(entry);
            Entry::own(level)?;
            (self.log)(Action::Protected(level.to_path_buf()));
        } else {
            drop(entry);
        }
        let open = granted || in_open;
        let mut entries = Vec::new();
        let listing =
            std::fs::read_dir(level).with_context(|| format!("cannot list {}", level.display()))?;
        for item in listing {
            let path = item
                .with_context(|| format!("cannot list {}", level.display()))?
                .path();
            let kind = self.kind_of(level, &path);
            entries.push((path, kind));
        }
        // A program that another account could have replaced goes before a refusal below.
        for (path, kind) in &entries {
            match kind {
                Some(Kind::Restic) => self.remove_restic(path, open)?,
                Some(Kind::Staged) => self.remove_staged(path, open)?,
                _ => {}
            }
        }
        if open && let Some((path, _)) = entries.iter().find(|(_, kind)| kind.is_none()) {
            bail!(
                "{} holds {}, which is not a file of the agent, so the agent does not repair {}; an administrator checks that file and removes it (the file, not the directory), then runs the command again",
                level.display(),
                path.display(),
                level.display()
            );
        }
        for (path, kind) in entries {
            match kind {
                Some(Kind::LegacyBin) => {
                    self.level(&path, open)?;
                }
                Some(Kind::Agent | Kind::PackageFile) => self.file(&path)?,
                _ => {}
            }
        }
        Ok(open)
    }

    fn kind_of(&self, level: &Path, path: &Path) -> Option<Kind> {
        let name = path.file_name()?.to_str()?.to_ascii_lowercase();
        if level != self.program {
            let below_program = level.parent() == Some(self.program);
            return if path == self.program {
                Some(Kind::Program)
            } else if below_program {
                restic_kind(&name)
            } else {
                None
            };
        }
        let agent = format!("restichub-agent{}", std::env::consts::EXE_SUFFIX);
        if name == agent {
            Some(Kind::Agent)
        } else if name == "bin" {
            Some(Kind::LegacyBin)
        } else if PACKAGE_FILES
            .iter()
            .any(|file| file.eq_ignore_ascii_case(&name))
        {
            Some(Kind::PackageFile)
        } else if is_staged_copy(&name, &agent) {
            Some(Kind::Staged)
        } else {
            restic_kind(&name)
        }
    }

    fn remove_restic(&mut self, path: &Path, open: bool) -> Result<()> {
        let Some(entry) = Entry::open(path)? else {
            return Ok(());
        };
        check_entry(&entry, path, ConfigKind::Agent)?;
        if !open && entry.exposure(Rights::Change)? == Exposure::Clean {
            return Ok(());
        }
        drop(entry);
        remove_file(path)?;
        self.restic_removed = true;
        (self.log)(Action::RemovedRestic(path.to_path_buf()));
        Ok(())
    }

    fn remove_staged(&mut self, path: &Path, open: bool) -> Result<()> {
        if !open {
            let Some(entry) = Entry::open(path)? else {
                return Ok(());
            };
            if !entry.is_reparse_point()
                && entry.links() <= 1
                && entry.exposure(Rights::Change)? == Exposure::Clean
            {
                return Ok(());
            }
        }
        remove_file(path)?;
        (self.log)(Action::RemovedTemporary(path.to_path_buf()));
        Ok(())
    }

    fn file(&mut self, path: &Path) -> Result<()> {
        let Some(entry) = Entry::open(path)? else {
            return Ok(());
        };
        check_entry(&entry, path, ConfigKind::Agent)?;
        if entry.is_dir() {
            bail!(
                "{} is a directory, so the agent does not use it; an administrator checks it and removes it, then runs the command again",
                path.display()
            );
        }
        if entry.exposure(Rights::Change)? == Exposure::ForeignGrant {
            entry
                .protect()
                .with_context(|| format!("cannot protect {}", path.display()))?;
            drop(entry);
            Entry::own(path)?;
            (self.log)(Action::Protected(path.to_path_buf()));
        }
        Ok(())
    }
}

fn restic_kind(name: &str) -> Option<Kind> {
    let suffix = std::env::consts::EXE_SUFFIX;
    if name == format!("restic{suffix}") || name == format!("restic-previous{suffix}") {
        Some(Kind::Restic)
    } else if name == format!("restic-new{suffix}")
        || is_staged_copy(name, &format!("restic{suffix}"))
    {
        Some(Kind::Staged)
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::path::PathBuf;

    use super::*;
    use crate::winsec::check_admin_only;
    use crate::winsec::tests::{current_user_sid, grant, grant_sid, owner_of, temp};

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
        std::fs::create_dir_all(&program).expect("cannot create the program directory");
        for file in ["restichub-agent.exe", "restic.exe", "LICENSE", "SHA256SUMS"] {
            std::fs::write(program.join(file), b"x").expect("cannot write");
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

    fn repair(tree: &Tree) -> (Result<()>, Vec<Action>) {
        let mut actions = Vec::new();
        let result = repair_program_tree(&tree.program, &tree.root, &tree.record, &mut |action| {
            actions.push(action);
        });
        (result, actions)
    }

    #[test]
    fn fr_in_07_an_install_directory_that_the_admin_account_could_change_is_repaired() {
        let tree = tree("program-open");
        // The entry of the creator owner on a client edition: the account of the administrator.
        let user = current_user_sid().expect("cannot read the user SID");
        grant_sid(&tree.root, &user, "(OI)(CI)(F)").expect("cannot add the entry");
        grant_sid(&tree.program, &user, "(OI)(CI)(F)").expect("cannot add the entry");
        let refused = format!(
            "{:#}",
            crate::winsec::create_protected_tree(&tree.program, &tree.root)
                .expect_err("the open tree must be refused before the repair")
        );
        assert!(refused.contains("access mask"), "{refused}");

        let (result, actions) = repair(&tree);
        result.expect("the repair must succeed");

        for level in [&tree.root, &tree.program] {
            check_admin_only(level).expect("the level must be protected");
            assert_eq!(owner_of(level).expect("owner"), "S-1-5-32-544");
        }
        for file in ["restichub-agent.exe", "LICENSE", "SHA256SUMS"] {
            check_admin_only(&tree.program.join(file)).expect("the file must be protected");
        }
        assert!(!tree.program.join("restic.exe").exists(), "restic must go");
        assert!(!tree.record.exists(), "the record of restic must go");
        assert!(actions.contains(&Action::Protected(tree.root.clone())));
        assert!(actions.contains(&Action::Protected(tree.program.clone())));
        assert!(actions.contains(&Action::RemovedRestic(tree.program.join("restic.exe"))));
        assert!(actions.contains(&Action::RemovedRestic(tree.record.clone())));
        crate::winsec::create_protected_tree(&tree.program, &tree.root)
            .expect("the normal check passes now");
        crate::winsec::check_program_directory(&tree.program).expect("no account adds a file");

        let (result, actions) = repair(&tree);
        result.expect("a second repair must succeed");
        assert!(actions.is_empty(), "{actions:?}");
    }

    #[test]
    fn fr_in_07_a_protected_install_directory_is_not_changed() {
        let tree = tree("program-clean");
        let (result, actions) = repair(&tree);
        result.expect("the repair must succeed");
        assert!(actions.is_empty(), "{actions:?}");
        assert!(tree.program.join("restic.exe").exists());
        assert!(tree.record.exists());
    }

    #[test]
    fn fr_in_07_an_unknown_file_in_an_open_install_directory_is_refused_after_restic_goes() {
        let tree = tree("program-unknown");
        let planted = tree.program.join("version.dll");
        std::fs::write(&planted, b"x").expect("cannot write");
        grant(&tree.program, "(OI)(CI)(M)").expect("cannot add the entry");
        let (result, actions) = repair(&tree);
        let error = format!(
            "{:#}",
            result.expect_err("an unknown file must stop the repair")
        );
        assert!(error.contains("version.dll"), "{error}");
        assert!(error.contains("the file, not the directory"), "{error}");
        check_admin_only(&tree.program).expect("the level stays protected after a refusal");
        assert!(!tree.program.join("restic.exe").exists());
        assert!(!tree.record.exists());
        assert!(actions.contains(&Action::Protected(tree.program.clone())));
    }

    #[test]
    fn fr_in_07_a_writable_restic_in_a_protected_directory_is_removed_with_its_record() {
        let tree = tree("program-restic");
        let restic = tree.program.join("restic.exe");
        grant(&restic, "(R,W)").expect("cannot add the entry");
        let (result, actions) = repair(&tree);
        result.expect("the repair must succeed");
        assert_eq!(
            actions,
            vec![
                Action::RemovedRestic(restic),
                Action::RemovedRestic(tree.record.clone())
            ]
        );
    }

    #[test]
    fn fr_in_07_the_legacy_bin_of_an_open_directory_loses_restic() {
        let tree = tree("program-bin");
        let bin = tree.program.join("bin");
        std::fs::create_dir(&bin).expect("cannot create bin");
        std::fs::write(bin.join("restic.exe"), b"x").expect("cannot write");
        std::fs::write(bin.join("restic.exe.42.new"), b"x").expect("cannot write");
        grant(&tree.program, "(OI)(CI)(M)").expect("cannot add the entry");
        let (result, actions) = repair(&tree);
        result.expect("the repair must succeed");
        assert!(!bin.join("restic.exe").exists());
        assert!(!bin.join("restic.exe.42.new").exists());
        assert!(
            actions.contains(&Action::RemovedTemporary(bin.join("restic.exe.42.new"))),
            "{actions:?}"
        );
        check_admin_only(&bin).expect("the legacy directory must be protected");
    }

    #[test]
    fn staged_copies_are_named_by_the_process_id() {
        assert!(is_staged_copy("restic.exe.123.new", "restic.exe"));
        assert!(!is_staged_copy("restic.exe.new", "restic.exe"));
        assert!(!is_staged_copy("restic.exe.12a.new", "restic.exe"));
    }
}
