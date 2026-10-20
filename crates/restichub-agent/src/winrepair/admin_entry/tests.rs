#![allow(clippy::expect_used)]

use super::*;
use crate::winsec::tests::{current_user_sid, grant, grant_sid, grant_unresolved, set_owner, temp};
use crate::winsec::{
    check_admin_only, create_protected_dir, create_protected_file, create_protected_tree,
};

fn protected(name: &str) -> PathBuf {
    let dir = temp(name);
    create_protected_dir(&dir).expect("cannot create the protected directory");
    dir
}

fn tree(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let base = temp(name);
    std::fs::create_dir_all(&base).expect("cannot create the base");
    let root = base.join("ResticHub");
    let config = root.join("Agent");
    create_protected_tree(&config, &root).expect("cannot create the tree");
    (base, root, config)
}

fn admin_sid() -> String {
    current_user_sid().expect("cannot read the user SID")
}

fn heal_level(dir: &Path, rights: Rights) -> Option<Opened> {
    let found = level(dir, rights).expect("cannot check the access")?;
    found.remove(dir).expect("cannot remove the access");
    Some(found)
}

/// An unprotected child takes a copy of each entry that the parent grants to the children, as
/// Windows gives it below the folder that File Explorer opens (ADR-0048).
fn unprotected_child(path: &Path) {
    std::fs::write(path, b"x").expect("cannot write the child");
    set_owner(path, "S-1-5-32-544").expect("cannot give the child the owner of the tree");
}

fn repair_actions(config: &Path, root: &Path, healed: &[PathBuf]) -> Result<Vec<PathBuf>> {
    crate::winrepair::repair_after_heal(config, root, healed, &mut |_| {})
}

