//! The access entry that Windows Explorer adds for a local administrator (ADR-0048, threat T20).
//!
//! A non-elevated administrator token holds the Administrators group as deny-only, so the
//! administrator has no list right on an agent folder. File Explorer asks for consent, then adds an
//! explicit full-control entry for the account of the administrator. The service stops with the
//! exit code 78 at its next start, often weeks later, although the entry existed for the whole run.
//!
//! The service removes that entry from the directories of the agent and continues. It removes no
//! other entry: another account, an inherited entry, an entry that the agent does not parse, and a
//! trustee that the computer cannot verify keep the refusal of the checks that follow. A file stays
//! a refusal, because a non-elevated process of the account could have changed its content. The
//! identity directory and its backup lose the entry too, with a warning that the identity was
//! exposed to the account. `install-service` and `register` without `--force` run the same heal of
//! these two levels before their repair (ADR-0049).

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use super::Action;
use crate::preflight::ChangeableIdentity;
use crate::winsec::{
    Account, Entry, Exposure, Foreign, Removal, Rights, Sid, Tree, local_administrator,
};

/// Removes the explicit entries of local administrators from the directories of the agent, and
/// returns `true` when it changed a level of the program tree (ADR-0048).
///
/// A level that stays open is left for the checks of the caller, which refuse it with the exit code
/// 78 as before. The checks and the repair that `install-service` runs for a level that another
/// account could write run first, while the entries are still in place, because the account could
/// write there while the entry existed. A refusal keeps the entries, so each later start refuses
/// again. The copies of a removed entry go from the children of the level. A process without the
/// right to change the access control, as a console run of a non-elevated administrator, changes
/// nothing.
///
/// # Errors
///
/// Returns an error when a level holds something that the checks refuse, or an identity that
/// another account could change.
pub(crate) fn heal(config: &Path) -> Result<bool> {
    heal_levels(
        config,
        &crate::paths::agent_root(),
        &crate::paths::program_dir(),
        levels(config),
    )
}

fn heal_levels(
    config: &Path,
    data_root: &Path,
    program: &Path,
    levels: Vec<(PathBuf, Tree, Rights)>,
) -> Result<bool> {
    let mut opened = Vec::new();
    let mut warnings = Vec::new();
    for (path, tree, rights) in levels {
        match level(&path, rights) {
            Ok(Some(found)) => opened.push((path, tree, found)),
            Ok(None) => {}
            Err(error) => {
                warnings.push(format!(
                    "cannot check the access of {}: {error:#}",
                    path.display()
                ));
            }
        }
    }
    let healed = check_and_remove(config, data_root, program, &opened, &mut warnings);
    // The file sink failed while the entry was on the directory of its log, so it gets a new try
    // before the warnings (ADR-0048).
    crate::logging::retry_log_file();
    for warning in &warnings {
        tracing::warn!("{warning}");
    }
    healed
}

/// Removes the explicit entries of local administrators from the identity directory and its backup
/// only, as the service does at its start (ADR-0049).
///
/// `install-service` and `register` without `--force` run it before their repair, which keeps an
/// identity directory that another account could change as found.
///
/// # Errors
///
/// Returns the error of [`heal`]. An identity that another account could change gives
/// [`ChangeableIdentity`].
pub(crate) fn heal_identity(config: &Path, data_root: &Path) -> Result<()> {
    heal_levels(
        config,
        data_root,
        &crate::paths::program_dir(),
        identity_levels(config),
    )?;
    Ok(())
}

fn check_and_remove(
    config: &Path,
    data_root: &Path,
    program: &Path,
    opened: &[(PathBuf, Tree, Opened)],
    warnings: &mut Vec<String>,
) -> Result<bool> {
    if opened.is_empty() {
        return Ok(false);
    }
    let mut actions = Vec::new();
    let mut log = |action: Action| {
        if !matches!(action, Action::Protected(_)) {
            actions.push(action.to_string());
        }
    };
    let healed = check_then_remove(config, data_root, program, opened, &mut log, warnings);
    warnings.append(&mut actions);
    healed
}

