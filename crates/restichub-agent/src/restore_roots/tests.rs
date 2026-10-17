#![allow(clippy::expect_used)]

use super::*;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "restichub-agent-roots-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    dir.canonicalize().expect("cannot resolve the directory")
}

fn access(root: &Path, config: &Path) -> RestoreAccess {
    RestoreAccess::new(
        vec![root.to_path_buf()],
        vec![config.to_path_buf(), PathBuf::from("/etc/restichub-agent")],
    )
}

#[test]
fn ac_rs_05_the_default_root_is_below_var_lib() {
    assert_eq!(default_root(), PathBuf::from("/var/lib/restichub-restore"));
}

#[test]
fn ac_rs_05_a_target_inside_a_root_is_accepted() {
    let base = temp_dir("inside");
    let root = base.join("restore");
    let config = base.join("config");
    std::fs::create_dir_all(&root).expect("cannot create the root");
    std::fs::create_dir_all(&config).expect("cannot create the config");
    let access = access(&root, &config);
    let target = root.join("job-1");
    assert_eq!(access.check(target.to_str().expect("utf-8")), Ok(()));
    let deep = root.join("a").join("b");
    assert_eq!(access.check(deep.to_str().expect("utf-8")), Ok(()));
    assert_eq!(access.check(root.to_str().expect("utf-8")), Ok(()));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_rs_05_the_filesystem_root_and_system_paths_are_refused() {
    let base = temp_dir("system");
    let root = base.join("restore");
    std::fs::create_dir_all(&root).expect("cannot create the root");
    let access = access(&root, &base.join("config"));
    assert!(access.check("/").is_err());
    assert_eq!(access.check("/etc"), Err(TargetDenied::ProtectedPath));
    assert_eq!(access.check("/etc/passwd"), Err(TargetDenied::OutsideRoots));
    assert_eq!(
        access.check("relative/path"),
        Err(TargetDenied::NotAbsolute)
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_rs_05_a_path_outside_the_roots_is_refused() {
    let base = temp_dir("outside");
    let root = base.join("restore");
    std::fs::create_dir_all(&root).expect("cannot create the root");
    let access = access(&root, &base.join("config"));
    let other = base.join("other");
    assert_eq!(
        access.check(other.to_str().expect("utf-8")),
        Err(TargetDenied::OutsideRoots)
    );
    let sibling = base.join("restore2");
    assert_eq!(
        access.check(sibling.to_str().expect("utf-8")),
        Err(TargetDenied::OutsideRoots)
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_rs_05_the_config_dir_and_what_lies_inside_or_above_it_are_refused() {
    let base = temp_dir("config");
    let config = base.join("restore").join("config");
    std::fs::create_dir_all(&config).expect("cannot create the config");
    // A root that holds the config dir shows that the protected rule wins over the root rule.
    let access = access(&base, &config);
    for target in [
        config.clone(),
        config.join("identity.json"),
        base.join("restore"),
        base.clone(),
    ] {
        assert_eq!(
            access.check(target.to_str().expect("utf-8")),
            Err(TargetDenied::ProtectedPath),
            "{}",
            target.display()
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn t20_the_restic_directory_is_a_protected_path() {
    let base = temp_dir("restic-dir");
    let bin = base.join("bin");
    std::fs::create_dir_all(&bin).expect("cannot create the bin directory");
    let access = RestoreAccess::for_host(
        &base.join("config"),
        &base.join("cache"),
        &bin.join("restic"),
    );
    assert_eq!(
        access.check(bin.join("restic").to_str().expect("utf-8")),
        Err(TargetDenied::ProtectedPath)
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn t20_the_program_and_data_directories_are_protected_paths() {
    let base = temp_dir("program-dir");
    let access = RestoreAccess::for_host(
        &base.join("config"),
        &base.join("cache"),
        &base.join("bin").join("restic"),
    );
    for dir in [crate::paths::program_dir(), crate::paths::data_dir()] {
        let target = dir.join("restore");
        assert_eq!(
            access.check(target.to_str().expect("utf-8")),
            Err(TargetDenied::ProtectedPath),
            "{}",
            target.display()
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_rs_05_a_link_to_the_config_dir_is_refused() {
    let base = temp_dir("link");
    let root = base.join("restore");
    let config = base.join("config");
    std::fs::create_dir_all(&root).expect("cannot create the root");
    std::fs::create_dir_all(&config).expect("cannot create the config");
    std::os::unix::fs::symlink(&config, root.join("link")).expect("cannot create the link");
    let access = access(&root, &config);
    for target in [root.join("link"), root.join("link").join("new")] {
        assert_eq!(
            access.check(target.to_str().expect("utf-8")),
            Err(TargetDenied::ProtectedPath),
            "{}",
            target.display()
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_rs_05_a_link_out_of_the_root_is_refused() {
    let base = temp_dir("escape");
    let root = base.join("restore");
    let outside = base.join("outside");
    std::fs::create_dir_all(&root).expect("cannot create the root");
    std::fs::create_dir_all(&outside).expect("cannot create the outside");
    std::os::unix::fs::symlink(&outside, root.join("link")).expect("cannot create the link");
    std::os::unix::fs::symlink(base.join("missing"), root.join("dangling"))
        .expect("cannot create the link");
    let access = access(&root, &base.join("config"));
    let escaped = root.join("link").join("x");
    assert_eq!(
        access.check(escaped.to_str().expect("utf-8")),
        Err(TargetDenied::OutsideRoots)
    );
    let dangling = root.join("dangling").join("x");
    assert_eq!(
        access.check(dangling.to_str().expect("utf-8")),
        Err(TargetDenied::Unresolvable)
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_rs_05_a_dot_dot_component_is_refused() {
    let base = temp_dir("dotdot");
    let root = base.join("restore");
    std::fs::create_dir_all(&root).expect("cannot create the root");
    let access = access(&root, &base.join("config"));
    let target = format!("{}/../config", root.display());
    assert_eq!(access.check(&target), Err(TargetDenied::NotAbsolute));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_rs_05_a_root_of_slash_allows_an_in_place_restore_but_not_the_agent_files() {
    let base = temp_dir("slash");
    let config = base.join("config");
    std::fs::create_dir_all(&config).expect("cannot create the config");
    let access = RestoreAccess::new(vec![PathBuf::from("/")], vec![config.clone()]);
    let data = base.join("data");
    assert_eq!(access.check(data.to_str().expect("utf-8")), Ok(()));
    assert_eq!(
        access.check(config.to_str().expect("utf-8")),
        Err(TargetDenied::ProtectedPath)
    );
    assert_eq!(access.check("/"), Err(TargetDenied::ProtectedPath));
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn adr_0045_a_restore_into_a_repository_root_is_refused() {
    let base = temp_dir("repository-root");
    let config = base.join("config");
    let repos = base.join("repos");
    std::fs::create_dir_all(&config).expect("cannot create the config");
    std::fs::create_dir_all(repos.join("fs-01")).expect("cannot create the repository");
    let protected = restore_protected(vec![config], vec![repos.clone()]);
    for root in [PathBuf::from("/"), base.clone()] {
        let access = RestoreAccess::new(vec![root.clone()], protected.clone());
        for target in [
            repos.clone(),
            repos.join("fs-01").join("config"),
            base.clone(),
        ] {
            assert_eq!(
                access.check(target.to_str().expect("utf-8")),
                Err(TargetDenied::ProtectedPath),
                "{} below {}",
                target.display(),
                root.display()
            );
        }
        let other = base.join("data");
        assert_eq!(access.check(other.to_str().expect("utf-8")), Ok(()));
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_rs_05_the_roots_file_lists_absolute_paths() {
    let (roots, skipped) = parse_roots("# comment\n\n/srv/restore\n  /data  \nrelative\n../x\n");
    assert_eq!(
        roots,
        vec![
            (3, PathBuf::from("/srv/restore")),
            (4, PathBuf::from("/data"))
        ]
    );
    assert_eq!(
        skipped,
        vec![(5, "relative".to_owned()), (6, "../x".to_owned())]
    );
}

#[test]
fn adr_0045_a_root_line_with_unit_syntax_is_skipped() {
    let (roots, skipped) = parse_roots(
        "/srv/\" \"/etc\n/srv/'a'\n/srv/a b\n/srv/a\tb\n/srv/a\\b\n/srv/%h\n/srv/a\u{7}\n/srv/ok\n",
    );
    assert_eq!(roots, vec![(8, PathBuf::from("/srv/ok"))]);
    assert_eq!(skipped.len(), 7);
}

#[test]
fn adr_0045_install_service_refuses_a_skipped_root_line_by_its_number() {
    let file = Path::new("/etc/restichub-agent-repository-roots.txt");
    let error = skipped_line_error(file, "# comment\n/srv/ok\n/srv/a b\nrelative\n")
        .expect_err("the line must be refused");
    let message = format!("{error:#}");
    assert!(
        message.contains("line 3 of /etc/restichub-agent-repository-roots.txt"),
        "{message}"
    );
    assert!(message.contains("\"/srv/a b\""), "{message}");
    assert!(skipped_line_error(file, "# comment\n\n/srv/ok\n").is_ok());
}

#[test]
fn ac_rs_05_install_service_refuses_the_restore_root_home_or_root() {
    let file = Path::new("/etc/restichub-agent-restore-roots.txt");
    for (text, line) in [
        ("/srv/ok\n/home\n", 2),
        ("/home/\n", 1),
        ("# c\n//root\n", 2),
    ] {
        let error = home_root_error(file, text).expect_err("the root must be refused");
        let message = format!("{error:#}");
        assert!(
            message.contains(&format!(
                "line {line} of /etc/restichub-agent-restore-roots.txt"
            )),
            "{message}"
        );
        assert!(message.contains("below it, owned by root"), "{message}");
    }
    assert!(home_root_error(file, "/home/restore\n/root/restore\n/srv\n").is_ok());
}

#[test]
fn ac_rs_05_the_agent_skips_the_restore_root_home_or_root() {
    let file = Path::new("/etc/restichub-agent-restore-roots.txt");
    assert!(!usable_restore_root(file, Path::new("/home")));
    assert!(!usable_restore_root(file, Path::new("/root/")));
    assert!(usable_restore_root(file, Path::new("/home/restore")));
    assert!(usable_restore_root(file, Path::new("/srv/restore")));
}

#[test]
fn adr_0045_a_restore_keeps_away_from_an_untrusted_repository_root() {
    use std::os::unix::fs::PermissionsExt as _;
    let base = temp_dir("untrusted-repository-root");
    let config = base.join("config");
    let repos = base.join("repos");
    std::fs::create_dir_all(&config).expect("cannot create the config");
    std::fs::create_dir_all(repos.join("fs-01")).expect("cannot create the repository");
    std::fs::set_permissions(&repos, std::fs::Permissions::from_mode(0o777))
        .expect("cannot set the mode");
    let file = crate::repository_roots::roots_file_for(&config);
    std::fs::write(&file, format!("{}\n", repos.display())).expect("cannot write the file");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o666))
        .expect("cannot set the mode");
    assert!(load_roots_file(&file).is_empty());
    let access = RestoreAccess::new(
        vec![PathBuf::from("/")],
        restore_protected(vec![config], listed_roots(&file)),
    );
    for target in [repos.clone(), repos.join("fs-01").join("config")] {
        assert_eq!(
            access.check(target.to_str().expect("utf-8")),
            Err(TargetDenied::ProtectedPath),
            "{}",
            target.display()
        );
    }
    assert!(listed_roots(&base.join("absent.txt")).is_empty());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_rs_05_a_roots_file_of_a_user_account_is_ignored() {
    let base = temp_dir("untrusted");
    let file = base.join("roots.txt");
    std::fs::write(&file, "/srv/extra\n").expect("cannot write the file");
    // The check compares the owner with root, so the test needs a non-root account.
    let owner = std::os::unix::fs::MetadataExt::uid(
        &std::fs::metadata(&file).expect("cannot read the file"),
    );
    if owner == 0 {
        let _ = std::fs::remove_dir_all(&base);
        return;
    }
    let roots = load_roots(&file);
    assert_eq!(roots, vec![default_root()]);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_rs_05_a_root_writable_by_others_is_ignored() {
    use std::os::unix::fs::PermissionsExt as _;
    let base = temp_dir("writable-root");
    let root = base.join("open");
    std::fs::create_dir_all(&root).expect("cannot create the root");
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o777))
        .expect("cannot set the mode");
    assert!(check_root(&root).is_err());
    // A directory below the open root fails through its ancestor.
    let below = root.join("below");
    std::fs::create_dir_all(&below).expect("cannot create the directory");
    std::fs::set_permissions(&below, std::fs::Permissions::from_mode(0o700))
        .expect("cannot set the mode");
    assert!(check_root(&below).is_err());
    assert!(check_root(&root.join("missing").join("x")).is_err());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_hk_02_a_restore_cannot_touch_the_trust_mode_file() {
    let base = temp_dir("trust-file");
    let config = base.join("config");
    std::fs::create_dir_all(&config).expect("cannot create the config");
    let access = RestoreAccess::for_host(&config, &base.join("cache"), &base.join("restic"));
    let mode_file = crate::scripts::mode_file_for(&config);
    assert_eq!(
        access.check(mode_file.to_str().expect("utf-8")),
        Err(TargetDenied::ProtectedPath)
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn ac_rs_05_the_roots_file_is_a_sibling_of_the_config_dir() {
    assert_eq!(
        roots_file_for(Path::new("/etc/restichub-agent")),
        PathBuf::from("/etc/restichub-agent-restore-roots.txt")
    );
}

#[test]
fn ac_rs_05_the_roots_are_reported_as_text() {
    let access = RestoreAccess::new(vec![PathBuf::from("/srv/restore")], Vec::new());
    assert_eq!(access.root_strings(), vec!["/srv/restore".to_owned()]);
}
