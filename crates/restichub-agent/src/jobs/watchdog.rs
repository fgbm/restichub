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

/// The largest pack file that restic writes. `--pack-size` refuses more than 128 MiB, and the
/// command of a job sets no pack size, so a pack can reach this size.
const MAX_PACK_BYTES: u64 = 128 * 1024 * 1024;

/// How many packs the raised window covers. A backup reads the files into packs and restic reports
/// `bytes_done` again only when an upload ends, so the window must hold a whole pack at the limit;
/// two packs leave room for one upload that shares the limit with the others in flight.
const UPLOAD_PACKS: u64 = 2;

/// The verify phase of a restore prints nothing on either stream, and a healthy verification of a
/// large target reads for hours. The agent cannot tell a slow disk from a hung one there, so the
/// phase gets a window of its own, longer than the 6 hours of `check` and `prune` (ADR-0051).
const VERIFY_STALL: Duration = Duration::from_secs(24 * 60 * 60);

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
        Self {
            max,
            stall: stall.max(upload_stall(step)),
        }
    }
}

/// The window that the upload limit of a `backup` step needs, or zero without a limit.
///
/// The limit is a rate at which restic cannot report progress while a pack uploads, so a window
/// that is shorter than one pack at that rate stops a healthy backup as stalled (FR-AG-05,
/// FR-AG-15).
fn upload_stall(step: &Step) -> Duration {
    let Some(step::Kind::Backup(backup)) = &step.kind else {
        return Duration::ZERO;
    };
    let bytes_per_second = backup.limit_upload_kib.saturating_mul(1024);
    if bytes_per_second == 0 {
        return Duration::ZERO;
    }
    Duration::from_secs(MAX_PACK_BYTES.saturating_mul(UPLOAD_PACKS) / bytes_per_second)
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
    /// nothing while it verifies (`cmd/restore.go`). The step keeps the longer of its own window
    /// and 24 hours, so a verification stops the step only after that time (ADR-0051).
    pub(super) fn verify(&mut self, now: Instant) {
        self.stall = self.stall.max(VERIFY_STALL);
        self.last_progress = now;
    }

    pub(super) fn is_stalled(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_progress) >= self.stall
    }
}

#[cfg(test)]
mod tests {
    use restichub_proto::agent::v1::{BackupStep, StepProgress};

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

    /// The verify phase of a restore prints nothing until it ends, so the step waits 24 hours
    /// there (ADR-0051).
    #[test]
    fn ac_ag_12_the_verify_phase_of_a_restore_has_a_window_of_24_hours() {
        let start = Instant::now();
        let mut watchdog = Watchdog::new(Duration::from_secs(60), start);
        watchdog.status(&status(1000, "file"), start);
        watchdog.verify(start);
        let almost = start + VERIFY_STALL - Duration::from_secs(1);
        assert!(!watchdog.is_stalled(almost), "the verify phase still runs");
        assert!(watchdog.is_stalled(start + VERIFY_STALL));
        assert_eq!(VERIFY_STALL, Duration::from_secs(24 * 60 * 60));
    }

    /// A step that already has a longer window keeps it: the verify phase only raises the window.
    #[test]
    fn ac_ag_12_the_verify_phase_keeps_a_longer_stall_timeout() {
        let start = Instant::now();
        let longer = VERIFY_STALL + Duration::from_secs(3600);
        let mut watchdog = Watchdog::new(longer, start);
        watchdog.verify(start);
        assert!(!watchdog.is_stalled(start + VERIFY_STALL));
        assert!(watchdog.is_stalled(start + longer));
    }

    /// A backup at a narrow link reports no new `bytes_done` while it uploads a pack. The window
    /// must hold a whole pack at the limit, or the backup stops as stalled (FR-AG-15).
    #[test]
    fn ac_ag_12_an_upload_limit_widens_the_stall_window_of_a_backup() {
        let limited = |limit_upload_kib| Step {
            kind: Some(step::Kind::Backup(BackupStep {
                limit_upload_kib,
                ..Default::default()
            })),
            ..Step::default()
        };
        assert_eq!(upload_stall(&limited(0)), Duration::ZERO);
        // 50 KiB/s: 87 minutes, longer than the 5-minute window of the profile and than the
        // 26 minutes that one pack of the default size of 16 MiB takes.
        assert_eq!(
            upload_stall(&limited(50)),
            Duration::from_secs(2 * 128 * 1024 * 1024 / (50 * 1024))
        );
        assert!(upload_stall(&limited(50)) > Duration::from_secs(44 * 60));
        // 500 KiB/s: 8 minutes, longer than the 171 s that restic held at that limit.
        assert!(upload_stall(&limited(500)) > Duration::from_secs(171));
        // A rate that uploads the packs faster than the window does not change the window.
        assert_eq!(
            StepLimits::for_step(&limited(1_000_000), None, 3600).stall,
            Duration::from_secs(3600)
        );
        // The raised window wins over a short window of the job.
        assert_eq!(
            StepLimits::for_step(&limited(50), None, 300).stall,
            upload_stall(&limited(50))
        );
        // The agent takes the maximum, so a longer window of the job stays.
        assert_eq!(
            StepLimits::for_step(&limited(50), None, 7200).stall,
            Duration::from_secs(7200)
        );
        // A step without an upload limit keeps the window of its job.
        let restore = Step {
            kind: Some(step::Kind::Restore(Default::default())),
            ..Step::default()
        };
        assert_eq!(upload_stall(&restore), Duration::ZERO);
    }

    /// A watchdog with the raised window of a backup at 50 KiB/s does not stop at the 302 s that
    /// the lab measured with a 5-minute window while restic uploaded its packs.
    #[test]
    fn ac_ag_12_a_backup_at_50_kibs_is_not_stalled_during_a_pack_upload() {
        let start = Instant::now();
        let step = Step {
            kind: Some(step::Kind::Backup(BackupStep {
                limit_upload_kib: 50,
                ..Default::default()
            })),
            ..Step::default()
        };
        let limits = StepLimits::for_step(&step, None, 300);
        let mut watchdog = Watchdog::new(limits.stall, start);
        watchdog.status(&status(59_000, "file"), start);
        let lab = start + Duration::from_secs(302);
        watchdog.status(&status(59_000, "file"), lab);
        assert!(!watchdog.is_stalled(lab), "the upload still runs");
        assert!(watchdog.is_stalled(start + limits.stall));
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