fn check_then_remove(
    config: &Path,
    data_root: &Path,
    program: &Path,
    opened: &[(PathBuf, Tree, Opened)],
    log: &mut dyn FnMut(Action),
    warnings: &mut Vec<String>,
) -> Result<bool> {
    let trustees: Vec<Sid> = opened
        .iter()
        .flat_map(|(_, _, found)| found.trustees.iter().cloned())
        .collect();
    let data: Vec<PathBuf> = opened
        .iter()
        .filter(|(_, tree, _)| *tree == Tree::Data)
        .map(|(path, _, _)| path.clone())
        .collect();
    let healed_program = opened.iter().any(|(_, tree, _)| *tree == Tree::Program);
    if healed_program {
        let root = program.parent().unwrap_or(program);
        super::program::check_program_tree_without(program, root, &trustees)?;
    }
    if !data.is_empty() {
        refuse_changeable(&super::repair_before_heal(
            config, data_root, &data, &trustees, log,
        )?)?;
    }
    let identity = [
        config.to_path_buf(),
        crate::identity::sibling(config, ".old"),
    ];
    for (path, tree, found) in opened {
        found.remove(path)?;
        for account in &found.accounts {
            let warning = if identity.contains(path) {
                format!(
                    "removed the access of {} ({}) from {}: Windows Explorer adds this access when an administrator opens the folder without elevation; the agent identity was exposed to this account; the agent checks the identity again, and the service continues; if you do not trust this account, {}",
                    account.name,
                    account.sid,
                    path.display(),
                    super::REENROLL_ADVICE
                )
            } else {
                format!(
                    "removed the access of {} ({}) from {}: Windows Explorer adds this access when an administrator opens the folder without elevation; the agent checks the folder, the identity and restic again, and the service continues",
                    account.name,
                    account.sid,
                    path.display()
                )
            };
            warnings.push(warning);
        }
        if *tree == Tree::Program {
            // Windows gave each child that is not protected a copy of the entry.
            restamp_children(path, &found.trustees)?;
        }
    }
    // The account could add an entry between the first listing and the removal.
    if !data.is_empty() {
        refuse_changeable(&super::repair_after_heal(config, data_root, &data, log)?)?;
    }
    Ok(healed_program)
}

fn refuse_changeable(changeable: &[PathBuf]) -> Result<()> {
    if changeable.is_empty() {
        return Ok(());
    }
    Err(ChangeableIdentity {
        paths: changeable.to_vec(),
    }
    .into())
}

