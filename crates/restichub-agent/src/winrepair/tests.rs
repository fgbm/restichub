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
        .protect(Tree::Data)
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
        .protect(Tree::Data)
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
fn fr_ag_04_the_service_refuses_a_key_that_another_account_could_read() {
    let (base, root, config) = tree("check-key-read");
    crate::identity::tests::sample()
        .save(&config)
        .expect("cannot save the identity");
    grant(&config.join(crate::identity::CA_CHAIN_FILE), "(R)").expect("cannot add");
    check_identity(&config, &root).expect("a readable certificate chain passes");

    let key = config.join(crate::identity::KEY_FILE);
    grant(&key, "(R)").expect("cannot add the entry");
    let error = format!(
        "{:#}",
        check_identity(&config, &root).expect_err("a readable key")
    );
    assert!(error.contains("agent-key.pem"), "{error}");
    assert!(error.contains("holds the private key"), "{error}");
    let (result, _) = repair(&config, &root);
    assert!(result.expect("the repair must succeed").is_empty());
    check_identity(&config, &root).expect("the repaired key passes");
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
fn fr_ag_04_a_writable_identity_directory_stays_as_found_until_a_new_registration_replaces_it() {
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
fn fr_ag_04_a_ca_chain_that_users_could_change_is_reported_and_a_new_registration_replaces_it() {
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
fn adr_0045_the_repository_roots_file_is_a_known_entry() {
    let (base, root, config) = tree("repair-repository-roots");
    let file = crate::repository_roots::roots_file_for(&config);
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .expect("the file has a name");
    assert!(
        root_entries(&config, &root)
            .iter()
            .any(|(entry, _)| *entry == name),
        "{name} must be a known entry"
    );
    create_protected_file(&file, b"/srv/restic").expect("cannot write");
    grant(&file, "(R,W)").expect("cannot add the entry");
    grant(&root, "(OI)(CI)(WD)").expect("cannot add the entry");
    let (result, actions) = repair(&config, &root);
    result.expect("the repair must succeed");
    assert!(!file.exists(), "a writable roots file must be set aside");
    let [
        Action::Protected(protected),
        Action::MovedRepositoryRoots { from, .. },
    ] = actions.as_slice()
    else {
        panic!("unexpected actions: {actions:?}");
    };
    assert_eq!((protected, from), (&root, &file));
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
    let error = format!(
        "{:#}",
        result.expect_err("the repair refuses a foreign owner")
    );
    assert!(actions.is_empty());
    assert!(error.contains("/setowner *S-1-5-32-544"), "{error}");
    assert!(
        error.contains("O:BAG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"),
        "{error}"
    );
    assert!(error.contains("Set-Acl"), "{error}");
    assert!(!error.contains("deletes it"), "{error}");
    let _ = std::fs::remove_dir_all(&base);
}
