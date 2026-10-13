//! CLI tests that run the built `restichub-fake-restic` binary.
//!
//! The test names refer to `design/acceptance-criteria.md` where the test
//! supports a criterion.

#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

const PASSWORD: &str = "test-password";
const CANARY: &str = "CANARY-7f3a9c2e";

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_restichub-fake-restic")
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn golden(name: &str) -> String {
    std::fs::read_to_string(golden_dir().join(name)).expect("read the golden file")
}

fn temp_path(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "restichub-fake-restic-{}-{nanos}-{name}",
        std::process::id()
    ));
    path
}

fn run(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(binary());
    command.args(args);
    for name in [
        "RESTIC_PASSWORD",
        "RESTIC_PASSWORD_FILE",
        "RESTIC_PASSWORD_COMMAND",
        "FAKE_RESTIC_EXIT",
        "FAKE_RESTIC_VERSION",
        "FAKE_RESTIC_PROGRESS_RATE",
        "FAKE_RESTIC_PROGRESS_COUNT",
        "FAKE_RESTIC_SCENARIO",
        "FAKE_RESTIC_ARGV_FILE",
    ] {
        command.env_remove(name);
    }
    for (name, value) in env {
        command.env(name, value);
    }
    command.output().expect("run the binary")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}

fn parse_lines(text: &str) -> Vec<Value> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("a JSON line"))
        .collect()
}

fn assert_json_lines(actual: &str, expected: &str) {
    assert_eq!(parse_lines(actual), parse_lines(expected));
}

fn assert_single_json(actual: &str, expected: &str) {
    let actual: Value = serde_json::from_str(actual.trim()).expect("actual JSON");
    let expected: Value = serde_json::from_str(expected.trim()).expect("golden JSON");
    assert_eq!(actual, expected);
}

#[test]
fn golden_backup_json() {
    let output = run(
        &["backup", "--json", "--repo=/srv/restic-repo", "/data"],
        &[
            ("RESTIC_PASSWORD", PASSWORD),
            ("FAKE_RESTIC_PROGRESS_COUNT", "3"),
            ("FAKE_RESTIC_PROGRESS_RATE", "1"),
        ],
    );
    assert!(output.status.success(), "exit: {:?}", output.status);
    assert_json_lines(&stdout(&output), &golden("backup.jsonl"));
}

#[test]
fn golden_snapshots_json() {
    let output = run(
        &["snapshots", "--json", "--repo=/srv/restic-repo"],
        &[("RESTIC_PASSWORD", PASSWORD)],
    );
    assert!(output.status.success(), "exit: {:?}", output.status);
    assert_single_json(&stdout(&output), &golden("snapshots.json"));
}

#[test]
fn golden_forget_json() {
    let output = run(
        &[
            "forget",
            "--json",
            "--repo=/srv/restic-repo",
            "--keep-last=7",
        ],
        &[("RESTIC_PASSWORD", PASSWORD)],
    );
    assert!(output.status.success(), "exit: {:?}", output.status);
    assert_single_json(&stdout(&output), &golden("forget.json"));
}

#[test]
fn golden_check_json() {
    let output = run(
        &["check", "--json", "--repo=/srv/restic-repo"],
        &[("RESTIC_PASSWORD", PASSWORD)],
    );
    assert!(output.status.success(), "exit: {:?}", output.status);
    assert_single_json(&stdout(&output), &golden("check.json"));
}

#[test]
fn golden_restore_json() {
    let output = run(
        &[
            "restore",
            "--json",
            "--repo=/srv/restic-repo",
            "--target=/tmp/out",
            "latest",
        ],
        &[("RESTIC_PASSWORD", PASSWORD)],
    );
    assert!(output.status.success(), "exit: {:?}", output.status);
    assert_json_lines(&stdout(&output), &golden("restore.jsonl"));
}

#[test]
fn golden_init_json() {
    let output = run(
        &["init", "--json", "--repo=/srv/restic-repo"],
        &[("RESTIC_PASSWORD", PASSWORD)],
    );
    assert!(output.status.success(), "exit: {:?}", output.status);
    assert_single_json(&stdout(&output), &golden("init.json"));
}

#[test]
fn golden_cat_config_json() {
    let output = run(
        &["cat", "config", "--repo=/srv/restic-repo"],
        &[("RESTIC_PASSWORD", PASSWORD)],
    );
    assert!(output.status.success(), "exit: {:?}", output.status);
    assert_single_json(&stdout(&output), &golden("cat_config.json"));
}

#[test]
fn ac_ag_05_exit_codes() {
    for code in [0, 1, 3, 10, 11, 12, 130, 99] {
        let text = code.to_string();
        let output = run(
            &["backup", "--json", "--repo=/x", "/data"],
            &[("RESTIC_PASSWORD", PASSWORD), ("FAKE_RESTIC_EXIT", &text)],
        );
        assert_eq!(output.status.code(), Some(code), "exit code {code}");
        if code != 0 {
            let error: Value =
                serde_json::from_str(stderr(&output).trim()).expect("an exit_error message");
            assert_eq!(error["message_type"], "exit_error", "code {code}");
            assert_eq!(error["code"], code, "code {code}");
        }
    }
}

