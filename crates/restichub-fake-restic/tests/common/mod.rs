//! Helpers for the scenario and the cancel tests.
#![allow(dead_code)]
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

pub fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_restichub-fake-restic")
}

/// A fresh temp directory that belongs to one test, so the call counters and
/// the lock directory of parallel tests cannot meet.
pub fn unique_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "restichub-fake-restic-{}-{nanos}-{tag}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("create the temp dir");
    path
}

/// Writes the scenario text and returns the scenario path and a repository path.
pub fn setup(tag: &str, scenario: &str) -> (PathBuf, PathBuf) {
    let dir = unique_dir(tag);
    let path = dir.join("scenario.json");
    std::fs::write(&path, scenario).expect("write the scenario");
    let repo = dir.join("repo");
    (path, repo)
}

/// The environment of one fake restic call, without the overrides of a test.
pub fn base_command(scenario: &Path) -> Command {
    let mut command = Command::new(binary());
    command.env("RESTIC_PASSWORD", "test-password");
    command.env("FAKE_RESTIC_SCENARIO", scenario);
    for name in [
        "FAKE_RESTIC_EXIT",
        "FAKE_RESTIC_VERSION",
        "FAKE_RESTIC_PROGRESS_RATE",
        "FAKE_RESTIC_PROGRESS_COUNT",
        "FAKE_RESTIC_ARGV_FILE",
    ] {
        command.env_remove(name);
    }
    command
}

pub fn run(scenario: &Path, args: &[&str]) -> Output {
    base_command(scenario)
        .args(args)
        .output()
        .expect("run the fake restic")
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}

pub fn repo_arg(repo: &Path) -> String {
    format!("--repo={}", repo.display())
}

pub fn has_lock(repo: &Path) -> bool {
    std::fs::read_dir(repo.join("locks"))
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

/// Polls `check` until it is true or the limit passes.
pub fn wait_until(mut check: impl FnMut() -> bool, limit: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    check()
}

/// Objects of the output, one for each line.
pub fn json_lines(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("a JSON line"))
        .collect()
}
