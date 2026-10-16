//! The repair of agent directories that an older build left with a foreign access (threat T20).
//!
//! An older build could leave `%ProgramData%\ResticHub` with the access control of
//! `%ProgramData%`, which lets a normal user create files there. Deleting the directory would
//! delete the agent identity, so an elevated command repairs the access instead, but only when the
//! repair cannot hand a foreign file to the agent: SYSTEM or Administrators own the level, and
//! the level holds only entries that the agent itself creates, all owned by SYSTEM or
//! Administrators. The service never repairs.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

use crate::winsec::{Exposure, exposure, protect_with_children, protect_without_propagation};

/// The directory that an older build used as the default restore root. The repair checks the owner
/// of the entry and never reads inside it.
const OLD_RESTORE_ROOT: &str = "Restore";

/// Repairs the levels from `root` down to `config` that exist, that SYSTEM or Administrators own
/// and that grant a foreign account a right on the agent (FR-AG-04).
///
/// A level of another owner, a reparse point and a level above `root` are left to the normal
/// check, which refuses them. `log` gets each repaired path.
///
/// # Errors
///
/// Returns an error when a level to repair holds an entry that the agent does not know, a link, or
/// an entry that another account owns, or when the access control cannot be set.
pub fn repair_agent_tree(config: &Path, root: &Path, log: &mut dyn FnMut(&Path)) -> Result<()> {
    if !config.starts_with(root) {
        return Ok(());
    }
    let mut levels: Vec<&Path> = config
        .ancestors()
        .take_while(|level| level.starts_with(root))
        .collect();
    levels.reverse();
    for level in levels {
        repair_level(level, root, config, log)?;
    }
    Ok(())
}

fn repair_level(
    level: &Path,
    root: &Path,
    config: &Path,
    log: &mut dyn FnMut(&Path),
) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(level) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read {}", level.display()));
        }
    };
    if !metadata.is_dir() || is_reparse_point(&metadata) {
        return Ok(());
    }
    if exposure(level)? != Exposure::ForeignGrant {
        return Ok(());
    }
    let known = if level == config {
        config_entries(config)
    } else {
        root_entries(config, root)
    };
    let entries = check_entries(level, &known)?;
    protect_without_propagation(level, true)?;
    log(level);
    for entry in entries {
        // The config directory is a level of its own, and the old restore root is not the agent's.
        if entry == config || is_old_restore_root(&entry) {
            continue;
        }
        repair_entry(&entry, log)?;
    }
    Ok(())
}

/// Reads the entries of `level` and refuses an unknown one, a link, or an entry of a foreign owner.
fn check_entries(level: &Path, known: &[String]) -> Result<Vec<PathBuf>> {
    let mut entries = Vec::new();
    let listing =
        std::fs::read_dir(level).with_context(|| format!("cannot list {}", level.display()))?;
    for entry in listing {
        let entry = entry.with_context(|| format!("cannot list {}", level.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !known.iter().any(|known| known.eq_ignore_ascii_case(&name)) {
            bail!(
                "{} holds {}, which is not a file of the agent, so the agent does not repair the access control of {}; an administrator checks that file and removes it (the file, not the directory), then runs the command again",
                level.display(),
                path.display(),
                level.display()
            );
        }
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        if is_reparse_point(&metadata) {
            bail!(
                "{} is a reparse point; an administrator removes the link, then runs the command again",
                path.display()
            );
        }
        if exposure(&path)? == Exposure::ForeignOwner {
            bail!(
                "{} is owned by an account that is not SYSTEM or Administrators, so the agent does not repair the access control of {}; an administrator checks that file and removes it (the file, not the directory), then runs the command again",
                path.display(),
                level.display()
            );
        }
        entries.push(path);
    }
    Ok(entries)
}

/// A directory flows its access into the children, so its tree must hold no link and no foreign
/// owner before the repair.
fn repair_entry(path: &Path, log: &mut dyn FnMut(&Path)) -> Result<()> {
    if exposure(path)? != Exposure::ForeignGrant {
        return Ok(());
    }
    if path.is_dir() {
        check_tree(path)?;
        protect_with_children(path)?;
    } else {
        protect_without_propagation(path, false)?;
    }
    log(path);
    Ok(())
}

fn check_tree(dir: &Path) -> Result<()> {
    let listing =
        std::fs::read_dir(dir).with_context(|| format!("cannot list {}", dir.display()))?;
    for entry in listing {
        let path = entry
            .with_context(|| format!("cannot list {}", dir.display()))?
            .path();
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        if is_reparse_point(&metadata) {
            bail!(
                "{} is a reparse point; an administrator removes the link, then runs the command again",
                path.display()
            );
        }
        if exposure(&path)? == Exposure::ForeignOwner {
            bail!(
                "{} is owned by an account that is not SYSTEM or Administrators; an administrator checks that file and removes it (the file, not the directory), then runs the command again",
                path.display()
            );
        }
        if metadata.is_dir() {
            check_tree(&path)?;
        }
    }
    Ok(())
}

fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn is_old_restore_root(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        name.to_string_lossy()
            .eq_ignore_ascii_case(OLD_RESTORE_ROOT)
    })
}

