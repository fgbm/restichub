#![allow(clippy::expect_used)]

use std::path::Path;

use super::super::RotatingFile;
use super::{names, read, temp_dir};

#[test]
fn a_full_file_starts_the_next_one() {
    let dir = temp_dir("size");
    let mut file = RotatingFile::open(&dir, 10, 5).expect("cannot open the log");
    file.write(b"1234\n").expect("cannot write");
    file.write(b"5678\n").expect("cannot write");
    file.write(b"abcd\n").expect("cannot write");
    assert_eq!(names(&dir), ["agent-000001.log", "agent-000002.log"]);
    assert_eq!(read(&dir, "agent-000001.log"), "1234\n5678\n");
    assert_eq!(read(&dir, "agent-000002.log"), "abcd\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_event_larger_than_a_file_stays_whole() {
    let dir = temp_dir("large");
    let mut file = RotatingFile::open(&dir, 4, 5).expect("cannot open the log");
    file.write(b"0123456789\n").expect("cannot write");
    assert_eq!(names(&dir), ["agent-000001.log"]);
    assert_eq!(read(&dir, "agent-000001.log"), "0123456789\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_oldest_files_above_the_count_are_removed() {
    let dir = temp_dir("count");
    std::fs::write(dir.join("notes.txt"), b"other").expect("cannot write");
    let mut file = RotatingFile::open(&dir, 5, 3).expect("cannot open the log");
    for line in ["one\n", "two\n", "three\n", "four\n", "five\n"] {
        file.write(line.as_bytes()).expect("cannot write");
    }
    assert_eq!(
        names(&dir),
        [
            "agent-000003.log",
            "agent-000004.log",
            "agent-000005.log",
            "notes.txt"
        ]
    );
    assert_eq!(read(&dir, "agent-000005.log"), "five\n");
    assert_eq!(file.current(), dir.join("agent-000005.log"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_restart_continues_the_newest_file_until_it_is_full() {
    let dir = temp_dir("restart");
    std::fs::write(dir.join("agent-000009.log"), b"old\n").expect("cannot write");
    std::fs::write(dir.join("agent-000010.log"), b"new\n").expect("cannot write");
    let mut file = RotatingFile::open(&dir, 9, 5).expect("cannot open the log");
    file.write(b"more\n").expect("cannot write");
    assert_eq!(read(&dir, "agent-000010.log"), "new\nmore\n");
    drop(file);
    let mut file = RotatingFile::open(&dir, 9, 5).expect("cannot open the log");
    file.write(b"next\n").expect("cannot write");
    assert_eq!(read(&dir, "agent-000011.log"), "next\n");
    assert_eq!(
        names(&dir),
        ["agent-000009.log", "agent-000010.log", "agent-000011.log"]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_name_that_is_not_a_log_file_is_left_alone() {
    let dir = temp_dir("names");
    for name in [
        "agent-.log",
        "agent-12a.log",
        "agent-000001.txt",
        "other-000001.log",
    ] {
        std::fs::write(dir.join(name), b"x").expect("cannot write");
    }
    let file = RotatingFile::open(&dir, 1, 1).expect("cannot open the log");
    assert_eq!(file.current(), dir.join("agent-000001.log"));
    assert_eq!(names(&dir).len(), 5);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
fn link_file(link: &Path, target: &Path) {
    std::os::unix::fs::symlink(target, link).expect("cannot create the link");
}

#[cfg(unix)]
fn link_dir(link: &Path, target: &Path) {
    std::os::unix::fs::symlink(target, link).expect("cannot create the link");
}

#[cfg(windows)]
fn link_file(link: &Path, target: &Path) {
    std::os::windows::fs::symlink_file(target, link).expect("cannot create the link");
}

#[cfg(windows)]
fn link_dir(link: &Path, target: &Path) {
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
fn fr_ag_04_the_rotation_never_opens_or_removes_a_link_or_a_directory_with_a_log_name() {
    let dir = temp_dir("links");
    let outside = temp_dir("links-outside");
    let target = outside.join("target.txt");
    std::fs::write(&target, b"keep\n").expect("cannot write");
    link_file(&dir.join("agent-000001.log"), &target);
    link_dir(&dir.join("agent-000002.log"), &outside);
    std::fs::create_dir(dir.join("agent-000003.log")).expect("cannot create the directory");
    std::fs::write(dir.join("agent-000004.log"), b"old\n").expect("cannot write");
    link_file(&dir.join("agent-000009.log"), &target);

    let mut file = RotatingFile::open(&dir, 8, 1).expect("cannot open the log");
    assert_eq!(file.current(), dir.join("agent-000004.log"));
    file.write(b"more\n").expect("cannot write");
    file.write(b"next\n").expect("cannot write");

    assert_eq!(file.current(), dir.join("agent-000006.log"));
    assert_eq!(
        names(&dir),
        [
            "agent-000001.log",
            "agent-000002.log",
            "agent-000003.log",
            "agent-000006.log",
            "agent-000009.log"
        ]
    );
    assert_eq!(
        std::fs::read_to_string(&target).expect("the target must stay"),
        "keep\n"
    );
    assert_eq!(names(&outside), ["target.txt"]);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn fr_ag_04_the_service_log_directory_leaves_an_interrupted_identity_swap_recoverable() {
    let base = temp_dir("recover");
    let config = base.join("Agent");
    let backup = crate::identity::sibling(&config, ".old");
    std::fs::create_dir(&backup).expect("cannot create the backup");
    std::fs::write(backup.join(crate::identity::METADATA_FILE), b"{}").expect("cannot write");

    let logs = crate::logging::service_log_dir(&config);
    assert_eq!(logs, base.join("Agent-logs"));
    crate::paths::create_protected_dir(&logs).expect("cannot create the log directory");
    assert!(
        !config.exists(),
        "the log directory must not create the identity directory"
    );

    assert_eq!(crate::identity::recover(&config), config);
    assert!(config.join(crate::identity::METADATA_FILE).exists());
    assert!(logs.exists());
    let _ = std::fs::remove_dir_all(&base);
}