/// Finds the explicit entries of local administrators on one directory, and changes nothing.
///
/// Returns `None` when the directory holds no entry of another account, when an entry stays that
/// the checks must refuse, and when this process may not change the access control.
///
/// # Errors
///
/// Returns an error when the access control cannot be read, for a reason other than a missing
/// directory or a missing right.
pub(crate) fn level(path: &Path, rights: Rights) -> Result<Option<Opened>> {
    let entry = match Entry::open(path) {
        Ok(Some(entry)) => entry,
        Ok(None) => return Ok(None),
        Err(error) if is_denied(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    if !entry.is_dir() || entry.is_reparse_point() {
        return Ok(None);
    }
    match entry.exposure(rights)? {
        // Nothing to remove, and a directory that another account owns keeps its refusal.
        Exposure::Clean | Exposure::ForeignOwner => return Ok(None),
        Exposure::ForeignGrant => {}
    }
    let Foreign::Grants(grants) = entry.foreign(rights)? else {
        return Ok(None);
    };
    let mut accounts = Vec::new();
    for grant in &grants {
        // An inherited entry comes from a parent directory, so Explorer did not add it here.
        if grant.inherited {
            return Ok(None);
        }
        let Some(account) = local_administrator(grant.sid.as_ptr()) else {
            return Ok(None);
        };
        accounts.push(account);
    }
    if accounts.is_empty() {
        return Ok(None);
    }
    let trustees: Vec<Sid> = grants.iter().map(|grant| grant.sid.clone()).collect();
    Ok(Some(Opened {
        entry,
        rights,
        accounts,
        trustees,
    }))
}

/// One directory that holds the entry of a local administrator.
pub(crate) struct Opened {
    /// The handle of the check, so the removal changes the same directory.
    entry: Entry,
    rights: Rights,
    /// Who holds the entry.
    pub(crate) accounts: Vec<Account>,
    /// Whose entries the directory and its children lose.
    pub(crate) trustees: Vec<Sid>,
}

impl Opened {
    /// Removes the entries, and refuses a directory that another account opened again since the
    /// check.
    pub(crate) fn remove(&self, path: &Path) -> Result<()> {
        self.entry
            .remove_permits(&self.trustees, Removal::All)
            .with_context(|| {
                format!(
                    "cannot remove the access of another account from {}",
                    path.display()
                )
            })?;
        if self.entry.exposure(self.rights)? != Exposure::Clean {
            bail!(
                "{} grants another account a change after the removal of the access of an administrator",
                path.display()
            );
        }
        Ok(())
    }
}

/// Removes the copies that Windows gave to the children of a healed program level (ADR-0048).
///
/// The level is protected again, so a new child inherits no entry. The checks of the caller still
/// refuse a child that another account owns, a link and a name that the agent does not know.
fn restamp_children(dir: &Path, trustees: &[Sid]) -> Result<()> {
    let listing =
        std::fs::read_dir(dir).with_context(|| format!("cannot list {}", dir.display()))?;
    for item in listing {
        let path = item
            .with_context(|| format!("cannot list {}", dir.display()))?
            .path();
        let Some(entry) = Entry::inspect(&path)? else {
            continue;
        };
        if entry.is_reparse_point() {
            continue;
        }
        let is_dir = entry.is_dir();
        let copies = match entry.foreign(Rights::Change)? {
            Foreign::Grants(grants) => grants
                .iter()
                .any(|grant| grant.inherited && trustees.contains(&grant.sid)),
            Foreign::None | Foreign::Unknown => false,
        };
        drop(entry);
        if copies && let Some(entry) = Entry::open(&path)? {
            entry
                .remove_permits(trustees, Removal::InheritedCopies)
                .with_context(|| format!("cannot remove the copies on {}", path.display()))?;
        }
        if is_dir {
            restamp_children(&path, trustees)?;
        }
    }
    Ok(())
}

/// The directories that the service checks at its start, with the rights that its checks apply
/// (ADR-0048, ADR-0049).
fn levels(config: &Path) -> Vec<(PathBuf, Tree, Rights)> {
    let program = crate::paths::program_dir();
    let root = program.parent().map(Path::to_path_buf).unwrap_or_default();
    [
        (root, Tree::Program, Rights::Change),
        (program, Tree::Program, Rights::Change),
        (crate::paths::agent_root(), Tree::Data, Rights::Change),
    ]
    .into_iter()
    .chain(identity_levels(config))
    .chain([
        (
            crate::logging::service_log_dir(config),
            Tree::Data,
            Rights::Change,
        ),
        (config.join("cache"), Tree::Data, Rights::Change),
        (
            config.join(crate::log_spool::SPOOL_DIR),
            Tree::Data,
            Rights::Change,
        ),
    ])
    .collect()
}

fn identity_levels(config: &Path) -> Vec<(PathBuf, Tree, Rights)> {
    vec![
        (config.to_path_buf(), Tree::Data, Rights::Change),
        (
            crate::identity::sibling(config, ".old"),
            Tree::Data,
            Rights::Change,
        ),
    ]
}

/// Tells whether the process may not change the access control, as in a console run without
/// elevation.
fn is_denied(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
        .any(|io| io.kind() == std::io::ErrorKind::PermissionDenied)
}

#[cfg(test)]
mod tests;
