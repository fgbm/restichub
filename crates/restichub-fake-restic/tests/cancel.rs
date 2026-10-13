//! Cancel tests that run the built `restichub-fake-restic` binary as the agent
//! starts it: in its own process group, with a cancel signal from the agent
//! (FR-AG-10, AC-AG-06, AC-AG-07).
#![allow(clippy::expect_used)]
// The test plays the agent: it sends SIGINT or CTRL_BREAK_EVENT with the
// platform layer. Every unsafe call carries its own safety note.
#![allow(unsafe_code)]

mod common;

use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use common::{base_command, has_lock, setup, wait_until};

#[cfg(unix)]
fn put_in_own_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn put_in_own_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP);
}

#[cfg(unix)]
fn send_cancel(child: &Child) {
    // SAFETY: the child is a group leader from `process_group(0)`, so the
    // negative pid names its group. `kill` reads only its arguments.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGINT);
    }
}

#[cfg(windows)]
fn send_cancel(child: &Child) {
    // SAFETY: mechanism B of spike P1. `AllocConsole` gives the test a console
    // when it has none and fails when it has one; either way the child shares
    // it. `GenerateConsoleCtrlEvent` reads only its arguments.
    unsafe {
        windows_sys::Win32::System::Console::AllocConsole();
        windows_sys::Win32::System::Console::GenerateConsoleCtrlEvent(
            windows_sys::Win32::System::Console::CTRL_BREAK_EVENT,
            child.id(),
        );
    }
}

fn spawn(scenario: &Path, repo: &Path, log_dir: &Path) -> (Child, std::path::PathBuf) {
    let stderr_path = log_dir.join("cancel-stderr.log");
    let mut command = base_command(scenario);
    command.args(["backup", "--json", &common::repo_arg(repo), "/data"]);
    command.stdout(Stdio::from(
        std::fs::File::create(log_dir.join("cancel-stdout.log")).expect("the stdout file"),
    ));
    command.stderr(Stdio::from(
        std::fs::File::create(&stderr_path).expect("the stderr file"),
    ));
    put_in_own_group(&mut command);
    (command.spawn().expect("spawn the fake restic"), stderr_path)
}

/// Waits for the exit, at most for the limit. A limit keeps a stuck signal
/// path from hanging the test job.
fn wait_with_limit(child: &mut Child, limit: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    while start.elapsed() < limit {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

#[test]
fn ac_ag_06_a_cancel_stops_the_backup_and_removes_the_lock() {
    let (scenario, repo) = setup(
        "cancel",
        r#"{"commands": {"backup": {
            "duration_ms": 15000, "progress_rate": 20, "lock": "manage",
            "cancel": {"delay_ms": 500}
        }}}"#,
    );
    let log_dir = scenario
        .parent()
        .expect("the scenario directory")
        .to_path_buf();
    let (mut child, stderr_path) = spawn(&scenario, &repo, &log_dir);

    assert!(
        wait_until(|| has_lock(&repo), Duration::from_secs(10)),
        "the backup must run and take the lock"
    );
    let sent = Instant::now();
    send_cancel(&child);
    let status = wait_with_limit(&mut child, Duration::from_secs(15));
    let elapsed = sent.elapsed();
    let Some(status) = status else {
        let _ = child.kill();
        let _ = child.wait();
        panic!("the cancel did not stop the process within 15 s");
    };

    assert_eq!(
        status.code(),
        Some(130),
        "the cancel exits with code 130 (AC-AG-06)"
    );
    assert!(
        elapsed >= Duration::from_millis(400),
        "the configured delay must apply: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "the exit must stay inside the grace period: {elapsed:?}"
    );
    let text = std::fs::read_to_string(&stderr_path).expect("read stderr");
    assert!(text.contains("cleaning up"), "stderr: {text}");
    assert!(
        !has_lock(&repo),
        "a graceful cancel leaves no lock (AC-AG-06)"
    );
}

#[test]
fn ac_ag_07_an_ignored_signal_leaves_a_stale_lock() {
    let (scenario, repo) = setup(
        "ignore",
        r#"{"commands": {"backup": {
            "duration_ms": 30000, "progress_rate": 20, "lock": "manage",
            "cancel": {"ignore": true}
        }}}"#,
    );
    let log_dir = scenario
        .parent()
        .expect("the scenario directory")
        .to_path_buf();
    let (mut child, _) = spawn(&scenario, &repo, &log_dir);

    assert!(
        wait_until(|| has_lock(&repo), Duration::from_secs(10)),
        "the backup must run and take the lock"
    );
    send_cancel(&child);

    // The agent waits for its grace period and then kills the process tree.
    std::thread::sleep(Duration::from_millis(1000));
    assert!(
        child.try_wait().expect("try_wait").is_none(),
        "an ignored cancel must not stop the process (AC-AG-07)"
    );
    child.kill().expect("kill the process tree");
    child.wait().expect("wait for the child");
    assert!(
        has_lock(&repo),
        "a hard kill after an ignored signal leaves the lock (AC-AG-07)"
    );
}
