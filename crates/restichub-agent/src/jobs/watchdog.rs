//! The stall watchdog of a running step (ADR-0039).
//!
//! A step has no wall-clock limit by default. The watchdog stops a step only when restic makes no
//! progress for the stall timeout. It is not a schedule: it lives inside one running step and
//! starts nothing (ADR-0002).

use std::time::Duration;

use restichub_proto::agent::v1::{Step, step};
use tokio::time::Instant;

use crate::output::Progress;

/// The stall timeout of a step kind when the job gives none (ADR-0039).
const DEFAULT_STALL: Duration = Duration::from_secs(60 * 60);

/// The smallest stall timeout that the variable may set. restic waits 4 minutes for a held lock
/// and the shortest window of a profile is 5 minutes, so a shorter window would stop the step as
/// stalled while restic still waits (FR-SC-05, ADR-0039).
const MIN_STALL_VARIABLE_SECS: u64 = 5 * 60;
const MAX_STALL_VARIABLE_SECS: u64 = 7 * 24 * 60 * 60;

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
            let job = Duration::from_secs(u64::from(job_stall_seconds));
            match step.kind {
                Some(step::Kind::Check(_) | step::Kind::Prune(_)) => job.max(LONG_STALL),
                _ => job,
            }
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
        .map(|seconds| seconds.clamp(MIN_STALL_VARIABLE_SECS, MAX_STALL_VARIABLE_SECS))
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
    bytes_skipped: u64,
    files_skipped: u64,
    total_bytes: u64,
    total_files: u64,
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
    last_status_line: Option<String>,
}

impl Watchdog {
    pub(super) fn new(stall: Duration, now: Instant) -> Self {
        Self {
            stall,
            last_progress: now,
            seen: None,
            last_status_line: None,
        }
    }

    /// A line of output other than a status line is progress: `check` and `prune` print text only
    /// when they advance.
    pub(super) fn output(&mut self, now: Instant) {
        self.last_progress = now;
    }

    /// A status line is progress only when one of its counters or its current files changed. A
    /// restore grows `files_skipped` and `bytes_skipped` while it reads the files that the target
    /// already holds, and `total_files` and `total_bytes` while it scans the tree (ADR-0039).
    pub(super) fn status(&mut self, progress: &Progress, now: Instant) {
        let seen = Seen {
            bytes_done: progress.message.bytes_done,
            files_done: progress.message.files_done,
            bytes_skipped: progress.bytes_skipped,
            files_skipped: progress.files_skipped,
            total_bytes: progress.message.total_bytes,
            total_files: progress.message.total_files,
            current_files: progress.message.current_files.clone(),
        };
        if self.seen.as_ref() != Some(&seen) {
            self.seen = Some(seen);
            self.last_progress = now;
        }
    }

    /// A status line without parsed counters is progress only when its text changed.
    pub(super) fn status_line(&mut self, line: &str, now: Instant) {
        if self.last_status_line.as_deref() != Some(line) {
            self.last_status_line = Some(line.to_owned());
            self.last_progress = now;
        }
    }

    /// The verify phase of a restore starts: it prints nothing until it ends.
    ///
    /// restic prints the last status and the summary before it verifies the files and prints
    /// nothing while it verifies (`cmd/restore.go`). The step keeps the long window of `check` and
    /// `prune`, so a verify over a target that hangs still stops the step (ADR-0051).
    pub(super) fn verify(&mut self, now: Instant) {
        self.stall = self.stall.max(LONG_STALL);
        self.last_progress = now;
    }

    pub(super) fn is_stalled(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_progress) >= self.stall
    }
}

#[cfg(test)]
mod tests {
    use restichub_proto::agent::v1::StepProgress;

    use super::*;

    fn status(bytes_done: u64, file: &str) -> Progress {
        Progress {
            message: StepProgress {
                bytes_done,
                files_done: bytes_done / 10,
                current_files: vec![file.to_owned()],
                ..StepProgress::default()
            },
            files_skipped: 0,
            bytes_skipped: 0,
        }
    }

