//! The repair of agent directories that an older build left with a foreign access (threat T20).
//!
//! An older build could leave `%ProgramData%\ResticHub` with the access control of
//! `%ProgramData%`, which lets a normal user create files there. Deleting the directory would
//! delete the agent identity, so an elevated command repairs the access instead. The repair never
//! hands the agent a file whose content another account could write: it removes the trust mode
//! file and the staged rotation files, moves the restore roots file aside, and warns that the
//! identity may be exposed. An identity that another account could change is not repaired: it is
//! left as found or removed, as the caller asks. It refuses an entry that the agent does not know,
//! a link, a file with more than one name, and an entry of a foreign owner. The service never
//! repairs.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::paths::UntrustedIdentity;
use crate::winsec::{Entry, Exposure, Rights};

mod program;

pub(crate) use program::rebuild_open_program_tree;

/// The directory that an older build used as the default restore root. The repair never reads
/// inside it.
const OLD_RESTORE_ROOT: &str = "Restore";

pub(crate) const REENROLL_ADVICE: &str = "register the agent again: create a token with \"Re-register\" on the host page and run `restichub-agent register --force` with it";

/// What the repair did to one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    Protected(PathBuf),
    RemovedStaged(PathBuf),
    RemovedTrustMode(PathBuf),
    MovedRestoreRoots {
        from: PathBuf,
        to: PathBuf,
    },
    IdentityExposed(PathBuf),
    RemovedIdentity(PathBuf),
    RemovedTemporary(PathBuf),
    RemovedLock(PathBuf),
    /// A restic binary or its record in a directory that another account could change.
    RemovedRestic(PathBuf),
    /// A program tree that another account could change, moved aside to be made again.
    MovedProgram {
        from: PathBuf,
        to: PathBuf,
    },
    RemovedMoved(PathBuf),
    KeptMoved {
        path: PathBuf,
        cause: String,
    },
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protected(path) => {
                write!(f, "repaired the access control of {}", path.display())
            }
            Self::RemovedStaged(path) => write!(
                f,
                "removed {}: another account could change it, so the certificate rotation does not use it",
                path.display()
            ),
            Self::RemovedTrustMode(path) => write!(
                f,
                "removed {}: another account could change it, so the trust mode for server scripts is off; to turn it on again, an administrator runs `restichub-agent trust-server-scripts --enable`",
                path.display()
            ),
            Self::MovedRestoreRoots { from, to } => write!(
                f,
                "moved {} to {}: another account could change it, so the extra restore roots are off; an administrator checks the file and moves it back to {} to turn them on again",
                from.display(),
                to.display(),
                from.display()
            ),
            Self::IdentityExposed(path) => write!(
                f,
                "warning: another account could read or change {}, so the agent identity may be exposed; {REENROLL_ADVICE}",
                path.display()
            ),
            Self::RemovedIdentity(path) => write!(
                f,
                "removed {}: another account could change it, so the new registration replaces it",
                path.display()
            ),
            Self::RemovedTemporary(path) => write!(
                f,
                "removed {}: another account could change it; it is a leftover of an interrupted write, and the agent does not read it",
                path.display()
            ),
            Self::RemovedLock(path) => write!(
                f,
                "removed {}: another account could change it or hold it; the agent creates the identity lock again",
                path.display()
            ),
            Self::RemovedRestic(path) => write!(
                f,
                "removed {}: another account could change restic, so the agent downloads and verifies restic again",
                path.display()
            ),
            Self::MovedProgram { from, to } => write!(
                f,
                "moved {} to {}: another account could change it, so the agent makes it again for SYSTEM and Administrators only, copies the running agent into it, and downloads and verifies restic again",
                from.display(),
                to.display()
            ),
            Self::RemovedMoved(path) => write!(f, "removed {}", path.display()),
            Self::KeptMoved { path, cause } => write!(
                f,
                "cannot remove {} ({cause}); the agent does not use it; an administrator removes it after a restart of the computer",
                path.display()
            ),
        }
    }
}

/// Prints the actions of a repair. Without `verbose`, the access control repairs give one line for
/// each top-level directory below `root`, because a restic cache holds thousands of files.
pub(crate) struct Report<'a> {
    root: PathBuf,
    verbose: bool,
    groups: Vec<(PathBuf, usize)>,
    print: &'a mut dyn FnMut(&str),
}

impl<'a> Report<'a> {
    pub(crate) fn new(root: &Path, verbose: bool, print: &'a mut dyn FnMut(&str)) -> Self {
        Self {
            root: root.to_path_buf(),
            verbose,
            groups: Vec::new(),
            print,
        }
    }

    pub(crate) fn action(&mut self, action: Action) {
        match action {
            Action::Protected(path) if !self.verbose => {
                let group = self.group_of(&path);
                match self.groups.iter_mut().find(|(dir, _)| *dir == group) {
                    Some((_, count)) => *count += 1,
                    None => self.groups.push((group, 1)),
                }
            }
            action => (self.print)(&action.to_string()),
        }
    }

    /// Prints the summary lines. The caller runs it after a failed repair too, because the access
    /// control of the entries before the failure is already changed.
    pub(crate) fn finish(self) {
        for (dir, count) in self.groups {
            let entries = if count == 1 { "entry" } else { "entries" };
            (self.print)(&format!(
                "repaired the access control of {count} {entries} under {}",
                dir.display()
            ));
        }
    }

    fn group_of(&self, path: &Path) -> PathBuf {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return path.to_path_buf();
        };
        let mut components = relative.components();
        match (components.next(), components.next()) {
            (None, _) => self.root.clone(),
            (Some(first), Some(_)) => self.root.join(first),
            (Some(first), None) if path.is_dir() => self.root.join(first),
            (Some(_), None) => self.root.clone(),
        }
    }
}

