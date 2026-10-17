//! journald holds no canary after the jobs of the agent (AC-AG-03).
//!
//! The service writes its `tracing` log to the standard error, which journald stores. This test
//! sends the log of the agent in this process to journald through `systemd-cat`, at every level,
//! runs ten jobs on three repositories whose password is the canary, and reads the journal back.
//! The install smoke workflow scans journald and the Windows Event Log of the installed service.

#![cfg(target_os = "linux")]
#![allow(clippy::expect_used)]

mod common;

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::*;
use restichub_agent::jobs::Jobs;
use restichub_canary_scan::{Canary, Scanner};
use restichub_proto::agent::v1::JobStatus;

const SUMMARY: &str = r#"{"message_type":"summary","files_new":4,"files_changed":0,"files_unmodified":0,"dirs_new":1,"dirs_changed":0,"dirs_unmodified":0,"data_blobs":4,"tree_blobs":1,"data_added":4096,"data_added_packed":2048,"total_files_processed":4,"total_bytes_processed":8000,"total_duration":0.5,"snapshot_id":"feedface"}"#;

/// The ubuntu job of CI sets this, so a host without a usable journal fails there.
fn journald_required() -> bool {
    std::env::var_os("RESTICHUB_REQUIRE_JOURNALD").is_some()
}

fn skip_or_panic(reason: &str) {
    assert!(
        !journald_required(),
        "RESTICHUB_REQUIRE_JOURNALD is set: {reason}"
    );
    println!("skipping: {reason}");
}

fn directory(name: &str) -> PathBuf {
    let dir = temp_config_dir(name);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    dir
}

/// The whole journal since `since`, with every field of every entry. `sudo -n` reads the entries of
/// all units where the runner allows it; without it the journal of the user is the scope.
fn journal_since(since: u64) -> Vec<u8> {
    let arguments = [
        "--no-pager".to_owned(),
        "--output=export".to_owned(),
        format!("--since=@{since}"),
    ];
    let privileged = Command::new("sudo")
        .arg("-n")
        .arg("journalctl")
        .args(&arguments)
        .stderr(Stdio::null())
        .output();
    if let Ok(output) = privileged
        && output.status.success()
    {
        return output.stdout;
    }
    Command::new("journalctl")
        .args(&arguments)
        .stderr(Stdio::null())
        .output()
        .map(|output| output.stdout)
        .unwrap_or_default()
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// journald stores a line some time after `systemd-cat` reads it.
fn wait_for_journal(since: u64, needle: &str) -> Option<Vec<u8>> {
    for _ in 0..60 {
        let journal = journal_since(since);
        if contains(&journal, needle) {
            return Some(journal);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    None
}

fn start_systemd_cat(tag: &str) -> Option<Child> {
    Command::new("systemd-cat")
        .args(["--identifier", tag])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()
}

/// AC-AG-03 (Linux): after ten jobs on three repositories, journald holds no canary.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_03_linux_ten_jobs_leave_no_canary_in_journald() {
    let tag = format!("restichub-canary-{}", uuid::Uuid::now_v7());
    let Some(mut cat) = start_systemd_cat(&tag) else {
        skip_or_panic("systemd-cat does not start");
        return;
    };
    let stdin = cat.stdin.take().expect("the systemd-cat input");
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(Mutex::new(stdin))
        .try_init()
        .expect("the journald log must be the only subscriber");
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after 1970")
        .as_secs()
        - 2;

    // A control value proves that the journal read back holds the lines of this process.
    let control = format!("CANARY-{}", uuid::Uuid::now_v7());
    tracing::info!(%control, "journald control line");
    let Some(journal) = wait_for_journal(since, &control) else {
        skip_or_panic("journald does not give back the lines of this process");
        return;
    };
    let control_scanner = Scanner::new(&[Canary::parse(&control)]);
    assert!(
        !control_scanner.scan("journald", &journal).is_empty(),
        "the scan must find the control value"
    );

    let canary = format!("CANARY-{}", uuid::Uuid::now_v7());
    let dir = directory("journald-canary-agent");
    let refused_dir = directory("journald-canary-refused-agent");
    let paths: Vec<PathBuf> = (1..=3)
        .map(|index| directory(&format!("journald-canary-repo-{index}")))
        .collect();
    for path in &paths {
        script_stdout(path, "backup", &format!("{SUMMARY}\n"));
        script_stderr(path, "backup", "Fatal: {PASSWORD} is {PASSWORD}\n");
    }
    let jobs = program_jobs(&dir);
    // A binary with another hash makes the agent log a warning for each job (FR-AG-11).
    let refused = Jobs::open(
        fake_restic(),
        "0".repeat(64),
        refused_dir.join("cache"),
        &refused_dir,
    )
    .expect("the job journal must open")
    .with_restore_access(temp_roots())
    .with_repository_access(restichub_agent::repository_roots::RepositoryAccess::unrestricted());
    for index in 0..10 {
        let repository = format!("repo-{}", index % 3 + 1);
        let job = job(
            &format!("job-journald-{index}"),
            vec![access(&repository, &paths[index % 3], &canary)],
            vec![backup_step(&repository, &["/srv"])],
        );
        if index % 3 == 2 {
            let collected = run(&refused, &job).await;
            assert_ne!(collected.result().outcome, JobStatus::Succeeded as i32);
        } else {
            let collected = run(&jobs, &job).await;
            assert_eq!(collected.result().outcome, JobStatus::Succeeded as i32);
        }
    }

    let end = format!("CANARY-{}", uuid::Uuid::now_v7());
    tracing::info!(%end, "journald end line");
    let journal = wait_for_journal(since, &end).expect("journald must store the end line");
    assert!(
        contains(&journal, "cannot start restic"),
        "the journal must hold the warnings of the agent"
    );
    let findings = Scanner::new(&[Canary::parse(&canary)]).scan("journald", &journal);
    assert!(
        findings.is_empty(),
        "the canary is in journald: {findings:?}"
    );

    let _ = cat.kill();
    let _ = cat.wait();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&refused_dir);
    for path in &paths {
        let _ = std::fs::remove_dir_all(path);
    }
}
