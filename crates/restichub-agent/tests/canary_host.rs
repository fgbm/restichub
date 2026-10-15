//! The canary checks of the host that the agent runs on: the temporary directory, the registry and
//! the process arguments (AC-AG-03, AC-AG-04, NFR-SEC-04, NFR-SEC-05).
//!
//! `step_journal.rs` scans the agent directory and the Linux process arguments. The tests in this
//! file also run on Windows, where the CI job `canary-windows` starts them with the real
//! `windows-latest` runner and `RESTICHUB_HOST_CANARY` set.

#![allow(clippy::expect_used)]

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::*;
use restichub_canary_scan::{Canary, Scanner};
use restichub_proto::agent::v1::JobStatus;

const SUMMARY: &str = r#"{"message_type":"summary","files_new":4,"files_changed":0,"files_unmodified":0,"dirs_new":1,"dirs_changed":0,"dirs_unmodified":0,"data_blobs":4,"tree_blobs":1,"data_added":4096,"data_added_packed":2048,"total_files_processed":4,"total_bytes_processed":8000,"total_duration":0.5,"snapshot_id":"feedface"}"#;

fn directory(name: &str) -> PathBuf {
    let dir = temp_config_dir(name);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    dir
}

/// Scans every readable file under `root` that changed since `since`. A host directory holds big
/// files of other programs that are locked or private, so the scan skips an unreadable file and
/// an older one: only the jobs of the test can have written the canary.
fn scan_tree(scanner: &Scanner, root: &Path, since: SystemTime, scanned: &mut usize) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut findings = 0;
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            findings += scan_tree(scanner, &entry.path(), since, scanned);
        } else if file_type.is_file()
            && entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .is_ok_and(|modified| modified >= since)
            && let Ok(found) = scanner.scan_file(&entry.path())
        {
            *scanned += 1;
            findings += found.len();
        }
    }
    findings
}

/// AC-AG-03: after ten jobs the canary is not in the temporary directory of the host.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_03_ten_jobs_leave_no_canary_in_the_temporary_directory() {
    let canary = format!("CANARY-{}", uuid::Uuid::now_v7());
    let scanner = Scanner::new(&[Canary::parse(&canary)]);
    let dir = directory("host-canary-agent");
    let path = directory("host-canary-repo");
    script_stdout(&path, "backup", &format!("{SUMMARY}\n"));
    script_stderr(&path, "backup", "Fatal: {PASSWORD} is {PASSWORD}\n");
    let jobs = program_jobs(&dir);
    let since = SystemTime::now() - Duration::from_secs(2);
    for index in 0..10 {
        let job = job(
            &format!("job-host-{index}"),
            vec![access("repo-1", &path, &canary)],
            vec![backup_step("repo-1", &["/srv"])],
        );
        let collected = run(&jobs, &job).await;
        assert_eq!(collected.result().outcome, JobStatus::Succeeded as i32);
    }
    let mut scanned = 0;
    let findings = scan_tree(&scanner, &std::env::temp_dir(), since, &mut scanned);
    assert!(scanned > 0, "the scan read no file");
    assert_eq!(findings, 0, "the canary is in the temporary directory");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// The registry dump and the process listing are slow and read the whole host, so only the CI job
/// `canary-windows` runs them (`RESTICHUB_HOST_CANARY`).
#[cfg(windows)]
fn host_scan_enabled() -> bool {
    let enabled = std::env::var_os("RESTICHUB_HOST_CANARY").is_some();
    if !enabled {
        println!("skipping: RESTICHUB_HOST_CANARY is not set");
    }
    enabled
}

#[cfg(windows)]
fn command_output(program: &str, arguments: &[&str]) -> Vec<u8> {
    let output = std::process::Command::new(program)
        .args(arguments)
        .output()
        .expect("cannot start the host tool");
    let mut text = output.stdout;
    text.extend_from_slice(&output.stderr);
    text
}

/// Reports whether `reg query` finds `needle` in a name or in data below `hive`. The tool searches
/// inside the process, so the registry is not dumped through a pipe.
#[cfg(windows)]
fn registry_search(hive: &str, needle: &str) -> bool {
    std::process::Command::new("reg")
        .args(["query", hive, "/s", "/f", needle, "/c"])
        .output()
        .expect("cannot start reg")
        .status
        .success()
}

/// AC-AG-03 (Windows): after ten jobs the canary is not in the registry.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_03_windows_ten_jobs_leave_no_canary_in_the_registry() {
    if !host_scan_enabled() {
        return;
    }
    let canary = format!("CANARY-{}", uuid::Uuid::now_v7());
    let dir = directory("registry-canary-agent");
    let path = directory("registry-canary-repo");
    script_stdout(&path, "backup", &format!("{SUMMARY}\n"));
    let jobs = program_jobs(&dir);
    for index in 0..10 {
        let job = job(
            &format!("job-registry-{index}"),
            vec![access("repo-1", &path, &canary)],
            vec![backup_step("repo-1", &["/srv"])],
        );
        let collected = run(&jobs, &job).await;
        assert_eq!(collected.result().outcome, JobStatus::Succeeded as i32);
    }
    // A control value proves that the search finds a canary in the registry.
    let control = format!("CANARY-{}", uuid::Uuid::now_v7());
    let key = "HKCU\\Software\\RestichubCanaryControl";
    let added = std::process::Command::new("reg")
        .args(["add", key, "/v", "control", "/d", &control, "/f"])
        .status()
        .expect("cannot start reg");
    assert!(added.success(), "cannot write the control value");
    let found = registry_search("HKCU\\Software", &control);
    let _ = std::process::Command::new("reg")
        .args(["delete", key, "/f"])
        .status();
    assert!(found, "the search must find the control value");
    for hive in [
        "HKCU",
        "HKLM\\SOFTWARE",
        "HKLM\\SYSTEM\\CurrentControlSet\\Services",
    ] {
        assert!(
            !registry_search(hive, &canary),
            "{hive}: the canary is in the registry"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// AC-AG-04 (Windows): while a backup runs, no process command line holds the canary. The query is
/// the CIM class `Win32_Process`, which lists the command line of every process.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_04_windows_no_process_argument_holds_the_canary_during_a_backup() {
    if !host_scan_enabled() {
        return;
    }
    let canary = format!("CANARY-{}", uuid::Uuid::now_v7());
    let scanner = Scanner::new(&[Canary::parse(&canary)]);
    let dir = directory("cim-agent");
    let path = directory("cim-repo");
    script_sleep(&path, 8_000);
    let jobs = program_jobs(&dir);
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    jobs.attach(&tx);
    let job = job(
        "job-cim",
        vec![access("repo-1", &path, &canary)],
        vec![backup_step("repo-1", &["/srv"])],
    );
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;

    assert!(
        wait_for_run(&path, "backup").await,
        "the backup must be running"
    );
    let listing = command_output(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_Process | ForEach-Object { $_.CommandLine }",
        ],
    );
    let program = fake_restic();
    let name = program
        .file_stem()
        .and_then(|name| name.to_str())
        .expect("the program name");
    assert!(
        String::from_utf8_lossy(&listing).contains(name),
        "the query must list the process of the backup"
    );
    let findings = scanner.scan("Win32_Process", &listing);
    assert!(findings.is_empty(), "a process argument holds the canary");
    let finished = collect(&mut rx).await;
    assert_eq!(finished.result().outcome, JobStatus::Succeeded as i32);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}