/// The role of a name that the agent keeps in a repaired level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// The configuration directory, a level of its own.
    Config,
    OldRestoreRoot,
    /// A file of a certificate rotation that the identity load moves into place.
    Staged,
    /// A temporary file of an identity write. The identity load never reads it.
    Temporary,
    /// The identity lock. Another account that created it could hold its lock for good.
    Lock,
    TrustMode,
    RestoreRoots,
    Identity,
    Agent,
}

impl Kind {
    fn rights(self) -> Rights {
        if self == Self::Identity {
            Rights::ReadOrChange
        } else {
            Rights::Change
        }
    }
}

/// Repairs the levels from `root` down to `config` that SYSTEM or Administrators own, and the
/// entries of the agent in them (FR-AG-04).
///
/// A level of another owner is refused. A reparse point and a level above `root` are left to the
/// normal check, which refuses them. An identity entry that another account could change, or every
/// identity entry of a configuration directory that another account could change, is handled as
/// `untrusted` says; a removal waits until every check of the tree passed, so a refusal leaves the
/// old identity in place. `log` gets each change and each warning. Returns the identity entries
/// that another account could change and that stay as found.
pub(crate) fn repair_agent_tree(
    config: &Path,
    root: &Path,
    untrusted: UntrustedIdentity,
    log: &mut dyn FnMut(Action),
) -> Result<Vec<PathBuf>> {
    if !config.starts_with(root) {
        return Ok(Vec::new());
    }
    let mut levels: Vec<&Path> = config
        .ancestors()
        .take_while(|level| level.starts_with(root))
        .collect();
    levels.reverse();
    let mut repair = Repair {
        config,
        untrusted,
        log,
        exposure_told: false,
        changeable: Vec::new(),
        doomed: Vec::new(),
    };
    for level in levels {
        repair.level(level)?;
    }
    for (path, is_dir) in std::mem::take(&mut repair.doomed) {
        if is_dir {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("cannot remove {}", path.display()))?;
        } else {
            remove_file(&path)?;
        }
        (repair.log)(Action::RemovedIdentity(path));
    }
    Ok(repair.changeable)
}

/// Refuses an identity that another account could change, through its files or a level that holds
/// them (FR-AG-04, T20).
///
/// The service runs it before it loads the identity. It never repairs: an open level stays a
/// refusal until an administrator runs the repair. It checks the levels, the backup of an
/// interrupted swap that the load can take instead, and every file that the load reads or moves
/// into place, also when `config` is empty. A missing entry passes, so the load reports it.
pub(crate) fn check_identity(config: &Path, root: &Path) -> Result<()> {
    let mut levels: Vec<PathBuf> = config
        .ancestors()
        .take_while(|level| *level == config || level.starts_with(root))
        .map(Path::to_path_buf)
        .collect();
    levels.reverse();
    let backup = crate::identity::sibling(config, ".old");
    levels.push(backup.clone());
    for level in &levels {
        let Some(entry) = Entry::inspect(level)? else {
            continue;
        };
        if entry.is_reparse_point() {
            bail!("{} is a reparse point", level.display());
        }
        refuse_exposure(&entry, level)?;
    }
    let names = crate::identity::loaded_names();
    for dir in [config, backup.as_path()] {
        for (name, _) in &names {
            let path = dir.join(name);
            let Some(entry) = Entry::inspect(&path)? else {
                continue;
            };
            if entry.is_reparse_point() || entry.links() > 1 {
                bail!(
                    "{} is a link, so another path can carry its content",
                    path.display()
                );
            }
            refuse_exposure(&entry, &path)?;
        }
    }
    Ok(())
}

fn refuse_exposure(entry: &Entry, path: &Path) -> Result<()> {
    match entry
        .exposure(Rights::Change)
        .with_context(|| format!("cannot check {}", path.display()))?
    {
        Exposure::Clean => Ok(()),
        Exposure::ForeignGrant => bail!(
            "{} grants a write access to an account that is not SYSTEM or Administrators",
            path.display()
        ),
        Exposure::ForeignOwner => bail!(
            "{} is owned by an account that is not SYSTEM or Administrators",
            path.display()
        ),
    }
}

struct Repair<'a> {
    config: &'a Path,
    untrusted: UntrustedIdentity,
    log: &'a mut dyn FnMut(Action),
    exposure_told: bool,
    changeable: Vec<PathBuf>,
    /// The identity entries that [`UntrustedIdentity::Remove`] removes after the last check.
    doomed: Vec<(PathBuf, bool)>,
}