/// The names that the agent keeps next to the configuration directory, below `root`.
fn root_entries(config: &Path, root: &Path) -> Vec<String> {
    let siblings = [
        config.to_path_buf(),
        crate::identity::sibling(config, ".staging"),
        crate::identity::sibling(config, ".old"),
        crate::restore_roots::roots_file_for(config),
        crate::scripts::mode_file_for(config),
        crate::scripts::allow_list_file_for(config),
    ];
    let mut names: Vec<String> = Vec::new();
    for path in siblings.iter().filter(|path| path.parent() == Some(root)) {
        names.extend(
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned()),
        );
        // An atomic replacement stages the file under this name.
        names.extend(
            path.with_extension("tmp")
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
        );
    }
    names.push(OLD_RESTORE_ROOT.to_owned());
    names
}

/// The names that the agent keeps in the configuration directory, including the staged and
/// temporary names of a rotation and of an atomic file replacement.
fn config_entries(config: &Path) -> Vec<String> {
    let files = [
        crate::identity::KEY_FILE,
        crate::identity::CERT_FILE,
        crate::identity::CA_CHAIN_FILE,
        crate::identity::METADATA_FILE,
        crate::identity::SIGNING_KEYS_FILE,
        crate::keys::KEY_UPDATES_FILE,
        crate::connection::STATUS_FILE,
        crate::journal::JOURNAL_FILE,
        crate::restic_store::RECORD_FILE,
    ];
    let mut names: Vec<String> = Vec::new();
    for file in files {
        names.push(file.to_owned());
        names.push(format!("{file}{}", crate::identity::STAGED_SUFFIX));
        names.push(
            Path::new(file)
                .with_extension("tmp")
                .to_string_lossy()
                .into_owned(),
        );
    }
    names.push(crate::identity::ROTATION_MARKER.to_owned());
    names.push(crate::log_spool::SPOOL_DIR.to_owned());
    names.push(crate::preflight::PROBE_FILE.to_owned());
    // Windows keeps the cache and, in older builds, restic below the configuration directory.
    names.extend(
        crate::paths::cache_dir()
            .strip_prefix(config)
            .ok()
            .and_then(|relative| relative.to_str())
            .map(str::to_owned),
    );
    names.push("bin".to_owned());
    names
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

    fn repair(config: &Path, root: &Path) -> (Result<()>, Vec<PathBuf>) {
        let mut repaired = Vec::new();
        let result = repair_agent_tree(config, root, &mut |path| repaired.push(path.to_path_buf()));
        (result, repaired)
    }

    #[test]
    fn fr_ag_04_a_foreign_grant_on_the_top_level_with_known_entries_is_repaired() {
        let (base, root, config) = tree("repair-top");
        create_protected_file(&config.join("agent-key.pem"), b"key").expect("cannot write");
        let roots = root.join("Agent-restore-roots.txt");
        create_protected_file(&roots, b"C:\\restore").expect("cannot write");
        let restore = root.join("Restore");
        std::fs::create_dir(&restore).expect("cannot create the old restore root");
        let restored = restore.join("restored.txt");
        std::fs::write(&restored, b"data").expect("cannot write");
        grant(&root, "(OI)(CI)(WD)").expect("cannot add the entry");
        grant(&roots, "(R,W)").expect("cannot add the entry");
        grant(&restored, "(R,W)").expect("cannot add the entry");
        assert!(check_admin_only(&root).is_err());

        let (result, repaired) = repair(&config, &root);
        result.expect("the repair must succeed");

        check_admin_only(&root).expect("the top level must be protected");
        check_admin_only(&roots).expect("the known file must be protected");
        check_admin_only(&config).expect("the identity directory must stay protected");
        assert_eq!(repaired, vec![root.clone(), roots]);
        assert_eq!(
            exposure(&restored).expect("cannot read"),
            Exposure::ForeignGrant,
            "the old restore tree must stay as it was"
        );
        create_protected_tree(&config, &root).expect("the normal check must pass now");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_a_protected_tree_is_not_repaired() {
        let (base, root, config) = tree("repair-clean");
        let (result, repaired) = repair(&config, &root);
        result.expect("the repair must succeed");
        assert!(repaired.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_the_config_directory_with_a_foreign_grant_is_repaired_with_its_files() {
        let (base, root, config) = tree("repair-config");
        let key = config.join("agent-key.pem");
        create_protected_file(&key, b"key").expect("cannot write");
        grant(&config, "(OI)(CI)(WD)").expect("cannot add the entry");
        grant(&key, "(R,W)").expect("cannot add the entry");
        let (result, repaired) = repair(&config, &root);
        result.expect("the repair must succeed");
        check_admin_only(&config).expect("the directory must be protected");
        check_admin_only(&key).expect("the key must be protected");
        assert_eq!(repaired, vec![config.clone(), key]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn fr_ag_04_an_unknown_entry_is_refused_and_named() {
        let (base, root, config) = tree("repair-unknown");
        std::fs::write(root.join("planted.exe"), b"x").expect("cannot write");
        grant(&root, "(OI)(CI)(WD)").expect("cannot add the entry");
        let (result, repaired) = repair(&config, &root);
        let error = format!(
            "{:#}",
            result.expect_err("an unknown entry must stop the repair")
        );
        assert!(error.contains("planted.exe"), "{error}");
        assert!(error.contains("the file, not the directory"), "{error}");
        assert!(repaired.is_empty());
        assert!(check_admin_only(&root).is_err(), "nothing was changed");
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
        let (result, repaired) = repair(&config, &root);
        let error = format!(
            "{:#}",
            result.expect_err("a foreign owner must stop the repair")
        );
        assert!(error.contains("Agent-restore-roots.txt"), "{error}");
        assert!(error.contains("the file, not the directory"), "{error}");
        assert!(repaired.is_empty());
        assert!(check_admin_only(&root).is_err(), "nothing was changed");
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
        let (result, repaired) = repair(&config, &root);
        result.expect("the repair leaves the refusal to the normal check");
        assert!(repaired.is_empty());
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

    #[test]
    fn fr_ag_04_the_normal_check_still_refuses_a_foreign_grant() {
        let (base, root, config) = tree("repair-service");
        grant(&root, "(OI)(CI)(WD)").expect("cannot add the entry");
        let error = format!(
            "{:#}",
            create_protected_tree(&config, &root).expect_err("the service must refuse")
        );
        assert!(error.contains("install-service"), "{error}");
        let _ = std::fs::remove_dir_all(&base);
    }
}
