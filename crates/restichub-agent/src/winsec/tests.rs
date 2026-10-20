#![allow(clippy::expect_used)]

use super::*;

pub(crate) fn temp(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "restichub-agent-winsec-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn a_directory_that_another_account_owns_is_refused() {
    // The base is SYSTEM and Administrators only, so the refusal names the inner directory.
    let base = temp("owner-base");
    create_protected_dir(&base).expect("cannot create the protected base");
    let dir = base.join("agent");
    create_protected_dir(&dir).expect("cannot create the protected directory");
    // An administrator owns a new directory through Administrators, so the test sets the owner
    // to the account of the current process, which is not SYSTEM or Administrators.
    let sid = current_user_sid().expect("cannot read the user SID");
    set_owner(&dir, &sid).expect("cannot set the owner");
    assert!(check_admin_only(&dir).is_err());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fr_ag_04_a_protected_file_never_holds_the_access_of_its_parent() {
    let dir = temp("open-parent");
    create_protected_dir(&dir).expect("cannot create the directory");
    grant(&dir, "(OI)(CI)(F)").expect("cannot add the access control entry");
    let inherited = dir.join("inherited");
    std::fs::write(&inherited, b"probe").expect("cannot write the probe");
    assert!(
        check_admin_only(&inherited).is_err(),
        "the parent passes its access on"
    );

    let key = dir.join("agent-key.pem");
    create_protected_file(&key, b"key").expect("cannot write the key");
    check_admin_only(&key).expect("the key holds only the protected access");
    let journal = dir.join("journal");
    drop(open_or_create_protected(&journal).expect("cannot create the file"));
    check_admin_only(&journal).expect("the new file holds only the protected access");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_directory_with_a_write_ace_for_users_is_refused() {
    let dir = temp("acl-write");
    create_protected_dir(&dir).expect("cannot create the protected directory");
    grant(&dir, "(OI)(CI)(WD)").expect("cannot add the access control entry");
    assert!(check_admin_only(&dir).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_access_control_change_reports_the_returned_code() {
    let base = temp("acl-missing");
    std::fs::create_dir_all(&base).expect("cannot create the base");
    let missing = base.join("absent");
    let error = apply_descriptor(&missing, DIR_SDDL).expect_err("a missing path must fail");
    let text = format!("{error:#}");
    // ERROR_FILE_NOT_FOUND, not the stale last error of the thread.
    assert!(text.contains("os error 2"), "{text}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_directory_with_a_write_owner_ace_for_users_is_refused() {
    let dir = temp("acl-owner");
    create_protected_dir(&dir).expect("cannot create the protected directory");
    grant(&dir, "(OI)(CI)(WO)").expect("cannot add the access control entry");
    assert!(check_admin_only(&dir).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ac_hk_08_a_program_directory_where_users_add_files_is_refused() {
    // `(W)` grants write data and attributes: the right to add a file, without a delete.
    for rights in ["(W)", "(OI)(CI)(WD)", "(AD)"] {
        let dir = temp("program-dir");
        create_protected_dir(&dir).expect("cannot create the protected directory");
        check_program_directory(&dir).expect("a protected directory must pass");
        grant(&dir, rights).expect("cannot add the access control entry");
        assert!(check_program_directory(&dir).is_err(), "{rights}");
        // A parent check accepts the same directory, so it cannot guard a program directory.
        if rights == "(W)" {
            check_parent(&dir).expect("a parent may let users add files");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn fr_ag_04_a_tree_with_two_missing_levels_is_created_protected() {
    let base = temp("tree-base");
    std::fs::create_dir_all(&base).expect("cannot create the base");
    let root = base.join("ResticHub");
    let dir = root.join("Agent").join("cache");
    create_protected_tree(&dir, &root).expect("cannot create the tree");
    for level in [&root, &root.join("Agent"), &dir] {
        check_admin_only(level).expect("a created level must be SYSTEM and Administrators only");
    }
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fr_ag_04_a_tree_below_a_user_writable_level_is_refused() {
    let base = temp("tree-writable");
    std::fs::create_dir_all(&base).expect("cannot create the base");
    let root = base.join("ResticHub");
    create_protected_dir(&root).expect("cannot create the protected directory");
    grant(&root, "(OI)(CI)(WD)").expect("cannot add the access control entry");
    let dir = root.join("Agent").join("cache");
    assert!(create_protected_tree(&dir, &root).is_err());
    assert!(!root.join("Agent").exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fr_ag_04_an_existing_level_is_not_protected_again() {
    let base = temp("tree-twice");
    std::fs::create_dir_all(&base).expect("cannot create the base");
    let root = base.join("ResticHub");
    let dir = root.join("Agent").join("cache");
    create_protected_tree(&dir, &root).expect("cannot create the tree");
    let mut created = Vec::new();
    protect_tree(&dir, &root, &root, &mut |level| {
        created.push(level.to_path_buf());
        Ok(())
    })
    .expect("cannot check the existing tree");
    assert!(created.is_empty(), "an existing level was created again");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fr_ag_04_only_a_level_of_the_agent_tree_names_the_repair() {
    let base = temp("tree-advice");
    std::fs::create_dir_all(&base).expect("cannot create the base");
    let root = base.join("ResticHub");
    let dir = root.join("Agent");
    create_protected_tree(&dir, &root).expect("cannot create the tree");
    grant(&root, "(OI)(CI)(WD)").expect("cannot add the access control entry");
    let refuse = |repaired: &Path| {
        let error = protect_tree(&dir, &root, repaired, &mut |_| Ok(()))
            .expect_err("an open level must be refused");
        format!("{error:#}")
    };
    let agent = refuse(&root);
    assert!(agent.contains("install-service"), "{agent}");
    let other = refuse(&base.join("elsewhere"));
    assert!(!other.contains("install-service"), "{other}");
    assert!(
        other.contains("removes the entry of that account"),
        "{other}"
    );
    let program = format!(
        "{:#}",
        check_program_directory(&root).expect_err("an open program directory")
    );
    assert!(!program.contains("install-service"), "{program}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fr_ag_04_the_security_of_an_existing_child_stays_after_a_second_call() {
    let base = temp("tree-child");
    std::fs::create_dir_all(&base).expect("cannot create the base");
    let root = base.join("ResticHub");
    let dir = root.join("cache");
    create_protected_tree(&dir, &root).expect("cannot create the tree");
    let child = dir.join("data");
    create_protected_file(&child, b"x").expect("cannot write the child");
    let before = std::fs::metadata(&child)
        .and_then(|metadata| metadata.modified())
        .expect("cannot read the child");
    create_protected_tree(&dir, &root).expect("cannot check the tree again");
    let after = std::fs::metadata(&child)
        .and_then(|metadata| metadata.modified())
        .expect("cannot read the child");
    assert_eq!(before, after);
    check_admin_only(&child).expect("the child must stay SYSTEM and Administrators only");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn fr_ag_04_a_junction_level_is_refused() {
    let base = temp("tree-junction");
    let root = base.join("ResticHub");
    let target = base.join("target");
    std::fs::create_dir_all(&target).expect("cannot create the target");
    create_protected_dir(&root).expect("cannot create the protected root");
    let junction = root.join("Agent");
    let status = std::process::Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(&junction)
        .arg(&target)
        .status()
        .expect("cannot run mklink");
    assert!(status.success(), "cannot create the junction");
    let error = create_protected_tree(&junction.join("cache"), &root)
        .expect_err("a junction level must be refused");
    assert!(format!("{error:#}").contains("reparse point"), "{error:#}");
    assert!(!target.join("cache").exists());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn a_plain_file_is_refused() {
    let file = temp("plain.pem");
    std::fs::write(&file, b"key").expect("cannot write the file");
    assert!(check_admin_only(&file).is_err());
    let _ = std::fs::remove_file(&file);
}

#[test]
fn the_key_file_is_admin_only() {
    let dir = temp("dir");
    create_protected_dir(&dir).expect("cannot create the protected directory");
    let file = dir.join("agent-key.pem");
    create_protected_file(&file, b"key").expect("cannot write the protected file");
    check_admin_only(&file).expect("the key file must be SYSTEM and Administrators only");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_program_files_acl_passes_the_agent_rule() {
    // The real Program Files DACL grants Users a read and TrustedInstaller a full control.
    let dir = temp("program-files-acl");
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    copy_program_files_dacl(&dir).expect("cannot copy the DACL");
    check_admin_only(&dir).expect("the Program Files DACL must pass");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_program_files_parent_chain_passes() {
    let mut current = Some(std::path::Path::new(r"C:\Program Files"));
    while let Some(dir) = current {
        check_parent(dir).unwrap_or_else(|error| panic!("{} must pass: {error}", dir.display()));
        current = dir.parent();
    }
}

#[test]
fn adr_0048_the_program_tree_grants_users_read_and_execute() {
    let base = temp("program-tree");
    std::fs::create_dir_all(&base).expect("cannot create the base");
    let root = base.join("ResticHub");
    let program = root.join("Agent");
    create_program_tree(&program, &root).expect("cannot create the program tree");
    let file = program.join("restichub-agent.exe");
    create_program_file(&file, b"x").expect("cannot write the program file");
    for path in [&root, &program, &file] {
        check_admin_only(path).expect("a read for Users is no right to change");
        assert!(
            grants_users_read(path),
            "{} must grant Users the read and execute access",
            path.display()
        );
    }
    check_program_directory(&root).expect("the program tree must pass the program check");
    check_program_directory(&program).expect("the program directory must pass the program check");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn adr_0048_an_install_adds_the_read_entry_to_an_existing_tree() {
    // An older build made the program tree for SYSTEM and Administrators only.
    let base = temp("program-old");
    std::fs::create_dir_all(&base).expect("cannot create the base");
    let root = base.join("ResticHub");
    let program = root.join("Agent");
    create_protected_tree(&program, &root).expect("cannot create the tree");
    let file = program.join("restic.exe");
    create_protected_file(&file, b"x").expect("cannot write the file");
    for path in [&root, &program, &file] {
        assert!(
            !grants_users_read(path),
            "{} must hold no read",
            path.display()
        );
        assert!(
            add_user_read(path).expect("cannot add the read access"),
            "{} must take the read access",
            path.display()
        );
        assert!(
            !add_user_read(path).expect("cannot check the read access"),
            "a second call must change nothing"
        );
        check_admin_only(path).expect("the entry must stay protected");
        assert!(
            grants_users_read(path),
            "{} must grant a read",
            path.display()
        );
    }
    let _ = std::fs::remove_dir_all(&base);
}

fn grants_users_read(path: &Path) -> bool {
    Entry::open(path)
        .expect("cannot open")
        .expect("the entry must exist")
        .grants_users_read()
        .expect("cannot read the access control")
}

/// Reads the security identifier of the account that runs the process.
pub(crate) fn current_user_sid() -> Result<String> {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
    let whoami = Path::new(&system_root).join("System32").join("whoami.exe");
    let output = std::process::Command::new(whoami)
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .context("cannot run whoami")?;
    let text = String::from_utf8_lossy(&output.stdout);
    // The line is `"domain\account","S-1-5-21-..."`.
    let sid = text
        .rsplit("\",\"")
        .next()
        .unwrap_or_default()
        .trim_matches(|character| character == '"' || character == '\r' || character == '\n')
        .to_owned();
    if !sid.starts_with("S-1-") {
        bail!("cannot read the user SID from `{text}`");
    }
    Ok(sid)
}

/// Sets the owner of `path` to the SDDL identity, for a negative test.
pub(crate) fn set_owner(path: &Path, sid: &str) -> Result<()> {
    let descriptor = descriptor(&format!("O:{sid}"))?;
    let mut owner: PSID = std::ptr::null_mut();
    let mut defaulted = 0;
    // SAFETY: the descriptor is a valid security descriptor.
    unsafe { GetSecurityDescriptorOwner(descriptor.0, &mut owner, &mut defaulted) };
    let wide = wide_path(path);
    // SAFETY: the path is a null-terminated UTF-16 string and the owner stays alive.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            owner,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if status != ERROR_SUCCESS {
        bail!("cannot set the owner (error {status})");
    }
    Ok(())
}

/// Grants the account of BUILTIN\Users the given rights, for a negative test.
pub(crate) fn grant(dir: &Path, rights: &str) -> Result<()> {
    grant_sid(dir, "S-1-5-32-545", rights)
}

pub(crate) fn grant_sid(path: &Path, sid: &str, rights: &str) -> Result<()> {
    run_icacls(path, &["/grant", &format!("*{sid}:{rights}")])
}

/// Copies the DACL of the real `C:\Program Files` onto `dir`, for a positive test.
fn copy_program_files_dacl(dir: &Path) -> Result<()> {
    let source = open_security(Path::new(r"C:\Program Files"))?;
    let owner = sid_from_string("S-1-5-32-544").context("cannot build the owner SID")?;
    let wide = wide_path(dir);
    let info = OWNER_SECURITY_INFORMATION
        | DACL_SECURITY_INFORMATION
        | PROTECTED_DACL_SECURITY_INFORMATION;
    // SAFETY: the path is a null-terminated UTF-16 string and the DACL stays alive.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            info,
            owner.0,
            std::ptr::null_mut(),
            source.dacl,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        bail!("cannot copy the DACL (error {status})");
    }
    Ok(())
}

/// Runs `%SystemRoot%\System32\icacls.exe` by absolute path.
fn run_icacls(dir: &Path, arguments: &[&str]) -> Result<()> {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
    let icacls = Path::new(&system_root).join("System32").join("icacls.exe");
    let status = std::process::Command::new(icacls)
        .arg(dir)
        .args(arguments)
        .status()
        .context("cannot run icacls")?;
    if !status.success() {
        bail!("icacls failed with {status}");
    }
    Ok(())
}
