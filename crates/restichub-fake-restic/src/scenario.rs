//! Scenario control for the fake restic (FR-AG-09, FR-AG-10).
//!
//! A scenario tailors one or more commands: the duration, the progress rate, a
//! failure, a stale lock and the behaviour on a cancel signal. The file is
//! JSON. The workspace has no YAML crate, and ADR-0019 keeps serde-saphyr
//! inside the control plane. The environment variables of the crate README
//! override the values of the scenario: the order is default, then scenario,
//! then environment variable.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Number of `status` messages when the scenario names neither a count nor a duration.
pub const DEFAULT_PROGRESS_COUNT: u64 = 3;

/// How a command treats the repository lock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockMode {
    /// The command does not touch the lock directory.
    #[default]
    None,
    /// The command takes the lock and releases it on a clean end. A present
    /// lock makes the command exit with code 11.
    Manage,
    /// The command leaves a lock behind and exits with code 11. A later
    /// `unlock` removes it.
    Stale,
}

/// Behaviour on a cancel signal: SIGINT or SIGTERM on Unix, Ctrl+C or
/// Ctrl+Break on Windows.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Cancel {
    /// The process continues after the signal. The caller kills it.
    pub ignore: bool,
    /// The process waits this long after the signal before it exits with code 130.
    pub delay_ms: u64,
}

/// The values of one command of the scenario file.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CommandScenario {
    pub duration_ms: u64,
    pub progress_rate: f64,
    pub progress_count: u64,
    pub exit_code: Option<u8>,
    pub fail_at_step: Option<u64>,
    pub fail_at_percent: Option<f64>,
    pub partial_output: bool,
    pub stderr_lines: Vec<String>,
    pub lock: LockMode,
    pub cancel: Cancel,
}

impl Default for CommandScenario {
    fn default() -> Self {
        Self {
            duration_ms: 0,
            progress_rate: 0.0,
            progress_count: DEFAULT_PROGRESS_COUNT,
            exit_code: None,
            fail_at_step: None,
            fail_at_percent: None,
            partial_output: false,
            stderr_lines: Vec::new(),
            lock: LockMode::None,
            cancel: Cancel::default(),
        }
    }
}

/// The scenario of one command after the environment overrides.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub duration_ms: u64,
    pub progress_rate: f64,
    pub progress_count: u64,
    pub exit_code: u8,
    pub fail_at_step: Option<u64>,
    pub fail_at_percent: Option<f64>,
    pub partial_output: bool,
    pub stderr_lines: Vec<String>,
    pub lock: LockMode,
    pub cancel: Cancel,
}

/// The scenario file: a default per command and an optional sequence per call.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Scenario {
    pub commands: HashMap<String, CommandScenario>,
    pub command_sequence: HashMap<String, Vec<CommandScenario>>,
}

impl Scenario {
    pub fn load(path: &Path) -> io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text).map_err(io::Error::other)
    }

    /// The values of the `index`-th call of `command`.
    ///
    /// A sequence entry has priority. When the sequence is shorter than the
    /// call count, the default of the command applies.
    pub fn for_command(&self, command: &str, index: usize) -> CommandScenario {
        if let Some(sequence) = self.command_sequence.get(command)
            && let Some(item) = sequence.get(index)
        {
            return item.clone();
        }
        self.commands.get(command).cloned().unwrap_or_default()
    }
}

impl CommandScenario {
    /// Applies the environment variables of the crate README as overrides.
    pub fn resolve(&self, env_exit: Option<u8>) -> Resolved {
        Resolved {
            duration_ms: self.duration_ms,
            progress_rate: env_f64("FAKE_RESTIC_PROGRESS_RATE")
                .unwrap_or(self.progress_rate)
                .max(0.0),
            progress_count: env_u64("FAKE_RESTIC_PROGRESS_COUNT")
                .unwrap_or(self.progress_count)
                .max(1),
            exit_code: env_exit.or(self.exit_code).unwrap_or(0),
            fail_at_step: self.fail_at_step,
            fail_at_percent: self.fail_at_percent,
            partial_output: self.partial_output,
            stderr_lines: self.stderr_lines.clone(),
            lock: self.lock,
            cancel: self.cancel.clone(),
        }
    }
}

/// Location of the call counters next to the scenario file.
pub fn state_path(scenario: &Path) -> PathBuf {
    let mut name = scenario.as_os_str().to_os_string();
    name.push(".state.json");
    PathBuf::from(name)
}

/// Returns the index of the next call of `command` and bumps the counter.
pub fn next_invocation(scenario: &Path, command: &str) -> usize {
    let path = state_path(scenario);
    let mut counts: HashMap<String, usize> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let index = counts.get(command).copied().unwrap_or(0);
    counts.insert(command.to_string(), index + 1);
    if let Ok(text) = serde_json::to_string(&counts) {
        let _ = std::fs::write(&path, text);
    }
    index
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse().ok()
}

fn env_f64(name: &str) -> Option<f64> {
    std::env::var(name).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{CommandScenario, LockMode, Scenario};

    #[test]
    fn a_sequence_entry_has_priority_over_the_default() {
        let scenario: Scenario = serde_json::from_str(
            r#"{
                "commands": {"backup": {"exit_code": 3}},
                "command_sequence": {
                    "backup": [{"exit_code": 11}, {"exit_code": 0}]
                }
            }"#,
        )
        .expect("parse");
        assert_eq!(scenario.for_command("backup", 0).exit_code, Some(11));
        assert_eq!(scenario.for_command("backup", 1).exit_code, Some(0));
        assert_eq!(scenario.for_command("backup", 2).exit_code, Some(3));
    }

    #[test]
    fn defaults_apply_to_an_absent_command() {
        let scenario = Scenario::default();
        let command = scenario.for_command("backup", 0);
        assert_eq!(command.progress_count, 3);
        assert_eq!(command.lock, LockMode::None);
        assert!(!command.cancel.ignore);
    }

    #[test]
    fn the_environment_overrides_the_scenario() {
        let command = CommandScenario {
            progress_count: 5,
            progress_rate: 2.0,
            exit_code: Some(3),
            ..CommandScenario::default()
        };
        // The variable is absent in this test process, so the scenario value stands.
        let resolved = command.resolve(None);
        assert_eq!(resolved.progress_count, 5);
        assert_eq!(resolved.exit_code, 3);
        let resolved = command.resolve(Some(11));
        assert_eq!(resolved.exit_code, 11);
    }
}