impl Repair<'_> {
    fn level(&mut self, level: &Path) -> Result<()> {
        let Some(entry) = Entry::open(level)? else {
            return Ok(());
        };
        if !entry.is_dir() || entry.is_reparse_point() {
            return Ok(());
        }
        let is_config = level == self.config;
        let rights = if is_config {
            Rights::ReadOrChange
        } else {
            Rights::Change
        };
        let open = match entry
            .exposure(rights)
            .with_context(|| format!("cannot check {}", level.display()))?
        {
            // The owner can change the access at any time, also of a level below it through a link.
            Exposure::ForeignOwner => bail!(
                "{} is owned by an account that is not SYSTEM or Administrators, so the agent does not repair it; an administrator checks it and makes Administrators its owner, then runs the command again",
                level.display()
            ),
            Exposure::ForeignGrant => true,
            Exposure::Clean => false,
        };
        let changeable =
            open && is_config && entry.exposure(Rights::Change)? == Exposure::ForeignGrant;
        if changeable && self.untrusted == UntrustedIdentity::Keep {
            // A repaired access would hide the grant, so a later install-service would trust
            // whatever that account wrote. Only a new registration replaces the directory.
            self.changeable.push(level.to_path_buf());
        } else if open {
            // The DACL comes before the listing, so a new handle cannot add an entry after the
            // check. A handle opened before keeps its access until it closes (T20 residual).
            entry
                .protect()
                .with_context(|| format!("cannot protect {}", level.display()))?;
            (self.log)(Action::Protected(level.to_path_buf()));
            if is_config && !changeable {
                self.identity_exposed(level);
            }
        }
        drop(entry);
        let known = if is_config {
            config_entries()
        } else {
            root_entries(self.config, level)
        };
        let mut entries = Vec::new();
        let listing =
            std::fs::read_dir(level).with_context(|| format!("cannot list {}", level.display()))?;
        for item in listing {
            let path = item
                .with_context(|| format!("cannot list {}", level.display()))?
                .path();
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let kind = known
                .iter()
                .find(|(known, _)| known.eq_ignore_ascii_case(&name))
                .map(|(_, kind)| *kind);
            entries.push((path, kind));
        }
        // The files whose content the agent would trust are handled first, so a refusal below
        // never leaves them in a level that the normal check now accepts.
        for (path, kind) in &entries {
            match kind {
                Some(Kind::Staged) => self.remove_leftover(path, open, Action::RemovedStaged)?,
                Some(Kind::Temporary) => {
                    self.remove_leftover(path, open, Action::RemovedTemporary)?;
                }
                Some(Kind::Lock) => {
                    self.remove_leftover(path, open, Action::RemovedLock)
                        .with_context(|| {
                            format!(
                                "another process may keep {} open; an administrator stops that process or restarts the computer, then runs the command again",
                                path.display()
                            )
                        })?;
                }
                Some(setting @ (Kind::TrustMode | Kind::RestoreRoots)) => {
                    self.distrust_setting(path, *setting)?;
                }
                _ => {}
            }
        }
        // A level that nobody else could write holds no planted entry, so only an open level
        // refuses an unknown name.
        if open && let Some((path, _)) = entries.iter().find(|(_, kind)| kind.is_none()) {
            bail!(
                "{} holds {}, which is not a file of the agent, so the agent does not repair {}; an administrator checks that file and removes it (the file, not the directory), then runs the command again",
                level.display(),
                path.display(),
                level.display()
            );
        }
        for (path, kind) in entries {
            if let Some(kind @ (Kind::Identity | Kind::Agent)) = kind {
                self.walk(&path, kind, changeable)?;
            }
        }
        Ok(())
    }

    /// An open level could hold a planted file, so its leftovers go even when they look clean.
    fn remove_leftover(
        &mut self,
        path: &Path,
        open: bool,
        action: fn(PathBuf) -> Action,
    ) -> Result<()> {
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
        (self.log)(action(path.to_path_buf()));
        Ok(())
    }

    /// A setting that another account could change turns its feature off (ADR-0037).
    fn distrust_setting(&mut self, path: &Path, kind: Kind) -> Result<()> {
        let Some(entry) = Entry::open(path)? else {
            return Ok(());
        };
        check_entry(&entry, path, kind)?;
        if entry.exposure(Rights::Change)? != Exposure::ForeignGrant {
            return Ok(());
        }
        drop(entry);
        if kind == Kind::TrustMode {
            remove_file(path)?;
            (self.log)(Action::RemovedTrustMode(path.to_path_buf()));
        } else {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let stamp = time::OffsetDateTime::now_utc().unix_timestamp();
            let aside = path.with_file_name(format!("{name}.untrusted-{stamp}"));
            std::fs::rename(path, &aside).with_context(|| {
                format!("cannot move {} to {}", path.display(), aside.display())
            })?;
            (self.log)(Action::MovedRestoreRoots {
                from: path.to_path_buf(),
                to: aside,
            });
        }
        Ok(())
    }

    /// Each directory is protected before its listing, so only a handle opened before the
    /// protection can add a child after the check. `in_changeable` says that another account could change the directory that holds `path`.
    fn walk(&mut self, path: &Path, kind: Kind, in_changeable: bool) -> Result<()> {
        let Some(entry) = Entry::open(path)? else {
            return Ok(());
        };
        check_entry(&entry, path, kind)?;
        if kind == Kind::Identity
            && (in_changeable || entry.exposure(Rights::Change)? == Exposure::ForeignGrant)
        {
            let is_dir = entry.is_dir();
            drop(entry);
            self.untrusted_identity(path, is_dir, in_changeable);
            return Ok(());
        }
        if entry.exposure(kind.rights())? == Exposure::ForeignGrant {
            entry
                .protect()
                .with_context(|| format!("cannot protect {}", path.display()))?;
            (self.log)(Action::Protected(path.to_path_buf()));
            if kind == Kind::Identity {
                self.identity_exposed(path);
            }
        }
        if !entry.is_dir() {
            return Ok(());
        }
        drop(entry);
        let listing =
            std::fs::read_dir(path).with_context(|| format!("cannot list {}", path.display()))?;
        for item in listing {
            let child = item
                .with_context(|| format!("cannot list {}", path.display()))?
                .path();
            self.walk(&child, kind, false)?;
        }
        Ok(())
    }

    fn untrusted_identity(&mut self, path: &Path, is_dir: bool, in_changeable: bool) {
        match self.untrusted {
            UntrustedIdentity::Keep => {
                // The directory that holds it is already named.
                if !in_changeable {
                    self.changeable.push(path.to_path_buf());
                }
            }
            UntrustedIdentity::Remove => self.doomed.push((path.to_path_buf(), is_dir)),
        }
    }

    fn identity_exposed(&mut self, path: &Path) {
        if !self.exposure_told {
            self.exposure_told = true;
            (self.log)(Action::IdentityExposed(path.to_path_buf()));
        }
    }
}

