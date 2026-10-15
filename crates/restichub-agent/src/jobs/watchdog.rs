//! The stall watchdog of a running step (ADR-0039).
//!
//! A step has no wall-clock limit by default. The watchdog stops a step only when restic makes no
//! progress for the stall timeout. It is not a schedule: it lives inside one running step and
//! starts nothing (ADR-0002).

use std::time::Duration;

use restichub_proto::agent::v1::{Step, StepProgress, step};
use tokio::time::Instant;

/// The stall timeout of a step kind when the job gives none (ADR-0039).
const DEFAULT_STALL: Duration = Duration::from_secs(60 * 60);

const STALL_VARIABLE: &str = "RESTICHUB_JOB_STALL_TIMEOUT_SECS";

/// `check` and `prune` print nothing for long periods while they work.
const LONG_STALL: Duration = Duration::from_secs(6 * 60 * 60);

/// The limits of one step: the optional maximum duration and the stall timeout (ADR-0039).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct StepLimits {
    /// Stops the step after this time whatever its progress. `None` means no limit.
    pub(super) max: Option<Duration>,
    /// Stops the step after this time without progress.
    pub(super) stall: Duration,
}

impl StepLimits {
    /// The limits of `step` in a job whose stall timeout is `job_stall_seconds`.
    pub(super) fn for_step(step: &Step, max: Option<Duration>, job_stall_seconds: u32) -> Self {
        let stall = if job_stall_seconds > 0 {
            Duration::from_secs(u64::from(job_stall_seconds))
        } else {
            default_stall(step, std::env::var(STALL_VARIABLE).ok().as_deref())
        };
        Self { max, stall }
    }
}

/// The default of a job that carries no stall timeout: the environment variable, else the kind default.
fn default_stall(step: &Step, variable: Option<&str>) -> Duration {
    if let Some(seconds) = variable
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
    {
        return Duration::from_secs(seconds);
    }
    match step.kind {
        Some(step::Kind::Check(_) | step::Kind::Prune(_)) => LONG_STALL,
        _ => DEFAULT_STALL,
    }
}

/// What restic reported the last time that it made progress.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    bytes_done: u64,
    files_done: u64,
    current_files: Vec<String>,
}

/// Measures the time since the last progress of a step.
///
/// The clock is a parameter of every call, so a test drives hours of progress without a wait.
#[derive(Debug)]
pub(super) struct Watchdog {
    stall: Duration,
    last_progress: Instant,
    seen: Option<Seen>,
}

impl Watchdog {
    pub(super) fn new(stall: Duration, now: Instant) -> Self {
        Self {
            stall,
            last_progress: now,
            seen: None,
        }
    }

    /// A line of output other than a status line is progress: `check` and `prune` print text only
    /// when they advance.
    pub(super) fn output(&mut self, now: Instant) {
        self.last_progress = now;
    }

    /// A status line is progress only when its counters or its current files changed. restic
    /// repeats the same status while it waits on a backend.
    pub(super) fn status(&mut self, progress: &StepProgress, now: Instant) {
        let seen = Seen {
            bytes_done: progress.bytes_done,
            files_done: progress.files_done,
            current_files: progress.current_files.clone(),
        };
        if self.seen.as_ref() != Some(&seen) {
            self.seen = Some(seen);
            self.last_progress = now;
        }
    }

    pub(super) fn is_stalled(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_progress) >= self.stall
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(bytes_done: u64, file: &str) -> StepProgress {
        StepProgress {
            bytes_done,
            files_done: bytes_done / 10,
            current_files: vec![file.to_owned()],
            ..StepProgress::default()
        }
    }

    #[test]
    fn ac_ag_job_limits_stall_variable_overrides_the_default() {
        let backup = Step::default();
        assert_eq!(default_stall(&backup, None), DEFAULT_STALL);
        assert_eq!(
            default_stall(&backup, Some("600")),
            Duration::from_secs(600)
        );
        assert_eq!(default_stall(&backup, Some("junk")), DEFAULT_STALL);
        assert_eq!(default_stall(&backup, Some("0")), DEFAULT_STALL);
    }

    #[test]
    fn ac_ag_12_a_step_that_makes_progress_for_longer_than_the_old_limit_is_not_stalled() {
        let start = Instant::now();
        let twelve_hours = Duration::from_secs(12 * 3600);
        let mut watchdog = Watchdog::new(Duration::from_secs(3600), start);
        let mut elapsed = Duration::ZERO;
        let mut bytes = 0;
        while elapsed < twelve_hours + Duration::from_secs(3600) {
            elapsed += Duration::from_secs(30);
            bytes += 1_000_000;
            watchdog.status(&status(bytes, "file"), start + elapsed);
            assert!(!watchdog.is_stalled(start + elapsed));
        }
    }

    #[test]
    fn ac_ag_12_a_repeated_status_line_is_not_progress() {
        let start = Instant::now();
        let mut watchdog = Watchdog::new(Duration::from_secs(3600), start);
        watchdog.status(&status(100, "file"), start);
        let mut elapsed = Duration::ZERO;
        while elapsed < Duration::from_secs(3590) {
            elapsed += Duration::from_secs(1);
            watchdog.status(&status(100, "file"), start + elapsed);
            assert!(!watchdog.is_stalled(start + elapsed));
        }
        let later = start + Duration::from_secs(3600);
        watchdog.status(&status(100, "file"), later);
        assert!(watchdog.is_stalled(later));
    }

    #[test]
    fn ac_ag_12_a_changed_file_or_counter_restarts_the_stall_time() {
        let start = Instant::now();
        let mut watchdog = Watchdog::new(Duration::from_secs(60), start);
        watchdog.status(&status(100, "a"), start);
        let almost = start + Duration::from_secs(59);
        watchdog.status(&status(100, "b"), almost);
        assert!(!watchdog.is_stalled(almost + Duration::from_secs(59)));
        assert!(watchdog.is_stalled(almost + Duration::from_secs(60)));
    }

    #[test]
    fn ac_ag_12_any_other_output_is_progress() {
        let start = Instant::now();
        let mut watchdog = Watchdog::new(Duration::from_secs(60), start);
        watchdog.output(start + Duration::from_secs(50));
        assert!(!watchdog.is_stalled(start + Duration::from_secs(100)));
        assert!(watchdog.is_stalled(start + Duration::from_secs(110)));
    }

    #[test]
    fn ac_ag_12_check_and_prune_wait_longer_than_a_backup_by_default() {
        let step = |kind| Step {
            kind: Some(kind),
            ..Step::default()
        };
        let backup = step(step::Kind::Backup(Default::default()));
        let check = step(step::Kind::Check(Default::default()));
        let prune = step(step::Kind::Prune(Default::default()));
        assert_eq!(StepLimits::for_step(&backup, None, 0).stall, DEFAULT_STALL);
        assert_eq!(StepLimits::for_step(&check, None, 0).stall, LONG_STALL);
        assert_eq!(StepLimits::for_step(&prune, None, 0).stall, LONG_STALL);
        assert_eq!(
            StepLimits::for_step(&check, None, 120).stall,
            Duration::from_secs(120)
        );
    }
}
