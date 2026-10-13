//! Scenario tests that run the built `restichub-fake-restic` binary.
//!
//! The test names refer to `design/acceptance-criteria.md` where the test
//! supports a criterion.
#![allow(clippy::expect_used)]

mod common;

use std::time::{Duration, Instant};

use common::{has_lock, json_lines, repo_arg, run, setup, stderr, stdout, wait_until};

const BACKUP: &str = r#"{"command_sequence": {"backup": [
    {"duration_ms": 30000, "progress_rate": 10, "lock": "manage"},
    {"lock": "manage"}
]}}"#;

#[test]
fn the_second_call_uses_the_sequence() {
    let (scenario, repo) = setup(
        "sequence",
        r#"{"command_sequence": {"backup": [{"exit_code": 0}, {"exit_code": 1}]}}"#,
    );
    let repo = repo_arg(&repo);
    let first = run(&scenario, &["backup", "--json", &repo, "/data"]);
    assert_eq!(first.status.code(), Some(0), "the first call succeeds");
    let second = run(&scenario, &["backup", "--json", &repo, "/data"]);
    assert_eq!(second.status.code(), Some(1), "the second call fails");
}

#[test]
fn ac_sc_05_five_locked_runs_from_the_sequence() {
    let (scenario, repo) = setup(
        "five-locks",
        r#"{"command_sequence": {"forget": [
            {"exit_code": 11}, {"exit_code": 11}, {"exit_code": 11},
            {"exit_code": 11}, {"exit_code": 11}, {"exit_code": 0}
        ]}}"#,
    );
    let repo = repo_arg(&repo);
    for expected in [11, 11, 11, 11, 11, 0] {
        let output = run(&scenario, &["forget", "--json", &repo, "--keep-last=1"]);
        assert_eq!(output.status.code(), Some(expected), "exit code {expected}");
    }
}

#[test]
fn fail_at_percent_stops_the_backup_before_the_summary() {
    let (scenario, repo) = setup(
        "fail-percent",
        r#"{"commands": {"backup": {"progress_count": 10, "fail_at_percent": 50, "exit_code": 1}}}"#,
    );
    let output = run(&scenario, &["backup", "--json", &repo_arg(&repo), "/data"]);
    assert_eq!(output.status.code(), Some(1));
    let messages = json_lines(&stdout(&output));
    assert!(messages.iter().any(|m| m["message_type"] == "status"));
    assert!(
        !messages.iter().any(|m| m["message_type"] == "summary"),
        "a failure at a percent writes no summary"
    );
}

#[test]
fn a_partial_output_has_no_summary() {
    let (scenario, repo) = setup(
        "partial",
        r#"{"commands": {"backup": {"progress_count": 3, "partial_output": true}}}"#,
    );
    let output = run(&scenario, &["backup", "--json", &repo_arg(&repo), "/data"]);
    assert_eq!(output.status.code(), Some(1));
    let messages = json_lines(&stdout(&output));
    assert_eq!(messages.len(), 3, "three status messages and no summary");
    assert!(messages.iter().all(|m| m["message_type"] == "status"));
}

#[test]
fn the_scenario_writes_stderr_lines() {
    let (scenario, repo) = setup(
        "stderr",
        r#"{"commands": {"backup": {"stderr_lines": ["warning: a source file was unreadable"]}}}"#,
    );
    let output = run(&scenario, &["backup", "--json", &repo_arg(&repo), "/data"]);
    assert!(output.status.success());
    assert!(stderr(&output).contains("warning: a source file was unreadable"));
}

#[test]
fn a_duration_adds_a_wait() {
    let (scenario, repo) = setup(
        "duration",
        r#"{"commands": {"backup": {"duration_ms": 1200}}}"#,
    );
    let start = Instant::now();
    let output = run(&scenario, &["backup", "--json", &repo_arg(&repo), "/data"]);
    assert!(output.status.success());
    assert!(
        start.elapsed() >= Duration::from_millis(1000),
        "the duration must add a wait: {:?}",
        start.elapsed()
    );
}

#[test]
fn a_stale_lock_needs_an_unlock_before_the_next_backup() {
    let (scenario, repo) = setup(
        "stale",
        r#"{"command_sequence": {"backup": [
            {"lock": "stale"}, {"lock": "manage"}, {"lock": "manage"}
        ]}}"#,
    );
    let repo_flag = repo_arg(&repo);

    let stale = run(&scenario, &["backup", "--json", &repo_flag, "/data"]);
    assert_eq!(stale.status.code(), Some(11), "a stale lock exits with 11");
    assert!(stderr(&stale).contains("already locked"));
    assert!(has_lock(&repo));

    let locked = run(&scenario, &["backup", "--json", &repo_flag, "/data"]);
    assert_eq!(locked.status.code(), Some(11), "the lock still blocks");

    let unlock = run(&scenario, &["unlock", &repo_flag]);
    assert_eq!(unlock.status.code(), Some(0));

    let clean = run(&scenario, &["backup", "--json", &repo_flag, "/data"]);
    assert_eq!(
        clean.status.code(),
        Some(0),
        "the backup succeeds after unlock"
    );
}

#[test]
fn ac_sc_06_unlock_clears_a_lock_left_by_a_hard_kill() {
    let (scenario, repo) = setup("kill", BACKUP);
    let dir = scenario
        .parent()
        .expect("the scenario directory")
        .to_path_buf();
    let stdout_path = dir.join("stdout.log");
    let stderr_path = dir.join("stderr.log");

    let mut command = common::base_command(&scenario);
    command.args(["backup", "--json", &repo_arg(&repo), "/data"]);
    command.stdout(std::process::Stdio::from(
        std::fs::File::create(&stdout_path).expect("the stdout file"),
    ));
    command.stderr(std::process::Stdio::from(
        std::fs::File::create(&stderr_path).expect("the stderr file"),
    ));
    let mut child = command.spawn().expect("spawn the fake restic");

    assert!(
        wait_until(|| has_lock(&repo), Duration::from_secs(10)),
        "the backup must take the lock"
    );
    child.kill().expect("kill the backup");
    child.wait().expect("wait for the backup");
    assert!(has_lock(&repo), "a hard kill leaves the lock (AC-SC-06)");

    let unlock = run(&scenario, &["unlock", &repo_arg(&repo)]);
    assert_eq!(unlock.status.code(), Some(0));
    assert!(!has_lock(&repo));

    let clean = run(&scenario, &["backup", "--json", &repo_arg(&repo), "/data"]);
    assert_eq!(clean.status.code(), Some(0));
}
