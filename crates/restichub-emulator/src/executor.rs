//! Step execution for the emulated agent.
//!
//! The default executor waits for the step time of a profile. The optional
//! executor spawns `restichub-fake-restic`. Epic #34 adds the load profile and
//! the metrics.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use restichub_proto::agent::v1::{Step, StepStatus, step};

use crate::agent::ProgressSink;

/// Status and exit code of one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepOutcome {
    pub status: StepStatus,
    pub exit_code: i32,
}

impl StepOutcome {
    /// Reports a completed step with exit code 0.
    #[must_use]
    pub fn success() -> Self {
        Self {
            status: StepStatus::Success,
            exit_code: 0,
        }
    }
}

/// Runs the steps of a job. The caller drops the future of a step to stop it
/// on a control plane cancel (FR-AG-10).
#[async_trait]
pub trait Executor: Send + Sync {
    async fn run_step(&self, step: &Step, progress: &ProgressSink) -> StepOutcome;
}

/// Timings of the simulated executor.
#[derive(Debug, Clone, Copy)]
pub struct SimulatedProfile {
    /// Wall time of one simulated step.
    pub step_seconds: f64,
    /// Progress messages for each step.
    pub progress_ticks: u32,
}

impl Default for SimulatedProfile {
    fn default() -> Self {
        Self {
            step_seconds: 0.1,
            progress_ticks: 0,
        }
    }
}

/// The default executor. It waits for the step time and reports the progress of
/// the profile.
#[derive(Debug, Clone, Copy, Default)]
pub struct Simulated {
    pub profile: SimulatedProfile,
}

#[async_trait]
impl Executor for Simulated {
    async fn run_step(&self, _step: &Step, progress: &ProgressSink) -> StepOutcome {
        let step_seconds = self.profile.step_seconds.max(0.0);
        let ticks = self.profile.progress_ticks;
        if ticks == 0 {
            tokio::time::sleep(Duration::from_secs_f64(step_seconds)).await;
        } else {
            let slice = step_seconds / f64::from(ticks);
            for tick in 1..=ticks {
                tokio::time::sleep(Duration::from_secs_f64(slice)).await;
                let fraction = f64::from(tick) / f64::from(ticks);
                progress.report(fraction, slice * f64::from(tick)).await;
            }
        }
        StepOutcome::success()
    }
}

/// The optional executor. It spawns `restichub-fake-restic` for each step and
/// maps the exit code to a step status (FR-AG-09).
#[derive(Debug, Clone)]
pub struct FakeRestic {
    pub program: PathBuf,
}

#[async_trait]
impl Executor for FakeRestic {
    async fn run_step(&self, step: &Step, _progress: &ProgressSink) -> StepOutcome {
        let Some(arguments) = restic_arguments(step) else {
            return StepOutcome {
                status: StepStatus::Skipped,
                exit_code: -1,
            };
        };
        let mut command = tokio::process::Command::new(&self.program);
        // A cancel drops this future, so the child must not outlive it
        // (FR-AG-10).
        command.args(&arguments).kill_on_drop(true);
        match command.output().await {
            Ok(output) => {
                let code = output.status.code().unwrap_or(-1);
                StepOutcome {
                    status: status_for_exit(code),
                    exit_code: code,
                }
            }
            Err(_) => StepOutcome {
                status: StepStatus::Failed,
                exit_code: -1,
            },
        }
    }
}