/// Refuses a link, a second name of a file and a foreign owner.
fn check_entry(entry: &Entry, path: &Path, kind: Kind) -> Result<()> {
    let remedy = if kind == Kind::Identity {
        format!(
            "an administrator checks it and removes it; the removal loses the agent identity, so then {REENROLL_ADVICE}"
        )
    } else {
        "an administrator checks it and removes it (the file, not the directory), then runs the command again".to_owned()
    };
    if entry.is_reparse_point() {
        bail!(
            "{} is a reparse point, so the agent does not use it; {remedy}",
            path.display()
        );
    }
    if !entry.is_dir() && entry.links() > 1 {
        bail!(
            "{} is a hard link with {} names, so another path can carry its content and the agent does not use it; {remedy}",
            path.display(),
            entry.links()
        );
    }
    if entry.exposure(kind.rights())? == Exposure::ForeignOwner {
        bail!(
            "{} is owned by an account that is not SYSTEM or Administrators, so the agent does not use it; {remedy}",
            path.display()
        );
    }
    Ok(())
}

fn remove_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("cannot remove {}", path.display())),
    }
}

/// The names that the agent keeps next to the configuration directory, below `root`.
fn root_entries(config: &Path, root: &Path) -> Vec<(String, Kind)> {
    let mut names = Vec::new();
    let mut add = |path: &Path, kind: Kind, staged_by_replace: bool| {
        if path.parent() != Some(root) {
            return;
        }
        names.extend(name_of(path).map(|name| (name, kind)));
        // An atomic replacement of a file stages it under this name.
        if staged_by_replace {
            names.extend(name_of(&path.with_extension("tmp")).map(|name| (name, Kind::Agent)));
        }
    };
    add(config, Kind::Config, false);
    add(&crate::identity::lock_file(config), Kind::Lock, false);
    add(
        &crate::identity::sibling(config, ".staging"),
        Kind::Identity,
        false,
    );
    add(
        &crate::identity::sibling(config, ".old"),
        Kind::Identity,
        false,
    );
    add(
        &crate::restore_roots::roots_file_for(config),
        Kind::RestoreRoots,
        true,
    );
    add(
        &crate::scripts::mode_file_for(config),
        Kind::TrustMode,
        true,
    );
    add(
        &crate::scripts::allow_list_file_for(config),
        Kind::Agent,
        true,
    );
    add(&crate::logging::service_log_dir(config), Kind::Agent, false);
    names.push((OLD_RESTORE_ROOT.to_owned(), Kind::OldRestoreRoot));
    names
}

fn config_entries() -> Vec<(String, Kind)> {
    let identity = crate::identity::IDENTITY_FILES;
    let other = [
        crate::keys::KEY_UPDATES_FILE,
        crate::connection::STATUS_FILE,
        crate::journal::JOURNAL_FILE,
        crate::restic_store::RECORD_FILE,
    ];
    let mut names = Vec::new();
    for (file, kind) in identity
        .iter()
        .map(|file| (file, Kind::Identity))
        .chain(other.iter().map(|file| (file, Kind::Agent)))
    {
        names.push(((*file).to_owned(), kind));
        names.push((
            format!("{file}{}", crate::identity::STAGED_SUFFIX),
            Kind::Staged,
        ));
        let temporary = if kind == Kind::Identity {
            Kind::Temporary
        } else {
            kind
        };
        names.extend(name_of(&Path::new(file).with_extension("tmp")).map(|name| (name, temporary)));
    }
    names.push((crate::identity::ROTATION_MARKER.to_owned(), Kind::Staged));
    names.push((crate::log_spool::SPOOL_DIR.to_owned(), Kind::Agent));
    names.push((crate::preflight::PROBE_FILE.to_owned(), Kind::Agent));
    // Windows keeps the cache and, in older builds, restic below the configuration directory.
    names.extend(
        crate::paths::cache_dir()
            .strip_prefix(crate::paths::config_dir())
            .ok()
            .and_then(|relative| relative.to_str())
            .map(|name| (name.to_owned(), Kind::Agent)),
    );
    names.push(("bin".to_owned(), Kind::Agent));
    names
}