    #[test]
    fn ac_ag_12_stall_variable_overrides_the_default() {
        let backup = Step::default();
        assert_eq!(default_stall(&backup, None), DEFAULT_STALL);
        assert_eq!(
            default_stall(&backup, Some("600")),
            Duration::from_secs(600)
        );
        assert_eq!(default_stall(&backup, Some("junk")), DEFAULT_STALL);
        assert_eq!(
            default_stall(&backup, Some("1")),
            Duration::from_secs(MIN_STALL_VARIABLE_SECS)
        );
        // A window shorter than the lock wait of restic would stop the step as stalled while
        // restic still waits for the lock (FR-SC-05).
        assert_eq!(
            default_stall(&backup, Some("240")),
            Duration::from_secs(300)
        );
    }

    #[test]
    fn ac_ag_12_a_step_that_makes_progress_for_over_twelve_hours_is_not_stalled() {
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
    fn ac_ag_12_an_unchanged_status_line_is_not_progress() {
        let start = Instant::now();
        let mut watchdog = Watchdog::new(Duration::from_secs(60), start);
        watchdog.status_line("{\"message_type\":\"status\"}", start);
        let later = start + Duration::from_secs(60);
        watchdog.status_line("{\"message_type\":\"status\"}", later);
        assert!(watchdog.is_stalled(later));
        watchdog.status_line("{\"message_type\":\"status\",\"percent_done\":0.5}", later);
        assert!(!watchdog.is_stalled(later));
    }

    /// A restore over a target that holds the same data skips every file, so only the skipped
    /// counters grow (ADR-0039).
    #[test]
    fn ac_ag_12_a_restore_that_skips_files_makes_progress() {
        let start = Instant::now();
        let mut watchdog = Watchdog::new(Duration::from_secs(60), start);
        for second in 0..600 {
            let at = start + Duration::from_secs(second);
            watchdog.status(
                &Progress {
                    message: StepProgress {
                        total_files: 1_000,
                        total_bytes: 10_000_000,
                        ..StepProgress::default()
                    },
                    files_skipped: second * 2,
                    bytes_skipped: second * 15_000,
                },
                at,
            );
            assert!(!watchdog.is_stalled(at), "a skipped file is progress");
        }
    }

    /// The scan of the tree grows the totals before restic restores the first file.
    #[test]
    fn ac_ag_12_a_tree_scan_makes_progress() {
        let start = Instant::now();
        let mut watchdog = Watchdog::new(Duration::from_secs(60), start);
        for step in 0..600 {
            let at = start + Duration::from_secs(step);
            watchdog.status(
                &Progress {
                    message: StepProgress {
                        total_files: step * 100,
                        total_bytes: step * 1_000_000,
                        ..StepProgress::default()
                    },
                    files_skipped: 0,
                    bytes_skipped: 0,
                },
                at,
            );
            assert!(!watchdog.is_stalled(at), "a growing total is progress");
        }
    }

    /// The verify phase of a restore prints nothing until it ends, so the step waits the long
    /// window of `check` and `prune` there (ADR-0051).
    #[test]
    fn ac_ag_12_the_verify_phase_of_a_restore_has_the_long_window() {
        let start = Instant::now();
        let mut watchdog = Watchdog::new(Duration::from_secs(60), start);
        watchdog.status(&status(1000, "file"), start);
        watchdog.verify(start);
        let almost = start + LONG_STALL - Duration::from_secs(1);
        assert!(!watchdog.is_stalled(almost), "the verify phase still runs");
        assert!(watchdog.is_stalled(start + LONG_STALL));
    }

    /// A step that already has a longer window keeps it: the verify phase only raises the window.
    #[test]
    fn ac_ag_12_the_verify_phase_keeps_a_longer_stall_timeout() {
        let start = Instant::now();
        let longer = LONG_STALL + Duration::from_secs(3600);
        let mut watchdog = Watchdog::new(longer, start);
        watchdog.verify(start);
        assert!(!watchdog.is_stalled(start + LONG_STALL));
        assert!(watchdog.is_stalled(start + longer));
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
            StepLimits::for_step(&check, None, 3600).stall,
            LONG_STALL,
            "a check in a job with a short stall timeout keeps its own default"
        );
        assert_eq!(
            StepLimits::for_step(&prune, None, 7 * 3600).stall,
            Duration::from_secs(7 * 3600)
        );
        assert_eq!(
            StepLimits::for_step(&backup, None, 120).stall,
            Duration::from_secs(120)
        );
    }
}