/// Builds the command line of a step (FR-PR-10, ADR-0021). The function
/// returns `None` for a hook step, because the emulator runs no hook.
#[must_use]
pub fn restic_arguments(step: &Step) -> Option<Vec<String>> {
    let mut arguments = Vec::new();
    match step.kind.as_ref()? {
        step::Kind::Backup(backup) => {
            arguments.push("backup".to_owned());
            arguments.push("--json".to_owned());
            for path in &backup.paths {
                arguments.push(path.clone());
            }
        }
        step::Kind::Check(_) => {
            arguments.push("check".to_owned());
            arguments.push("--json".to_owned());
        }
        step::Kind::Snapshots(_) => {
            arguments.push("snapshots".to_owned());
            arguments.push("--json".to_owned());
        }
        step::Kind::Ls(ls) => {
            arguments.push("ls".to_owned());
            arguments.push("--json".to_owned());
            arguments.push("--".to_owned());
            arguments.push(ls.path.clone());
        }
        step::Kind::Restore(restore) => {
            arguments.push("restore".to_owned());
            arguments.push("--json".to_owned());
            arguments.push(format!("--target={}", restore.target));
            arguments.push("--".to_owned());
            arguments.push(restore.snapshot_id.clone());
        }
        step::Kind::Unlock(_) => arguments.push("unlock".to_owned()),
        step::Kind::UnlockAll(_) => {
            arguments.push("unlock".to_owned());
            arguments.push("--remove-all".to_owned());
        }
        step::Kind::Forget(_) => {
            arguments.push("forget".to_owned());
            arguments.push("--json".to_owned());
        }
        step::Kind::Prune(_) => arguments.push("prune".to_owned()),
        step::Kind::Stats(_) => {
            arguments.push("stats".to_owned());
            arguments.push("--json".to_owned());
        }
        step::Kind::Hook(_) => return None,
    }
    Some(arguments)
}

/// Maps a restic exit code to a step status (FR-AG-09).
#[must_use]
pub fn status_for_exit(code: i32) -> StepStatus {
    match code {
        0 => StepStatus::Success,
        3 => StepStatus::Warning,
        11 => StepStatus::Retry,
        10 | 12 => StepStatus::ConfigError,
        130 => StepStatus::Cancelled,
        _ => StepStatus::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use restichub_proto::agent::v1::{BackupStep, HookStep, LsStep, RestoreStep};

    fn step(kind: step::Kind) -> Step {
        Step {
            step_id: "step-1".to_owned(),
            kind: Some(kind),
            ..Step::default()
        }
    }

    #[test]
    fn ac_pr_08_puts_positional_arguments_after_double_dash() {
        let backup = step(step::Kind::Backup(BackupStep {
            repository_id: "repo-1".to_owned(),
            paths: vec!["/srv".to_owned(), "/etc".to_owned()],
            ..BackupStep::default()
        }));

        assert_eq!(
            restic_arguments(&backup),
            Some(vec![
                "backup".to_owned(),
                "--json".to_owned(),
                "/srv".to_owned(),
                "/etc".to_owned(),
            ])
        );
    }

    #[test]
    fn ac_pr_08_attaches_an_option_value() {
        let restore = step(step::Kind::Restore(RestoreStep {
            snapshot_id: "latest".to_owned(),
            target: "/restore".to_owned(),
            ..RestoreStep::default()
        }));
        let arguments = restic_arguments(&restore).expect("restore arguments");

        assert!(arguments.contains(&"--target=/restore".to_owned()));
        assert!(!arguments.contains(&"/restore".to_owned()));
    }

    #[test]
    fn ls_puts_the_path_after_double_dash() {
        let ls = step(step::Kind::Ls(LsStep {
            snapshot_id: "latest".to_owned(),
            path: "-x".to_owned(),
            ..LsStep::default()
        }));

        assert_eq!(
            restic_arguments(&ls),
            Some(vec![
                "ls".to_owned(),
                "--json".to_owned(),
                "--".to_owned(),
                "-x".to_owned(),
            ])
        );
    }

    #[test]
    fn a_hook_step_has_no_restic_arguments() {
        let hook = step(step::Kind::Hook(HookStep::default()));

        assert_eq!(restic_arguments(&hook), None);
    }

    #[test]
    fn ac_ag_09_maps_the_exit_codes() {
        assert_eq!(status_for_exit(0), StepStatus::Success);
        assert_eq!(status_for_exit(3), StepStatus::Warning);
        assert_eq!(status_for_exit(11), StepStatus::Retry);
        assert_eq!(status_for_exit(10), StepStatus::ConfigError);
        assert_eq!(status_for_exit(12), StepStatus::ConfigError);
        assert_eq!(status_for_exit(130), StepStatus::Cancelled);
        assert_eq!(status_for_exit(1), StepStatus::Failed);
    }
}
