//! The JSON report and the restic CLI calls that the report needs.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

/// The top-level report. One file holds all mechanisms.
#[derive(Debug, Serialize, Deserialize)]
pub struct Report {
    pub mode: String,
    pub restic_path: String,
    pub restic_version: String,
    pub repository: String,
    pub source: String,
    pub mechanisms: Vec<MechanismResult>,
}

/// The result of one mechanism.
#[derive(Debug, Serialize, Deserialize)]
pub struct MechanismResult {
    pub mechanism: String,
    pub mode: String,
    pub description: String,
    /// The repository of this mechanism. The program gives each mechanism a
    /// new repository.
    pub repository: String,
    pub api_calls: Vec<ApiCall>,
    pub ready: Ready,
    /// True when the child was not running at the stop time.
    pub exited_before_stop: bool,
    /// True when the restic output holds a JSON `summary` message.
    pub saw_summary: bool,
    /// True when the restic standard error holds an interrupt message.
    pub interrupt_message: bool,
    /// The signal that restic used, when it was graceful.
    pub graceful_signal: Option<String>,
    pub locks_before_stop: i64,
    pub locks_after_stop: i64,
    /// The standard error of the last `restic list locks` call, when it failed.
    pub locks_after_stop_stderr_tail: Vec<String>,
    pub exited_within_30s: bool,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub restic_stderr_tail: Vec<String>,
    /// The computed verdict of this run. See the spike document.
    pub verdict: String,
    pub notes: String,
}

impl MechanismResult {
    /// A result for a mechanism that could not start.
    pub fn failed(
        mechanism: &str,
        mode: &str,
        description: &str,
        error: String,
    ) -> MechanismResult {
        MechanismResult {
            mechanism: mechanism.to_string(),
            mode: mode.to_string(),
            description: description.to_string(),
            repository: String::new(),
            api_calls: Vec::new(),
            ready: Ready {
                saw_status_line: false,
                waited_ms: 0,
            },
            exited_before_stop: false,
            saw_summary: false,
            interrupt_message: false,
            graceful_signal: None,
            locks_before_stop: -1,
            locks_after_stop: -1,
            locks_after_stop_stderr_tail: Vec::new(),
            exited_within_30s: false,
            exit_code: None,
            elapsed_ms: 0,
            restic_stderr_tail: Vec::new(),
            verdict: "invalid_run".to_string(),
            notes: format!("setup failed: {error}"),
        }
    }

    /// The verdict of this run.
    ///
    /// A run is valid only when the repository held a lock before the stop and
    /// the child ran at the stop time.
    pub fn compute_verdict(&self) -> &'static str {
        if self.locks_before_stop <= 0 || self.exited_before_stop {
            return "invalid_run";
        }
        if !self.exited_within_30s {
            return "no_stop";
        }
        if self.locks_after_stop > 0 {
            return "stale_lock";
        }
        let graceful = matches!(self.exit_code, Some(130))
            || (matches!(self.exit_code, Some(1)) && self.interrupt_message);
        if graceful && self.locks_after_stop == 0 {
            "graceful_stop"
        } else {
            "invalid_run"
        }
    }
}

/// One Win32 call and its result.
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiCall {
    pub call: String,
    pub result: String,
    pub last_error: Option<u32>,
}

/// The readiness state before the stop.
#[derive(Debug, Serialize, Deserialize)]
pub struct Ready {
    pub saw_status_line: bool,
    pub waited_ms: u64,
}

/// The result of one lock probe.
#[derive(Debug)]
pub struct LockProbe {
    /// The lock count, or -1 when the call failed.
    pub count: i64,
    /// The last lines of the standard error, when the call failed.
    pub stderr_tail: Vec<String>,
}

/// Count the locks of the repository. This runs `restic list locks --no-lock`.
///
/// A count of -1 means that the call failed or that restic exited with a
/// non-zero code. The probe then holds the standard error tail.
pub fn count_locks(restic: &Path, repository: &Path) -> LockProbe {
    let output = Command::new(restic)
        .arg("-r")
        .arg(repository)
        .arg("list")
        .arg("locks")
        .arg("--no-lock")
        .output();
    match output {
        Ok(output) if output.status.success() => LockProbe {
            count: String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count() as i64,
            stderr_tail: Vec::new(),
        },
        Ok(output) => LockProbe {
            count: -1,
            stderr_tail: tail_bytes(&output.stderr, 20),
        },
        Err(error) => LockProbe {
            count: -1,
            stderr_tail: vec![format!("run restic list locks: {error}")],
        },
    }
}

/// Create a new repository at this path. The path must be empty.
pub fn init(restic: &Path, repository: &Path) -> Result<(), String> {
    let output = Command::new(restic)
        .arg("-r")
        .arg(repository)
        .arg("init")
        .output()
        .map_err(|error| format!("run restic init: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "restic init failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// The last lines of a byte buffer.
fn tail_bytes(bytes: &[u8], count: usize) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().collect();
    lines
        .iter()
        .rev()
        .take(count)
        .rev()
        .map(|line| line.to_string())
        .collect()
}

/// Remove the locks of the repository after one mechanism.
pub fn unlock(restic: &Path, repository: &Path) -> io::Result<()> {
    let status = Command::new(restic)
        .arg("-r")
        .arg(repository)
        .arg("unlock")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "restic unlock exit code {status}"
        )))
    }
}

/// The first line of `restic version`.
pub fn restic_version(restic: &Path) -> String {
    let output = Command::new(restic).arg("version").output();
    match output {
        Ok(output) => String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or("unknown")
            .to_string(),
        Err(error) => format!("error: {error}"),
    }
}
