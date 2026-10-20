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
use crate::winsec::{Entry, Exposure, Rights, Tree};

mod admin_entry;
mod program;

pub(crate) use admin_entry::heal as heal_admin_entries;
pub(crate) use program::{check_program_tree, rebuild_open_program_tree};

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
    MovedRepositoryRoots {
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
    /// A program tree that another process keeps open, so it cannot move.
    RepairingInPlace {
        dir: PathBuf,
        cause: String,
        holders: String,
    },
    RemovedAgent(PathBuf),
    RemovedUnused(PathBuf),
    /// A program that runs cannot be removed, but it can be renamed.
    SetAside {
        from: PathBuf,
        to: PathBuf,
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
            Self::MovedRepositoryRoots { from, to } => {
                if cfg!(windows) {
                    write!(
                        f,
                        "moved {} to {}: another account could change it; the agent reads the local repository roots file on Linux only, so this host ignores it",
                        from.display(),
                        to.display()
                    )
                } else {
                    write!(
                        f,
                        "moved {} to {}: another account could change it, so the local repository roots are off; an administrator checks the file and moves it back to {} to turn them on again",
                        from.display(),
                        to.display(),
                        from.display()
                    )
                }
            }
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
            Self::RepairingInPlace {
                dir,
                cause,
                holders,
            } => write!(
                f,
                "cannot move {} ({cause}); open in: {holders}; so the agent repairs it in place: it gives each level and file the access of SYSTEM and Administrators only, and replaces the agent and restic",
                dir.display()
            ),
            Self::RemovedAgent(path) => write!(
                f,
                "removed {}: another account could change it, so the agent copies the running agent there again",
                path.display()
            ),
            Self::RemovedUnused(path) => write!(
                f,
                "removed {}: another account could change it, and the agent does not use it",
                path.display()
            ),
            Self::SetAside { from, to } => write!(
                f,
                "moved {} to {}: a process runs it, so it cannot be removed now; Windows removes it at the next restart",
                from.display(),
                to.display()
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
    /// The repository roots file of ADR-0045, which the agent reads on Linux only.
    RepositoryRoots,
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
    run(config, root, untrusted, &[], log)
}

/// Runs the same checks and repairs for the levels whose entry of a local administrator the service
/// removed at this start (ADR-0048).
///
/// The entry gave that account a change of the level, so a name that the agent does not know, a lock
/// or a leftover of an interrupted write is treated as it is for a level that the repair finds open.
/// The identity of another account stays as found, so the caller refuses it as before.
pub(crate) fn repair_after_heal(
    config: &Path,
    root: &Path,
    healed: &[PathBuf],
    log: &mut dyn FnMut(Action),
) -> Result<Vec<PathBuf>> {
    run(config, root, UntrustedIdentity::Keep, healed, log)
}

fn run(
    config: &Path,
    root: &Path,
    untrusted: UntrustedIdentity,
    healed: &[PathBuf],
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
        let opened = healed.iter().any(|healed| healed == level);
        repair.level(level, opened)?;
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
        refuse_exposure(&entry, level, Rights::Change)?;
    }
    let names = crate::identity::trusted_names();
    for dir in [config, backup.as_path()] {
        for (name, secret) in &names {
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
            // As on Linux, a private key that another account can read is refused.
            let rights = if *secret {
                Rights::ReadOrChange
            } else {
                Rights::Change
            };
            refuse_exposure(&entry, &path, rights)?;
        }
    }
    Ok(())
}

fn refuse_exposure(entry: &Entry, path: &Path, rights: Rights) -> Result<()> {
    match entry
        .exposure(rights)
        .with_context(|| format!("cannot check {}", path.display()))?
    {
        Exposure::Clean => Ok(()),
        Exposure::ForeignGrant if rights == Rights::ReadOrChange => bail!(
            "{} holds the private key and grants a read or write access to an account that is not SYSTEM or Administrators",
            path.display()
        ),
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
    /// `opened` says that the level lost an entry of a local administrator at this start, so its
    /// checks treat it as a level that another account could write (ADR-0048).
    fn level(&mut self, level: &Path, opened: bool) -> Result<()> {
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
        let exposure = entry
            .exposure(rights)
            .with_context(|| format!("cannot check {}", level.display()))?;
        if exposure == Exposure::ForeignOwner {
            // The owner can change the access at any time, also of a level below it through a link.
            bail!("{}", crate::winsec::foreign_owner_advice(level));
        }
        let open = opened || exposure == Exposure::ForeignGrant;
        let changeable =
            open && is_config && entry.exposure(Rights::Change)? == Exposure::ForeignGrant;
        if changeable && self.untrusted == UntrustedIdentity::Keep {
            // A repaired access would hide the grant, so a later install-service would trust
            // whatever that account wrote. Only a new registration replaces the directory.
            self.changeable.push(level.to_path_buf());
        } else if exposure == Exposure::ForeignGrant {
            // The DACL comes before the listing, so a new handle cannot add an entry after the
            // check. A handle opened before keeps its access until it closes (T20 residual).
            entry
                .protect(Tree::Data)
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
                Some(setting @ (Kind::TrustMode | Kind::RestoreRoots | Kind::RepositoryRoots)) => {
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
            return Ok(());
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stamp = time::OffsetDateTime::now_utc().unix_timestamp();
        let aside = path.with_file_name(format!("{name}.untrusted-{stamp}"));
        std::fs::rename(path, &aside)
            .with_context(|| format!("cannot move {} to {}", path.display(), aside.display()))?;
        // The file keeps the access of the level that it came from, so an administrator who checks
        // it and moves it back gets a file that the agent trusts again (ADR-0048).
        if let Some(entry) = Entry::open(&aside)? {
            entry
                .protect(Tree::Data)
                .with_context(|| format!("cannot protect {}", aside.display()))?;
        }
        let action = if kind == Kind::RepositoryRoots {
            Action::MovedRepositoryRoots {
                from: path.to_path_buf(),
                to: aside,
            }
        } else {
            Action::MovedRestoreRoots {
                from: path.to_path_buf(),
                to: aside,
            }
        };
        (self.log)(action);
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
                .protect(Tree::Data)
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
        &crate::repository_roots::roots_file_for(config),
        Kind::RepositoryRoots,
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
mod tests;