#[test]
fn ac_ag_15_an_explorer_entry_of_an_administrator_on_a_directory_is_removed() {
    let dir = protected("admin-entry");
    grant_sid(&dir, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    assert!(
        check_admin_only(&dir).is_err(),
        "the test must open the access"
    );
    let removed = heal_level(&dir, Rights::Change).expect("the entry must go");
    assert_eq!(removed.accounts.len(), 1, "{:?}", removed.accounts);
    check_admin_only(&dir).expect("the directory must be protected again");
    let entry = Entry::open(&dir)
        .expect("cannot open")
        .expect("the directory must exist");
    assert!(
        entry.exposure(Rights::ReadOrChange).expect("cannot read") == Exposure::Clean,
        "the descriptor must stay protected"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ac_ag_15_the_heal_removes_only_the_entry_of_the_administrator() {
    let dir = protected("only-entry");
    grant_sid(&dir, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    grant(&dir, "(OI)(CI)(RX)").expect("cannot add the read entry");
    heal_level(&dir, Rights::Change).expect("the entry of the administrator must go");
    check_admin_only(&dir).expect("the directory must be protected again");
    let entry = Entry::open(&dir)
        .expect("cannot open")
        .expect("the directory must exist");
    assert!(
        entry.grants_users_read().expect("cannot read the access"),
        "an entry that the checks allow must stay"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ac_ag_15_an_entry_of_a_non_administrator_stays() {
    let dir = protected("user-entry");
    grant(&dir, "(OI)(CI)(F)").expect("cannot add the entry");
    assert!(
        heal_level(&dir, Rights::Change).is_none(),
        "the entry of a group must stay"
    );
    assert!(check_admin_only(&dir).is_err(), "the check must refuse it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ac_ag_15_an_unresolved_sid_counts_as_a_non_administrator() {
    let dir = protected("unknown-entry");
    // A deleted account leaves its identifier in the access control list.
    grant_unresolved(&dir).expect("cannot add the entry");
    assert!(
        heal_level(&dir, Rights::Change).is_none(),
        "an identifier that does not resolve must stay"
    );
    assert!(check_admin_only(&dir).is_err(), "the check must refuse it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ac_ag_15_an_entry_on_a_file_is_not_removed() {
    let dir = protected("file-entry");
    let file = dir.join("restichub-agent.exe");
    create_protected_file(&file, b"x").expect("cannot write the file");
    grant_sid(&file, &admin_sid(), "(M)").expect("cannot add the entry");
    assert!(
        heal_level(&file, Rights::Change).is_none(),
        "a file keeps its refusal: the content may be changed already"
    );
    assert!(check_admin_only(&file).is_err(), "the check must refuse it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ac_ag_15_an_inherited_entry_is_not_removed() {
    let parent = protected("inherited-entry");
    grant_sid(&parent, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    let child = parent.join("cache");
    std::fs::create_dir(&child).expect("cannot create the child");
    assert!(
        heal_level(&child, Rights::Change).is_none(),
        "Explorer adds no inherited entry, so the heal must not touch it"
    );
    assert!(
        check_admin_only(&child).is_err(),
        "the check must refuse it"
    );
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn ac_ag_15_a_child_of_a_healed_program_level_loses_the_copy() {
    let base = temp("program-copy");
    std::fs::create_dir_all(&base).expect("cannot create the base");
    let root = base.join("ResticHub");
    let program = root.join("Agent");
    create_protected_tree(&program, &root).expect("cannot create the tree");
    let previous = program.join("restic-previous.exe");
    unprotected_child(&previous);
    let bin = program.join("bin");
    std::fs::create_dir(&bin).expect("cannot create the directory");
    let old = bin.join("restic.exe");
    unprotected_child(&old);
    grant_sid(&program, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    assert!(
        check_admin_only(&previous).is_err() && check_admin_only(&old).is_err(),
        "the children must carry the copy before the heal"
    );
    let removed =
        heal_level(&program, Rights::Change).expect("the entry of the administrator must go");
    restamp_children(&program, &removed.trustees).expect("cannot remove the copies");
    check_admin_only(&previous).expect("the file must lose the copy");
    check_admin_only(&old).expect("the file in bin must lose the copy");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_ag_15_the_repair_reaches_the_children_of_a_healed_level() {
    let (base, root, config) = tree("heal-children");
    let logs = crate::logging::service_log_dir(&config);
    create_protected_dir(&logs).expect("cannot create the log directory");
    let log = logs.join("agent-000001.log");
    unprotected_child(&log);
    let cache = config.join("cache");
    create_protected_dir(&cache).expect("cannot create the cache");
    let blob = cache.join("blob");
    unprotected_child(&blob);
    let sid = admin_sid();
    for path in [&logs, &cache] {
        grant_sid(path, &sid, "(OI)(CI)(F)").expect("cannot add the entry");
    }
    assert!(
        check_admin_only(&log).is_err() && check_admin_only(&blob).is_err(),
        "the children must carry the copy before the heal"
    );
    let mut healed = Vec::new();
    for path in [&logs, &cache] {
        let removed =
            heal_level(path, Rights::Change).expect("the entry of the administrator must go");
        assert_eq!(removed.accounts.len(), 1, "{:?}", removed.accounts);
        healed.push(path.clone());
    }
    let changeable = repair_actions(&config, &root, &healed).expect("the repair must pass");
    assert!(changeable.is_empty(), "{changeable:?}");
    check_admin_only(&log).expect("the log file must lose the copy");
    check_admin_only(&blob).expect("the cache file must lose the copy");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_ag_15_a_name_that_the_agent_does_not_know_stops_the_start() {
    let (base, root, config) = tree("heal-planted");
    let planted = root.join("planted.txt");
    std::fs::write(&planted, b"x").expect("cannot plant the file");
    grant_sid(&root, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    heal_level(&root, Rights::Change).expect("the entry of the administrator must go");
    let error = format!(
        "{:#}",
        repair_actions(&config, &root, std::slice::from_ref(&root))
            .expect_err("a name that the agent does not know must stop the start")
    );
    assert!(error.contains("planted.txt"), "{error}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_ag_15_a_lock_in_a_healed_level_goes() {
    let (base, root, config) = tree("heal-lock");
    let lock = crate::identity::lock_file(&config);
    std::fs::write(&lock, b"x").expect("cannot write the lock");
    grant_sid(&root, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    heal_level(&root, Rights::Change).expect("the entry of the administrator must go");
    repair_actions(&config, &root, std::slice::from_ref(&root)).expect("the repair must pass");
    assert!(
        !lock.exists(),
        "a lock that another account could hold must not wait for the identity load"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_ag_15_a_folder_of_another_owner_stops_the_start() {
    let (base, root, config) = tree("heal-owner");
    let spool = config.join(crate::log_spool::SPOOL_DIR);
    create_protected_dir(&spool).expect("cannot create the spool");
    set_owner(
        &spool,
        &current_user_sid().expect("cannot read the user SID"),
    )
    .expect("cannot change the owner");
    grant_sid(&root, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    heal_level(&root, Rights::Change).expect("the entry of the administrator must go");
    let error = format!(
        "{:#}",
        repair_actions(&config, &root, std::slice::from_ref(&root))
            .expect_err("a folder of another owner must stop the start")
    );
    assert!(error.contains("owned by an account"), "{error}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_ag_15_an_open_identity_directory_keeps_the_refusal() {
    let (base, root, config) = tree("heal-identity");
    grant_sid(&config, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    let changeable = repair_actions(&config, &root, &[]).expect("the repair must pass");
    assert_eq!(changeable.as_slice(), std::slice::from_ref(&config));
    assert!(
        check_admin_only(&config).is_err(),
        "the identity directory stays open, so a new registration replaces it"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_ag_15_the_heal_leaves_the_identity_directory_alone() {
    let (base, _, config) = tree("heal-levels");
    let healed: Vec<PathBuf> = levels(&config)
        .into_iter()
        .map(|(path, _, _)| path)
        .collect();
    assert!(!healed.contains(&config), "the identity directory stays");
    assert!(
        !healed.contains(&crate::identity::sibling(&config, ".old")),
        "the backup of the identity stays"
    );
    assert!(healed.contains(&crate::paths::agent_root()), "{healed:?}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_ag_15_a_mixed_dacl_is_not_healed() {
    let dir = protected("mixed-entry");
    grant_sid(&dir, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    grant(&dir, "(OI)(CI)(WD)").expect("cannot add the entry");
    assert!(
        heal_level(&dir, Rights::Change).is_none(),
        "an entry of another trustee must keep the refusal"
    );
    assert!(check_admin_only(&dir).is_err(), "the check must refuse it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ac_ag_15_a_planted_library_after_the_heal_stops_the_start() {
    let base = temp("planted-library");
    std::fs::create_dir_all(&base).expect("cannot create the base");
    let root = base.join("ResticHub");
    let program = root.join("Agent");
    create_protected_tree(&program, &root).expect("cannot create the tree");
    create_protected_file(&program.join("restichub-agent.exe"), b"x")
        .expect("cannot write the agent");
    grant_sid(&root, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    assert!(
        heal_level(&root, Rights::Change).is_some(),
        "the entry of the administrator must go"
    );
    // A library that the account planted while the entry existed loads before `main`.
    std::fs::write(program.join("version.dll"), b"x").expect("cannot plant the library");
    let error = format!(
        "{:#}",
        crate::winrepair::check_program_tree(&program, &root)
            .expect_err("a planted library must stop the start")
    );
    assert!(error.contains("version.dll"), "{error}");
    let _ = std::fs::remove_dir_all(&base);
}

fn data_level(root: &Path) -> Vec<(PathBuf, Tree, Rights)> {
    vec![(root.to_path_buf(), Tree::Data, Rights::Change)]
}

#[test]
fn ac_ag_15_a_clean_level_loses_the_entry_and_the_start_continues() {
    let (base, root, config) = tree("heal-clean");
    grant_sid(&root, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    let program = base.join("Program").join("Agent");
    let healed_program =
        heal_levels(&config, &root, &program, data_level(&root)).expect("the heal must pass");
    assert!(!healed_program, "no level of the program tree changed");
    check_admin_only(&root).expect("the entry must go");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_ag_15_a_refused_heal_keeps_the_entry_for_the_next_start() {
    let (base, root, config) = tree("heal-refused");
    std::fs::write(root.join("planted.txt"), b"x").expect("cannot plant the file");
    grant_sid(&root, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    let program = base.join("Program").join("Agent");
    for start in ["first", "second"] {
        let error = format!(
            "{:#}",
            heal_levels(&config, &root, &program, data_level(&root))
                .expect_err("a name that the agent does not know must stop each start")
        );
        assert!(error.contains("planted.txt"), "{start} start: {error}");
        assert!(
            check_admin_only(&root).is_err(),
            "the {start} start must keep the entry, so the next start refuses again"
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_ag_15_a_refused_program_heal_keeps_the_entry_for_the_next_start() {
    let base = temp("program-refused");
    std::fs::create_dir_all(&base).expect("cannot create the base");
    let root = base.join("ResticHub");
    let program = root.join("Agent");
    create_protected_tree(&program, &root).expect("cannot create the tree");
    create_protected_file(&program.join("restichub-agent.exe"), b"x")
        .expect("cannot write the agent");
    std::fs::write(program.join("version.dll"), b"x").expect("cannot plant the library");
    grant_sid(&root, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    let levels = vec![
        (root.clone(), Tree::Program, Rights::Change),
        (program.clone(), Tree::Program, Rights::Change),
    ];
    let data = base.join("Data");
    for start in ["first", "second"] {
        let error = format!(
            "{:#}",
            heal_levels(&data.join("Agent"), &data, &program, levels.clone())
                .expect_err("a planted library must stop each start")
        );
        assert!(error.contains("version.dll"), "{start} start: {error}");
        assert!(
            check_admin_only(&root).is_err(),
            "the {start} start must keep the entry, so the next start refuses again"
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_ag_15_a_child_with_inherited_entries_only_keeps_system_and_administrators() {
    let parent = protected("inherited-only");
    grant_sid(&parent, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    let child = parent.join("restic.exe");
    std::fs::write(&child, b"x").expect("cannot write the child");
    let found = level(&parent, Rights::Change)
        .expect("cannot check the access")
        .expect("the parent must hold the entry");
    let entry = Entry::open(&child)
        .expect("cannot open the child")
        .expect("the child must exist");
    assert!(
        entry
            .remove_permits(&found.trustees, Removal::InheritedCopies)
            .expect("cannot remove the copy"),
        "the child must carry the copy"
    );
    drop(entry);
    check_admin_only(&child).expect("the child must lose the copy");
    // An empty DACL passes the check above but denies every access.
    std::fs::read(&child).expect("SYSTEM and Administrators must keep the access of the child");
    let _ = std::fs::remove_dir_all(&parent);
}