#[test]
fn ac_sc_05_retry_exit_code_11() {
    let output = run(
        &["forget", "--json", "--repo=/x", "--keep-last=1"],
        &[("RESTIC_PASSWORD", PASSWORD), ("FAKE_RESTIC_EXIT", "11")],
    );
    assert_eq!(output.status.code(), Some(11));
    let error: Value = serde_json::from_str(stderr(&output).trim()).expect("an exit_error message");
    assert_eq!(error["message_type"], "exit_error");
    assert_eq!(error["code"], 11);
}

#[test]
fn ac_ag_09_reports_the_configured_restic_version() {
    let current = run(&["version", "--json"], &[]);
    assert!(current.status.success());
    let current: Value = serde_json::from_str(stdout(&current).trim()).expect("version JSON");
    assert_eq!(current["message_type"], "version");
    assert_eq!(current["version"], "0.19.1");

    let older = run(&["version", "--json"], &[("FAKE_RESTIC_VERSION", "0.18.1")]);
    assert!(older.status.success());
    let older: Value = serde_json::from_str(stdout(&older).trim()).expect("version JSON");
    assert_eq!(older["version"], "0.18.1");

    let text = run(&["version"], &[]);
    assert!(stdout(&text).starts_with("restic 0.19.1"));
}

#[test]
fn ac_ag_04_argv_file_excludes_the_environment_secret() {
    let path = temp_path("argv-secret.json");
    let output = run(
        &["snapshots", "--json", "--repo=/srv/restic-repo"],
        &[
            ("RESTIC_PASSWORD", CANARY),
            (
                "FAKE_RESTIC_ARGV_FILE",
                path.to_str().expect("a UTF-8 path"),
            ),
        ],
    );
    assert!(output.status.success(), "exit: {:?}", output.status);
    let recorded = std::fs::read_to_string(&path).expect("read the argument vector file");
    assert!(
        !recorded.contains(CANARY),
        "the argument vector holds the secret"
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn ac_ag_04_rejects_the_password_flag() {
    let output = run(
        &[
            "--password",
            "secret",
            "backup",
            "--json",
            "--repo=/x",
            "/data",
        ],
        &[("RESTIC_PASSWORD", CANARY)],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("unknown flag: --password"));
    assert!(!stderr(&output).contains(CANARY));
    assert!(!stdout(&output).contains(CANARY));
}

#[test]
fn argv_file_records_the_received_vector() {
    let path = temp_path("argv.json");
    let output = run(
        &["snapshots", "--json", "--repo=/srv/restic-repo"],
        &[
            ("RESTIC_PASSWORD", PASSWORD),
            (
                "FAKE_RESTIC_ARGV_FILE",
                path.to_str().expect("a UTF-8 path"),
            ),
        ],
    );
    assert!(output.status.success(), "exit: {:?}", output.status);
    let recorded: Vec<String> = serde_json::from_str(
        std::fs::read_to_string(&path)
            .expect("read the argument vector file")
            .trim(),
    )
    .expect("the argument vector is a JSON array");
    assert_eq!(
        recorded,
        vec!["snapshots", "--json", "--repo=/srv/restic-repo"]
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn password_comes_from_the_environment_or_a_file() {
    let environment = run(
        &["snapshots", "--json", "--repo=/x"],
        &[("RESTIC_PASSWORD", PASSWORD)],
    );
    assert!(
        environment.status.success(),
        "stderr: {}",
        stderr(&environment)
    );

    let path = temp_path("password");
    std::fs::write(&path, format!("{PASSWORD}\n")).expect("write the password file");
    let path = path.to_str().expect("a UTF-8 path");

    let option = run(
        &["snapshots", "--json", "--repo=/x", "--password-file", path],
        &[],
    );
    assert!(option.status.success(), "stderr: {}", stderr(&option));

    let file_variable = run(
        &["snapshots", "--json", "--repo=/x"],
        &[("RESTIC_PASSWORD_FILE", path)],
    );
    assert!(
        file_variable.status.success(),
        "stderr: {}",
        stderr(&file_variable)
    );
    std::fs::remove_file(path).ok();
}

#[test]
fn a_repository_command_without_a_password_fails() {
    let output = run(&["snapshots", "--json", "--repo=/x"], &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("password"));
}

#[test]
fn password_command_is_never_read() {
    let output = run(
        &["snapshots", "--json", "--repo=/x"],
        &[("RESTIC_PASSWORD_COMMAND", "echo secret")],
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "RESTIC_PASSWORD_COMMAND must not supply the password"
    );
}

#[test]
fn progress_rate_adds_a_wait() {
    let start = std::time::Instant::now();
    let output = run(
        &["backup", "--json", "--repo=/x", "/data"],
        &[
            ("RESTIC_PASSWORD", PASSWORD),
            ("FAKE_RESTIC_PROGRESS_COUNT", "3"),
            ("FAKE_RESTIC_PROGRESS_RATE", "1"),
        ],
    );
    assert!(output.status.success());
    assert!(
        start.elapsed() >= std::time::Duration::from_millis(1500),
        "the configured rate must add a wait"
    );
}

#[test]
fn unlock_and_prune_write_no_json() {
    for command in ["unlock", "prune"] {
        let output = run(
            &[command, "--json", "--repo=/x"],
            &[("RESTIC_PASSWORD", PASSWORD)],
        );
        assert!(
            output.status.success(),
            "{command} exit: {:?}",
            output.status
        );
        assert!(stdout(&output).trim().is_empty(), "{command} wrote output");
    }
}

#[test]
fn unknown_command_is_rejected() {
    let output = run(&["explode"], &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("unknown command"));
}
