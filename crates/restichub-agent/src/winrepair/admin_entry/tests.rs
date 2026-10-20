#![allow(clippy::expect_used)]

use super::*;
use crate::winsec::tests::{current_user_sid, grant, grant_sid, temp};
use crate::winsec::{
    check_admin_only, create_protected_dir, create_protected_file, create_protected_tree,
};

fn protected(name: &str) -> PathBuf {
    let dir = temp(name);
    create_protected_dir(&dir).expect("cannot create the protected directory");
    dir
}

fn admin_sid() -> String {
    current_user_sid().expect("cannot read the user SID")
}

fn heal_level(dir: &Path, rights: Rights) -> Option<Vec<Account>> {
    level(dir, Tree::Data, rights).expect("cannot remove the access")
}

#[test]
fn adr_0048_an_explorer_entry_of_an_administrator_on_a_directory_is_removed() {
    let dir = protected("admin-entry");
    grant_sid(&dir, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    assert!(
        check_admin_only(&dir).is_err(),
        "the test must open the access"
    );
    let accounts = heal_level(&dir, Rights::Change).expect("the entry must go");
    assert_eq!(accounts.len(), 1, "{accounts:?}");
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
fn adr_0048_an_entry_of_a_non_administrator_stays() {
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
fn adr_0048_an_unresolved_sid_counts_as_a_non_administrator() {
    let dir = protected("unknown-entry");
    // A deleted account leaves its identifier in the access control list.
    grant_sid(&dir, "S-1-5-21-1-2-3-4242", "(OI)(CI)(F)").expect("cannot add the entry");
    assert!(
        heal_level(&dir, Rights::Change).is_none(),
        "an identifier that does not resolve must stay"
    );
    assert!(check_admin_only(&dir).is_err(), "the check must refuse it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn adr_0048_an_entry_on_a_file_is_not_removed() {
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
fn adr_0048_an_inherited_entry_is_not_removed() {
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
fn adr_0048_a_mixed_dacl_is_not_healed() {
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
fn adr_0048_a_planted_library_after_the_heal_stops_the_start() {
    let base = temp("planted-library");
    std::fs::create_dir_all(&base).expect("cannot create the base");
    let root = base.join("ResticHub");
    let program = root.join("Agent");
    create_protected_tree(&program, &root).expect("cannot create the tree");
    create_protected_file(&program.join("restichub-agent.exe"), b"x")
        .expect("cannot write the agent");
    grant_sid(&root, &admin_sid(), "(OI)(CI)(F)").expect("cannot add the entry");
    assert!(
        level(&root, Tree::Program, Rights::Change)
            .expect("cannot remove the access")
            .is_some(),
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
