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
//! identity directory is not healed either: the repair of `install-service` leaves it open on
//! purpose, so a new registration replaces it.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::winsec::{
    Account, Entry, Exposure, Foreign, Removal, Rights, Sid, Tree, local_administrator,
};

/// Removes the explicit entries of local administrators from the directories of the agent, and
/// returns `true` when it changed a level of the program tree (ADR-0048).
///
/// A level that stays open is left for the checks of the caller, which refuse it with the exit code
/// 78 as before. The copies of a removed entry go from the children of the level. A healed data
/// level then gets the checks and the repair that `install-service` runs for it, because the account
/// could write there while the entry existed. A process without the right to change the access
/// control, as a console run of a non-elevated administrator, changes nothing.
///
/// # Errors
///
/// Returns an error when a healed level holds something that the repair refuses, or an identity
/// that another account could change.
pub(crate) fn heal(config: &Path) -> Result<bool> {
    let mut healed = Vec::new();
    let mut warnings = Vec::new();
    let mut program = false;
    for (path, tree, rights) in levels(config) {
        match level(&path, rights) {
            Ok(Some(removed)) => {
                for account in &removed.accounts {
                    warnings.push(format!(
                        "removed the access of {} ({}) from {}: Windows Explorer adds this access when an administrator opens the folder without elevation; the agent checks the folder, the identity and restic again, and the service continues",
                        account.name,
                        account.sid,
                        path.display()
                    ));
                }
                if tree == Tree::Program {
                    // Windows gave each child that is not protected a copy of the entry.
                    restamp_children(&path, &removed.trustees)?;
                    program = true;
                } else {
                    healed.push(path);
                }
            }
            Ok(None) => {}
            Err(error) => {
                warnings.push(format!(
                    "cannot check the access of {}: {error:#}",
                    path.display()
                ));
            }
        }
    }
    // The file sink failed while the entry was on the directory of its log, so it gets a new try
    // before the warnings (ADR-0048).
    crate::logging::retry_log_file();
    for warning in &warnings {
        tracing::warn!("{warning}");
    }
    if healed.is_empty() {
        return Ok(program);
    }
    let mut log = |action: crate::winrepair::Action| {
        if !matches!(action, crate::winrepair::Action::Protected(_)) {
            tracing::warn!("{action}");
        }
    };
    let changeable = crate::winrepair::repair_after_heal(
        config,
        &crate::paths::agent_root(),
        &healed,
        &mut log,
    )?;
    if !changeable.is_empty() {
        let files = changeable
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "the identity of this host could be changed by another account ({files}), so the agent does not use it; {}",
            crate::preflight::REGISTER_AGAIN
        );
    }
    Ok(program)
}

/// Removes the explicit entries of local administrators from one directory.
///
/// Returns `None` when the directory holds no entry of another account, when an entry stays that
/// the checks must refuse, and when this process may not change the access control.
///
/// # Errors
///
/// Returns an error when the access control cannot be read or set, for a reason other than a
/// missing directory or a missing right.
pub(crate) fn level(path: &Path, rights: Rights) -> Result<Option<Removed>> {
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
    entry
        .remove_permits(&trustees, Removal::All)
        .with_context(|| {
            format!(
                "cannot remove the access of another account from {}",
                path.display()
            )
        })?;
    if entry.exposure(rights)? != Exposure::Clean {
        return Ok(None);
    }
    Ok(Some(Removed { accounts, trustees }))
}

/// One directory that lost the entry of a local administrator.
pub(crate) struct Removed {
    /// Who held the entry that the directory lost.
    pub(crate) accounts: Vec<Account>,
    /// Whose entries the children lose as well.
    pub(crate) trustees: Vec<Sid>,
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
/// (ADR-0048). The identity directory and its backup stay out: `install-service` leaves an open
/// identity directory for a new registration.
fn levels(config: &Path) -> Vec<(PathBuf, Tree, Rights)> {
    let program = crate::paths::program_dir();
    let root = program.parent().map(Path::to_path_buf).unwrap_or_default();
    vec![
        (root, Tree::Program, Rights::Change),
        (program, Tree::Program, Rights::Change),
        (crate::paths::agent_root(), Tree::Data, Rights::Change),
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