fn name_of(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::winsec::tests::{current_user_sid, grant, set_owner, temp};
    use crate::winsec::{check_admin_only, create_protected_file, create_protected_tree};

    fn tree(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = temp(name);
        std::fs::create_dir_all(&base).expect("cannot create the base");
        let root = base.join("ResticHub");
        let config = root.join("Agent");
        create_protected_tree(&config, &root).expect("cannot create the tree");
        (base, root, config)
    }

    fn repair(config: &Path, root: &Path) -> (Result<Vec<PathBuf>>, Vec<Action>) {
        repair_with(config, root, UntrustedIdentity::Keep)
    }

    fn repair_with(
        config: &Path,
        root: &Path,
        untrusted: UntrustedIdentity,
    ) -> (Result<Vec<PathBuf>>, Vec<Action>) {
        let mut actions = Vec::new();
        let result = repair_agent_tree(config, root, untrusted, &mut |action| actions.push(action));
        (result, actions)
    }

    fn is_open(path: &Path) -> bool {
        Entry::open(path)
            .expect("cannot open")
            .expect("the path must exist")
            .exposure(Rights::ReadOrChange)
            .expect("cannot read")
            != Exposure::Clean
    }

    fn junction(link: &Path, target: &Path) {
        let status = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .status()
            .expect("cannot run mklink");
        assert!(status.success(), "cannot create the junction");
    }

    #[test]
    fn fr_ag_04_an_open_top_level_is_repaired_and_its_writable_settings_are_not_trusted() {
        let (base, root, config) = tree("repair-top");
        let roots = root.join("Agent-restore-roots.txt");
        create_protected_file(&roots, b"C:\\restore").expect("cannot write");
        let mode = root.join("Agent-server-scripts");
        create_protected_file(&mode, b"server_scripts = enabled\n").expect("cannot write");
        let restore = root.join("Restore");
        std::fs::create_dir(&restore).expect("cannot create the old restore root");
        let restored = restore.join("restored.txt");
        std::fs::write(&restored, b"data").expect("cannot write");
        grant(&root, "(OI)(CI)(WD)").expect("cannot add the entry");
        grant(&roots, "(R,W)").expect("cannot add the entry");
        grant(&mode, "(R,W)").expect("cannot add the entry");
        grant(&restored, "(R,W)").expect("cannot add the entry");

        let (result, actions) = repair(&config, &root);
        result.expect("the repair must succeed");

        check_admin_only(&root).expect("the top level must be protected");
        assert!(!mode.exists(), "the trust mode must fall back to off");
        assert!(!roots.exists(), "the restore roots file must be set aside");
        let [
            Action::Protected(protected),
            Action::MovedRestoreRoots { from, to },
            removed @ Action::RemovedTrustMode(removed_path),
        ] = actions.as_slice()
        else {
            panic!("unexpected actions: {actions:?}");
        };
        assert_eq!((protected, from, removed_path), (&root, &roots, &mode));
        assert!(
            to.to_string_lossy()
                .contains("Agent-restore-roots.txt.untrusted-"),
            "{}",
            to.display()
        );
        assert_eq!(
            std::fs::read(to).expect("the set-aside file must stay"),
            b"C:\\restore"
        );
        assert!(
            removed
                .to_string()
                .contains("trust-server-scripts --enable"),
            "{removed}"
        );
        assert!(
            is_open(&restored),
            "the old restore tree must stay as it was"
        );
        create_protected_tree(&config, &root).expect("the normal check must pass now");
        let _ = std::fs::remove_dir_all(&base);
    }

    fn open_cache(config: &Path, files: usize) -> PathBuf {
        let data = config.join("cache").join("0123").join("data");
        std::fs::create_dir_all(&data).expect("cannot create the cache");
        for index in 0..files {
            std::fs::write(data.join(format!("blob-{index}")), b"x").expect("cannot write");
        }
        grant(&config.join("cache"), "(OI)(CI)(M)").expect("cannot add the entry");
        data
    }

    fn report_lines(config: &Path, root: &Path, verbose: bool) -> Vec<String> {
        let mut lines = Vec::new();
        let mut print = |line: &str| lines.push(line.to_owned());
        let mut report = Report::new(root, verbose, &mut print);
        let result = repair_agent_tree(config, root, UntrustedIdentity::Keep, &mut |action| {
            report.action(action);
        });
        report.finish();
        result.expect("the repair must succeed");
        lines
    }

    #[test]
    fn fr_ag_04_the_repair_prints_one_line_for_each_top_level_directory() {
        let (base, root, config) = tree("repair-summary");
        let data = open_cache(&config, 40);
        let mode = root.join("Agent-server-scripts");
        create_protected_file(&mode, b"server_scripts = enabled\n").expect("cannot write");
        grant(&mode, "(R,W)").expect("cannot add the entry");

        let lines = report_lines(&config, &root, false);

        // The cache, its two levels and the 40 files, under one top-level directory.
        let summary = format!(
            "repaired the access control of 43 entries under {}",
            config.display()
        );
        assert!(lines.contains(&summary), "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|line| line.contains("trust-server-scripts")),
            "a removal keeps its own line: {lines:?}"
        );
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(!is_open(&data.join("blob-0")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_verbose_prints_each_repaired_entry() {
        let (base, root, config) = tree("repair-verbose");
        let data = open_cache(&config, 3);
        let lines = report_lines(&config, &root, true);
        assert_eq!(lines.len(), 6, "{lines:?}");
        assert!(
            lines.contains(&Action::Protected(data.join("blob-2")).to_string()),
            "{lines:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_refusal_in_the_cache_keeps_the_identity_for_register_force() {
        let (base, root, config) = tree("repair-force-order");
        crate::identity::tests::sample()
            .save(&config)
            .expect("cannot save the identity");
        let target = base.join("target");
        std::fs::create_dir(&target).expect("cannot create the target");
        let cache = config.join("cache");
        std::fs::create_dir(&cache).expect("cannot create the cache");
        junction(&cache.join("link"), &target);
        grant(&config, "(OI)(CI)(WD)").expect("cannot add the entry");

        // The identity files come before `cache` in the listing.
        let (result, actions) = repair_with(&config, &root, UntrustedIdentity::Remove);
        let error = result.expect_err("a junction must stop register --force");

        assert!(format!("{error:#}").contains("reparse point"), "{error:#}");
        for file in crate::identity::IDENTITY_FILES {
            assert!(
                config.join(file).exists(),
                "{file} must stay after a refusal"
            );
        }
        assert!(
            !actions
                .iter()
                .any(|action| matches!(action, Action::RemovedIdentity(_))),
            "{actions:?}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_level_of_a_foreign_owner_stops_the_repair() {
        let (base, root, config) = tree("repair-owner-root");
        crate::identity::tests::sample()
            .save(&config)
            .expect("cannot save the identity");
        set_owner(
            &root,
            &current_user_sid().expect("cannot read the user SID"),
        )
        .expect("cannot set the owner");
        grant(&config, "(OI)(CI)(WD)").expect("cannot add the entry");
        let (result, actions) = repair_with(&config, &root, UntrustedIdentity::Remove);
        let error = format!("{:#}", result.expect_err("a foreign owner must stop"));
        assert!(error.contains("is owned by an account"), "{error}");
        assert!(actions.is_empty(), "nothing below it changes: {actions:?}");
        assert!(is_open(&config));
        assert!(config.join(crate::identity::KEY_FILE).exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_the_service_refuses_an_identity_that_another_account_could_change() {
        let (base, root, config) = tree("check-identity");
        crate::identity::tests::sample()
            .save(&config)
            .expect("cannot save the identity");
        check_identity(&config, &root).expect("a protected identity passes");

        let key = config.join(crate::identity::KEY_FILE);
        grant(&key, "(R,W)").expect("cannot add the entry");
        let error = format!(
            "{:#}",
            check_identity(&config, &root).expect_err("an open key")
        );
        assert!(error.contains("agent-key.pem"), "{error}");
        Entry::open(&key)
            .expect("cannot open")
            .expect("the key must exist")
            .protect()
            .expect("cannot protect");
        check_identity(&config, &root).expect("a protected key passes again");

        grant(&root, "(OI)(CI)(WD)").expect("cannot add the entry");
        let error = format!(
            "{:#}",
            check_identity(&config, &root).expect_err("an open level")
        );
        assert!(error.contains("grants a write access"), "{error}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_an_empty_identity_directory_does_not_hide_a_planted_backup() {
        let (base, root, config) = tree("check-backup");
        let backup = crate::identity::sibling(&config, ".old");
        crate::identity::tests::sample()
            .save(&backup)
            .expect("cannot save the identity");
        check_identity(&config, &root).expect("a protected backup passes");

        grant(&backup.join(crate::identity::CA_CHAIN_FILE), "(R,W)").expect("cannot add");
        let error = format!(
            "{:#}",
            check_identity(&config, &root).expect_err("an open file in the backup")
        );
        assert!(error.contains("Agent.old"), "{error}");
        Entry::open(&backup.join(crate::identity::CA_CHAIN_FILE))
            .expect("cannot open")
            .expect("the chain must exist")
            .protect()
            .expect("cannot protect");

        // The attack: the identity directory is empty and the root is open.
        grant(&root, "(OI)(CI)(WD)").expect("cannot add the entry");
        let error = format!(
            "{:#}",
            check_identity(&config, &root).expect_err("an open root")
        );
        assert!(error.contains("grants a write access"), "{error}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_protected_tree_is_not_changed() {
        let (base, root, config) = tree("repair-clean");
        create_protected_file(&config.join("agent-key.pem"), b"key").expect("cannot write");
        create_protected_file(
            &root.join("Agent-server-scripts"),
            b"server_scripts = enabled\n",
        )
        .expect("cannot write");
        let (result, actions) = repair(&config, &root);
        result.expect("the repair must succeed");
        assert!(actions.is_empty(), "{actions:?}");
        assert!(root.join("Agent-server-scripts").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_writable_identity_directory_stays_as_found_until_a_new_registration_replaces_it()
    {
        let (base, root, config) = tree("repair-config");
        crate::identity::tests::sample()
            .save(&config)
            .expect("cannot save the identity");
        let key = config.join(crate::identity::KEY_FILE);
        let journal = config.join(crate::journal::JOURNAL_FILE);
        create_protected_file(&journal, b"[]").expect("cannot write");
        grant(&config, "(OI)(CI)(WD)").expect("cannot add the entry");
        grant(&key, "(R,W)").expect("cannot add the entry");
        grant(&journal, "(R,W)").expect("cannot add the entry");

        for run in ["first", "second"] {
            let (result, actions) = repair(&config, &root);
            let changeable = result.expect("the repair must succeed");
            assert_eq!(changeable, vec![config.clone()], "{run} run");
            assert!(is_open(&config), "{run} run: the directory must stay open");
            assert!(is_open(&key), "{run} run: the key must stay open");
            assert!(!is_open(&journal), "{run} run: other files are repaired");
            assert!(
                actions
                    .iter()
                    .all(|action| *action == Action::Protected(journal.clone())),
                "{run} run: {actions:?}"
            );
        }

        let (result, actions) = repair_with(&config, &root, UntrustedIdentity::Remove);
        assert!(result.expect("register --force must repair").is_empty());
        check_admin_only(&config).expect("the directory must be protected");
        assert_eq!(actions[0], Action::Protected(config.clone()));
        for file in [
            crate::identity::KEY_FILE,
            crate::identity::CERT_FILE,
            crate::identity::CA_CHAIN_FILE,
            crate::identity::METADATA_FILE,
            crate::identity::SIGNING_KEYS_FILE,
        ] {
            let path = config.join(file);
            assert!(!path.exists(), "{file} must be removed");
            assert!(
                actions.contains(&Action::RemovedIdentity(path)),
                "{file}: {actions:?}"
            );
        }
        assert!(!crate::identity::StoredIdentity::exists(&config));
        assert!(journal.exists(), "only the identity goes");

        crate::identity::tests::sample()
            .save(&config)
            .expect("register --force must write the identity");
        let (result, actions) = repair(&config, &root);
        assert!(result.expect("the repair must succeed").is_empty());
        assert!(actions.is_empty(), "{actions:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_writable_temporary_identity_file_is_removed_without_a_refusal() {
        let (base, root, config) = tree("repair-temporary");
        crate::identity::tests::sample()
            .save(&config)
            .expect("cannot save the identity");
        let temporary = config.join("ca-chain.tmp");
        create_protected_file(&temporary, b"planted").expect("cannot write");
        grant(&temporary, "(R,W)").expect("cannot add the entry");
        let clean = config.join("identity.tmp");
        create_protected_file(&clean, b"{}").expect("cannot write");

        let (result, actions) = repair(&config, &root);
        assert!(result.expect("the repair must succeed").is_empty());
        assert!(!temporary.exists());
        assert!(
            clean.exists(),
            "a clean leftover of a protected directory stays"
        );
        assert_eq!(actions, vec![Action::RemovedTemporary(temporary)]);
        crate::identity::StoredIdentity::load(&config).expect("the identity stays usable");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_key_that_users_could_read_is_protected_with_a_warning() {
        let (base, root, config) = tree("repair-read");
        let key = config.join("agent-key.pem");
        create_protected_file(&key, b"key").expect("cannot write");
        grant(&key, "(R)").expect("cannot add the entry");
        let (result, actions) = repair(&config, &root);
        let changeable = result.expect("the repair must succeed");
        assert!(changeable.is_empty(), "a read keeps the identity in use");
        assert!(!is_open(&key), "the read must be gone");
        assert_eq!(
            actions,
            vec![Action::Protected(key.clone()), Action::IdentityExposed(key),]
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_ca_chain_that_users_could_change_is_reported_and_a_new_registration_replaces_it()
    {
        let (base, root, config) = tree("repair-ca-chain");
        crate::identity::tests::sample()
            .save(&config)
            .expect("cannot save the identity");
        let chain = config.join(crate::identity::CA_CHAIN_FILE);
        grant(&chain, "(R,W)").expect("cannot add the entry");

        for run in ["first", "second"] {
            let (result, actions) = repair(&config, &root);
            let changeable = result.expect("the repair must succeed");
            assert_eq!(changeable, vec![chain.clone()], "{run} run");
            assert!(is_open(&chain), "{run} run: the chain must stay as found");
            assert!(actions.is_empty(), "{run} run: {actions:?}");
        }

        let (result, actions) = repair_with(&config, &root, UntrustedIdentity::Remove);
        assert!(result.expect("register --force must repair").is_empty());
        assert!(!chain.exists());
        assert_eq!(actions, vec![Action::RemovedIdentity(chain)]);
        crate::identity::tests::sample()
            .save(&config)
            .expect("register --force must replace the identity");
        let (result, actions) = repair(&config, &root);
        assert!(result.expect("the repair must succeed").is_empty());
        assert!(actions.is_empty(), "{actions:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_rotation_in_an_open_identity_directory_is_removed_and_the_key_stays() {
        let (base, root, config) = tree("repair-rotation");
        let key = config.join("agent-key.pem");
        create_protected_file(&key, b"key").expect("cannot write");
        let marker = config.join("rotation.next");
        let staged = config.join("agent-key.pem.next");
        create_protected_file(&marker, b"").expect("cannot write");
        create_protected_file(&staged, b"planted").expect("cannot write");
        grant(&config, "(OI)(CI)(WD)").expect("cannot add the entry");

        let (result, actions) = repair(&config, &root);
        result.expect("the repair must succeed");

        assert!(!marker.exists() && !staged.exists());
        assert!(
            actions.contains(&Action::RemovedStaged(marker)),
            "{actions:?}"
        );
        assert!(
            actions.contains(&Action::RemovedStaged(staged)),
            "{actions:?}"
        );
        let _ = crate::identity::StoredIdentity::load(&config);
        assert_eq!(std::fs::read(&key).expect("the key must stay"), b"key");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_writable_staged_file_in_a_protected_directory_is_removed() {
        let (base, root, config) = tree("repair-staged");
        let marker = config.join("rotation.next");
        let staged = config.join("agent-cert.pem.next");
        create_protected_file(&marker, b"").expect("cannot write");
        create_protected_file(&staged, b"cert").expect("cannot write");
        grant(&staged, "(R,W)").expect("cannot add the entry");
        let (result, actions) = repair(&config, &root);
        result.expect("the repair must succeed");
        assert!(
            marker.exists(),
            "a clean staged file of a protected directory stays"
        );
        assert!(!staged.exists());
        assert_eq!(actions, vec![Action::RemovedStaged(staged)]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_an_unknown_entry_is_refused_after_the_level_is_protected() {
        let (base, root, config) = tree("repair-unknown");
        let planted = root.join("planted.exe");
        std::fs::write(&planted, b"x").expect("cannot write");
        let mode = root.join("Agent-server-scripts");
        create_protected_file(&mode, b"server_scripts = enabled\n").expect("cannot write");
        grant(&mode, "(R,W)").expect("cannot add the entry");
        grant(&root, "(OI)(CI)(WD)").expect("cannot add the entry");
        let (result, actions) = repair(&config, &root);
        let error = format!(
            "{:#}",
            result.expect_err("an unknown entry must stop the repair")
        );
        assert!(error.contains("planted.exe"), "{error}");
        assert!(error.contains("the file, not the directory"), "{error}");
        check_admin_only(&root).expect("the level stays protected after a refusal");
        assert!(planted.exists());
        assert!(
            !mode.exists(),
            "a refusal must not leave a writable mode file"
        );
        assert_eq!(
            actions,
            vec![
                Action::Protected(root.clone()),
                Action::RemovedTrustMode(mode)
            ]
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_known_entry_of_a_foreign_owner_is_refused_and_named() {
        let (base, root, config) = tree("repair-owner-file");
        let roots = root.join("Agent-restore-roots.txt");
        create_protected_file(&roots, b"C:\\restore").expect("cannot write");
        set_owner(
            &roots,
            &current_user_sid().expect("cannot read the user SID"),
        )
        .expect("cannot set the owner");
        grant(&root, "(OI)(CI)(WD)").expect("cannot add the entry");
        let (result, _) = repair(&config, &root);
        let error = format!(
            "{:#}",
            result.expect_err("a foreign owner must stop the repair")
        );
        assert!(error.contains("Agent-restore-roots.txt"), "{error}");
        assert!(error.contains("the file, not the directory"), "{error}");
        assert!(roots.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_an_identity_file_of_a_foreign_owner_names_the_new_registration() {
        let (base, root, config) = tree("repair-owner-key");
        let key = config.join("agent-key.pem");
        create_protected_file(&key, b"key").expect("cannot write");
        set_owner(&key, &current_user_sid().expect("cannot read the user SID"))
            .expect("cannot set the owner");
        let (result, _) = repair(&config, &root);
        let error = format!(
            "{:#}",
            result.expect_err("a foreign owner must stop the repair")
        );
        assert!(error.contains("agent-key.pem"), "{error}");
        assert!(error.contains("register --force"), "{error}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_hard_link_with_a_known_name_is_refused() {
        let (base, root, config) = tree("repair-link");
        let other = base.join("other.ini");
        create_protected_file(&other, b"server_scripts = enabled\n").expect("cannot write");
        let mode = root.join("Agent-server-scripts");
        std::fs::hard_link(&other, &mode).expect("cannot create the hard link");
        let (result, actions) = repair(&config, &root);
        let error = format!("{:#}", result.expect_err("a hard link must be refused"));
        assert!(error.contains("Agent-server-scripts"), "{error}");
        assert!(error.contains("hard link"), "{error}");
        assert!(actions.is_empty(), "{actions:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_junction_in_the_cache_is_refused_and_its_target_stays() {
        let (base, root, config) = tree("repair-junction");
        let target = base.join("target");
        std::fs::create_dir(&target).expect("cannot create the target");
        grant(&target, "(OI)(CI)(M)").expect("cannot add the entry");
        let cache = config.join("cache");
        std::fs::create_dir(&cache).expect("cannot create the cache");
        junction(&cache.join("link"), &target);
        grant(&config, "(OI)(CI)(WD)").expect("cannot add the entry");
        let (result, _) = repair(&config, &root);
        let error = format!("{:#}", result.expect_err("a junction must be refused"));
        assert!(error.contains("reparse point"), "{error}");
        assert!(is_open(&target), "the repair must not change the target");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_an_explicit_entry_on_a_nested_cache_file_is_removed() {
        let (base, root, config) = tree("repair-nested");
        let cache = config.join("cache");
        create_protected_tree(&cache.join("snapshots"), &root).expect("cannot create the cache");
        let file = cache.join("snapshots").join("data");
        create_protected_file(&file, b"x").expect("cannot write");
        grant(&file, "(R,W)").expect("cannot add the entry");
        let (result, actions) = repair(&config, &root);
        result.expect("the repair must succeed");
        assert!(!is_open(&file), "the explicit entry must be gone");
        assert_eq!(actions, vec![Action::Protected(file)]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_an_open_top_level_with_the_held_identity_lock_is_repaired() {
        let (base, root, config) = tree("repair-lock");
        crate::identity::tests::sample()
            .save(&config)
            .expect("cannot save the identity");
        let lock = crate::identity::lock_file(&config);
        assert_eq!(lock, root.join("Agent.lock"));
        grant(&lock, "(R,W)").expect("cannot add the entry");
        grant(&root, "(OI)(CI)(WD)").expect("cannot add the entry");
        let held = std::fs::File::open(&lock).expect("cannot open the lock");
        held.lock().expect("cannot take the lock");

        let (result, actions) = repair(&config, &root);
        assert!(result.expect("the repair must succeed").is_empty());
        check_admin_only(&root).expect("the top level must be protected");
        assert!(
            actions.contains(&Action::RemovedLock(lock.clone())),
            "{actions:?}"
        );
        crate::identity::StoredIdentity::load(&config)
            .expect("the identity loads while the old lock stays held");
        drop(held);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_an_identity_lock_that_another_process_keeps_open_is_named() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let (base, root, config) = tree("repair-lock-open");
        crate::identity::tests::sample()
            .save(&config)
            .expect("cannot save the identity");
        let lock = crate::identity::lock_file(&config);
        grant(&root, "(OI)(CI)(WD)").expect("cannot add the entry");
        // FILE_SHARE_READ | FILE_SHARE_WRITE, without FILE_SHARE_DELETE.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2)
            .open(&lock)
            .expect("cannot open the lock");

        let (result, _) = repair(&config, &root);
        let error = format!(
            "{:#}",
            result.expect_err("a lock that cannot be removed must stop the repair")
        );
        assert!(error.contains("Agent.lock"), "{error}");
        assert!(error.contains("restarts the computer"), "{error}");
        drop(held);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_the_service_log_is_a_known_entry_and_its_files_are_protected() {
        let (base, root, config) = tree("repair-logs");
        let logs = crate::logging::service_log_dir(&config);
        assert_eq!(logs, root.join("Agent-logs"));
        create_protected_tree(&logs, &root).expect("cannot create the logs directory");
        let file = logs.join("agent-000001.log");
        create_protected_file(&file, b"line\n").expect("cannot write");
        grant(&file, "(R,W)").expect("cannot add the entry");
        grant(&logs, "(OI)(CI)(WD)").expect("cannot add the entry");
        let (result, actions) = repair_with(&config, &root, UntrustedIdentity::Keep);
        result.expect("the repair must succeed");
        check_admin_only(&logs).expect("the directory must be protected");
        assert!(!is_open(&file), "the entry on the log file must be gone");
        assert!(actions.contains(&Action::Protected(logs)), "{actions:?}");
        assert!(actions.contains(&Action::Protected(file)), "{actions:?}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_service_log_directory_that_is_a_junction_is_refused() {
        let (base, root, config) = tree("repair-logs-junction");
        let target = base.join("elsewhere");
        std::fs::create_dir(&target).expect("cannot create the target");
        junction(&crate::logging::service_log_dir(&config), &target);
        let (result, _) = repair(&config, &root);
        let error = format!("{:#}", result.expect_err("a junction must be refused"));
        assert!(error.contains("reparse point"), "{error}");
        assert!(
            create_protected_tree(&crate::logging::service_log_dir(&config), &root).is_err(),
            "the service must refuse the junction too"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_level_of_a_foreign_owner_is_not_repaired_and_the_advice_is_not_to_delete() {
        let (base, root, config) = tree("repair-owner-dir");
        set_owner(
            &root,
            &current_user_sid().expect("cannot read the user SID"),
        )
        .expect("cannot set the owner");
        let (result, actions) = repair(&config, &root);
        result.expect("the repair leaves the refusal to the normal check");
        assert!(actions.is_empty());
        let error = format!(
            "{:#}",
            create_protected_tree(&config, &root).expect_err("a foreign owner must be refused")
        );
        assert!(error.contains("/setowner *S-1-5-32-544"), "{error}");
        assert!(
            error.contains("O:BAG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"),
            "{error}"
        );
        assert!(error.contains("Set-Acl"), "{error}");
        assert!(!error.contains("deletes it"), "{error}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
