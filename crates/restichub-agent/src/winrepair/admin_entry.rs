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
//! a refusal, because a non-elevated process of the account could have changed its content.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::winsec::{Account, Entry, Exposure, Foreign, Rights, Tree, local_administrator};

/// Removes the explicit entry of a local administrator from the directories of the agent, and
/// returns `true` when it changed a level of the program tree (ADR-0048).
///
/// A level that stays open is left for the checks of the caller, which refuse it with the exit code
/// 78 as before. A process without the right to change the access control, as a console run of a
/// non-elevated administrator, changes nothing.
pub(crate) fn heal(config: &Path) -> bool {
    let mut program = false;
    for (path, tree, rights) in levels(config) {
        match level(&path, tree, rights) {
            Ok(Some(accounts)) => {
                for account in &accounts {
                    tracing::warn!(
                        "removed the access of {} ({}) from {}: Windows Explorer adds this access when an administrator opens the folder without elevation; the agent checks the folder, the identity and restic again, and the service continues",
                        account.name,
                        account.sid,
                        path.display()
                    );
                }
                program = program || tree == Tree::Program;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!("cannot check the access of {}: {error:#}", path.display());
            }
        }
    }
    program
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
pub(crate) fn level(path: &Path, tree: Tree, rights: Rights) -> Result<Option<Vec<Account>>> {
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
    entry.protect(tree).with_context(|| {
        format!(
            "cannot remove the access of another account from {}",
            path.display()
        )
    })?;
    Ok(Some(accounts))
}

/// The directories that the service checks at its start, with the rights that its checks apply
/// (ADR-0048).
fn levels(config: &Path) -> Vec<(PathBuf, Tree, Rights)> {
    let program = crate::paths::program_dir();
    let root = program.parent().map(Path::to_path_buf).unwrap_or_default();
    vec![
        (root, Tree::Program, Rights::Change),
        (program, Tree::Program, Rights::Change),
        (crate::paths::agent_root(), Tree::Data, Rights::Change),
        // The identity directory, its backup and its files: a read counts too.
        (config.to_path_buf(), Tree::Data, Rights::ReadOrChange),
        (
            crate::identity::sibling(config, ".old"),
            Tree::Data,
            Rights::ReadOrChange,
        ),
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
