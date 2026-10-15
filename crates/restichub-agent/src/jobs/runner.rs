//! The run of one restic step: start, output, stop and the typed outcome (FR-AG-05, FR-AG-06,
//! FR-AG-08, FR-AG-09, FR-AG-10, FR-AG-15).
//!
//! The agent reads the standard output and the standard error of restic through bounded readers
//! ([`crate::output`]), scrubs every line ([`crate::scrub`]) and sends progress and log chunks while
//! the step runs. A cancel, a stall or the maximum duration stops the step in two stages: the
//! graceful signal, then the forced kill of the process tree after the grace period (ADR-0039).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use restichub_compiler::ValidatedJob;
use restichub_compiler::arg::build_command;
use restichub_proto::agent::v1::{
    AgentMessage, ErrorDetail, LogChunk, LogStream, ProcessPriority, Step, StepProgress, StepStatus,
    agent_message, step, step_result,
};
use tokio::sync::mpsc;

use super::log::JobLog;
use crate::log_spool::Stored;
use super::watchdog::{StepLimits, Watchdog};
use super::{Jobs, lock};
use crate::exec::JobChild;
use crate::output::{Accepted, Collector, Line, OutputKind, Pipe, spawn_reader};
use crate::scrub::Scrubber;

/// How often the agent checks the restic child for its exit.
pub(super) const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// The shortest time between two progress messages of a step (FR-AG-08).
const PROGRESS_INTERVAL: Duration = Duration::from_secs(2);

/// The longest time that log lines wait before the agent sends them.
pub(super) const LOG_INTERVAL: Duration = Duration::from_secs(1);

/// The log size that the agent sends at once, whatever the interval.
pub(super) const LOG_FLUSH_BYTES: usize = 16 * 1024;

/// How long the agent waits for the rest of the output after restic exited.
pub(super) const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a log or progress message may wait for room in the stream buffer, so a slow control
/// plane never blocks a cancel.
const SEND_WAIT: Duration = Duration::from_millis(250);

/// The lines that the agent takes before it looks at the child again.
pub(super) const LINES_PER_TURN: usize = 256;

/// Why a step is not a success. The stable code reaches the control plane (NFR-DOC-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepError {
    Warning,
    LockHeld,
    ConfigError,
    Failed,
    Cancelled,
    /// The step ran longer than its maximum duration (ADR-0039).
    MaxDuration,
    /// restic made no progress for the stall timeout (ADR-0039).
    Stalled,
    /// A hook ran longer than its own timeout (FR-HK-02).
    TimedOut,
    NotStarted,
}

impl StepError {
    fn code(self) -> &'static str {
        match self {
            Self::Warning => "step_warning",
            Self::LockHeld => "repository_locked",
            Self::ConfigError => "step_config_error",
            Self::Failed => "step_failed",
            Self::Cancelled => "step_cancelled",
            Self::MaxDuration => "step_max_duration",
            Self::Stalled => "step_stalled",
            Self::TimedOut => "step_timeout",
            Self::NotStarted => "step_not_started",
        }
    }

    fn for_status(status: StepStatus) -> Option<Self> {
        match status {
            StepStatus::Warning => Some(Self::Warning),
            StepStatus::Retry => Some(Self::LockHeld),
            StepStatus::ConfigError => Some(Self::ConfigError),
            StepStatus::Failed => Some(Self::Failed),
            StepStatus::Cancelled => Some(Self::Cancelled),
            StepStatus::Success | StepStatus::Skipped | StepStatus::Unspecified => None,
        }
    }

    pub(super) fn detail(self) -> ErrorDetail {
        ErrorDetail {
            code: self.code().to_owned(),
            params: Vec::new(),
        }
    }
}

/// Exit status and code of one step.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StepOutcome {
    pub(super) status: StepStatus,
    pub(super) exit_code: i32,
    /// Set after a forced kill of the tree: restic may have left a lock (FR-AG-10, AC-AG-07).
    pub(super) possible_stale_lock: bool,
    pub(super) error: Option<StepError>,
    /// The typed output that the agent parsed from the restic JSON (FR-AG-08).
    pub(super) output: Option<step_result::Output>,
}

impl StepOutcome {
    pub(super) fn new(status: StepStatus, exit_code: i32) -> Self {
        Self {
            status,
            exit_code,
            possible_stale_lock: false,
            error: StepError::for_status(status),
            output: None,
        }
    }

    pub(super) fn failed() -> Self {
        Self::new(StepStatus::Failed, -1)
    }

    pub(super) fn skipped() -> Self {
        Self::new(StepStatus::Skipped, -1)
    }

    pub(super) fn cancelled(possible_stale_lock: bool) -> Self {
        Self {
            possible_stale_lock,
            ..Self::new(StepStatus::Cancelled, -1)
        }
    }

    pub(super) fn timed_out() -> Self {
        Self {
            error: Some(StepError::TimedOut),
            ..Self::new(StepStatus::Failed, -1)
        }
    }

    pub(super) fn limit_exceeded(limit: LimitStop, possible_stale_lock: bool) -> Self {
        Self {
            possible_stale_lock,
            error: Some(match limit {
                LimitStop::MaxDuration => StepError::MaxDuration,
                LimitStop::Stalled => StepError::Stalled,
            }),
            ..Self::new(StepStatus::Failed, -1)
        }
    }

    pub(super) fn not_started() -> Self {
        Self {
            error: Some(StepError::NotStarted),
            ..Self::new(StepStatus::Failed, -1)
        }
    }

    fn from_exit(code: i32) -> Self {
        Self::new(status_for_exit(code), code)
    }
}

/// Why the agent stops a step on its own (ADR-0039).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LimitStop {
    MaxDuration,
    Stalled,
}

/// The handle that stops the restic child of a running job (FR-AG-10).
#[derive(Clone)]
pub(super) struct CancelHandle {
    pub(super) child: Arc<Mutex<Option<JobChild>>>,
    cancels: Arc<AtomicU32>,
}

impl CancelHandle {
    pub(super) fn new() -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            cancels: Arc::new(AtomicU32::new(0)),
        }
    }

    pub(super) fn cancel(&self) {
        let _ = self
            .cancels
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                Some(count.saturating_add(1))
            });
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancel_count() > 0
    }

    /// How many cancels arrived. A cleanup step reads it to see a cancel that came while it runs.
    pub(super) fn cancel_count(&self) -> u32 {
        self.cancels.load(Ordering::SeqCst)
    }
}

/// Maps a restic exit code to a step status (FR-AG-09, AC-AG-05).
#[must_use]
pub(super) fn status_for_exit(code: i32) -> StepStatus {
    match code {
        0 => StepStatus::Success,
        3 => StepStatus::Warning,
        11 => StepStatus::Retry,
        10 | 12 => StepStatus::ConfigError,
        130 => StepStatus::Cancelled,
        _ => StepStatus::Failed,
    }
}

/// Starts the collector of a step with the parameters that shape its output (AC-RS-02, FR-RS-04).
fn collector_for(step: &Step, kind: OutputKind, job_id: &str) -> Collector {
    let collector = Collector::new(kind, job_id, &step.step_id);
    match step.kind.as_ref() {
        Some(step::Kind::Ls(ls)) => collector.with_window(&ls.path, ls.offset, ls.limit),
        Some(step::Kind::Restore(restore)) => collector.with_dry_run(restore.dry_run),
        _ => collector,
    }
}

/// The priority that a step asks for (FR-AG-15).
fn priority_of(step: &Step) -> ProcessPriority {
    let value = match step.kind.as_ref() {
        Some(step::Kind::Backup(backup)) => backup.priority,
        Some(step::Kind::Check(check)) => check.priority,
        Some(step::Kind::Forget(forget)) => forget.priority,
        Some(step::Kind::Prune(prune)) => prune.priority,
        _ => 0,
    };
    ProcessPriority::try_from(value).unwrap_or(ProcessPriority::Unspecified)
}

/// Output state of one running step.
struct StepOutput {
    step_id: String,
    collector: Collector,
    scrubber: Scrubber,
    /// The newest progress message that the agent did not send yet.
    progress: Option<StepProgress>,
    watchdog: Watchdog,
    last_progress: Option<tokio::time::Instant>,
    last_log: tokio::time::Instant,
}

impl StepOutput {
    /// Handles one line of restic output: a log line, a progress message or typed output.
    fn line(&mut self, line: Line, log: &mut JobLog) {
        let now = tokio::time::Instant::now();
        let text = String::from_utf8_lossy(&line.bytes);
        match line.pipe {
            Pipe::Stdout => match self.collector.accept(&text, line.truncated) {
                Accepted::Log(text) => {
                    self.watchdog.output(now);
                    log.push(
                        &self.step_id,
                        LogStream::Stdout,
                        &self.scrubber.scrub_line(&text, line.truncated),
                    );
                }
                Accepted::Progress(progress) => {
                    self.watchdog.status(&progress, now);
                    self.progress = Some(progress);
                }
                Accepted::Status(line) => self.watchdog.status_line(&line, now),
                Accepted::Consumed => self.watchdog.output(now),
            },
            Pipe::Stderr => {
                self.watchdog.output(now);
                let text = crate::output::stderr_text(&text, line.truncated);
                log.push(
                    &self.step_id,
                    LogStream::Stderr,
                    &self.scrubber.scrub_line(&text, line.truncated),
                );
            }
        }
    }
}

impl Jobs {
    /// Runs one step with the installed restic and stops it in two stages (FR-AG-05, FR-AG-06,
    /// FR-AG-09, FR-AG-10).
    ///
    /// `limits` holds the maximum duration of the step, or `None` for no limit, and the stall
    /// timeout (ADR-0039).
    pub(super) async fn run_step(
        &self,
        validated: &ValidatedJob,
        step: &Step,
        cancel: &CancelHandle,
        limits: StepLimits,
        log: &mut JobLog,
    ) -> StepOutcome {
        // A cancel that arrived before the spawn starts no process (FR-AG-10).
        if cancel.is_cancelled() {
            return StepOutcome::cancelled(false);
        }
        let line = match build_command(validated, step) {
            Ok(line) => line.with_json(),
            Err(error) => {
                tracing::warn!(%error, "cannot build the restic command");
                return StepOutcome::not_started();
            }
        };
        let step_id = step.step_id.as_str();
        let scrubber = Scrubber::for_command(&line);
        // The hash check and the start share one opening of the binary (FR-AG-11, threat T20).
        let spawned = self.expected_sha256().and_then(|expected| {
            crate::exec::spawn_job_restic(
                &self.program,
                &expected,
                &line,
                &self.cache,
                priority_of(step),
            )
        });
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => {
                let detail = format!("{error:#}");
                tracing::warn!(%detail, "cannot start restic");
                log.push(step_id, LogStream::Agent, "restic did not start");
                self.send_log(log).await;
                return StepOutcome::not_started();
            }
        };
        // The secrets leave the memory of the agent with the command line (FR-AG-07).
        drop(line);

        let kind = step.kind.as_ref().map_or(OutputKind::Plain, OutputKind::of);
        let (stdout, stderr) = child.take_output();
        let (tx, mut lines) = mpsc::channel::<Line>(kind.queue_depth());
        if let Some(stdout) = stdout {
            spawn_reader(stdout, Pipe::Stdout, kind.stdout_limit(), tx.clone());
        }
        if let Some(stderr) = stderr {
            spawn_reader(stderr, Pipe::Stderr, crate::output::MAX_LINE, tx.clone());
        }
        drop(tx);
        let mut output = StepOutput {
            step_id: step_id.to_owned(),
            collector: collector_for(step, kind, &validated.job().job_id),
            scrubber,
            progress: None,
            watchdog: Watchdog::new(limits.stall, tokio::time::Instant::now()),
            last_progress: None,
            last_log: tokio::time::Instant::now(),
        };

        *lock(&cancel.child) = Some(child);
        let started = tokio::time::Instant::now();
        let mut limit_stop: Option<LimitStop> = None;
        let mut lines_open = true;
        // The stop is two stages: the graceful signal first, then the forced kill of the tree after
        // the grace period (FR-AG-10, AC-AG-07).
        let mut stopped_at: Option<tokio::time::Instant> = None;
        let mut hard_killed = false;
        // An `ls` page that is full needs no more of the listing, so the agent stops restic.
        let mut page_done = false;
        let code = loop {
            let reaped = {
                let mut slot = lock(&cancel.child);
                match slot.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(Some(status)) => {
                            let code = status.code().unwrap_or(-1);
                            *slot = None;
                            Some(code)
                        }
                        Ok(None) => None,
                        Err(error) => {
                            tracing::warn!(%error, "cannot wait for restic");
                            *slot = None;
                            return StepOutcome::failed();
                        }
                    },
                    None => None,
                }
            };
            if let Some(code) = reaped {
                break code;
            }
            if stopped_at.is_none() {
                if output.collector.page_complete() {
                    page_done = true;
                    interrupt(&cancel.child);
                    stopped_at = Some(tokio::time::Instant::now());
                } else if cancel.is_cancelled() {
                    interrupt(&cancel.child);
                    stopped_at = Some(tokio::time::Instant::now());
                } else if limits.max.is_some_and(|max| started.elapsed() >= max) {
                    limit_stop = Some(LimitStop::MaxDuration);
                    interrupt(&cancel.child);
                    stopped_at = Some(tokio::time::Instant::now());
                } else if output.watchdog.is_stalled(tokio::time::Instant::now()) {
                    limit_stop = Some(LimitStop::Stalled);
                    log.push(
                        step_id,
                        LogStream::Agent,
                        "restic made no progress; stopping the step",
                    );
                    interrupt(&cancel.child);
                    stopped_at = Some(tokio::time::Instant::now());
                }
            } else if !hard_killed
                && stopped_at.is_some_and(|since| since.elapsed() >= self.stop_grace)
            {
                kill_tree(&cancel.child);
                hard_killed = true;
            }
            if lines_open {
                tokio::select! {
                    line = lines.recv() => match line {
                        Some(line) => {
                            output.line(line, log);
                            for _ in 0..LINES_PER_TURN {
                                match lines.try_recv() {
                                    Ok(line) => output.line(line, log),
                                    Err(_) => break,
                                }
                            }
                        }
                        None => lines_open = false,
                    },
                    () = tokio::time::sleep(POLL_INTERVAL) => {}
                }
            } else {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            self.flush_output(&mut output, log, false).await;
        };

        // The pipes stay open while a grandchild lives, so the wait for the rest is bounded.
        let deadline = tokio::time::Instant::now() + DRAIN_TIMEOUT;
        while lines_open
            && let Ok(Some(line)) = tokio::time::timeout_at(deadline, lines.recv()).await
        {
            output.line(line, log);
        }
        output.progress = None;
        self.flush_output(&mut output, log, true).await;

        let mut outcome = if cancel.is_cancelled() {
            StepOutcome::cancelled(hard_killed)
        } else if let Some(limit) = limit_stop {
            StepOutcome::limit_exceeded(limit, hard_killed)
        } else if page_done {
            StepOutcome {
                possible_stale_lock: hard_killed,
                ..StepOutcome::new(StepStatus::Success, 0)
            }
        } else {
            StepOutcome::from_exit(code)
        };
        outcome.output = output.collector.finish();
        outcome
    }

    /// Sends the progress and the log lines that are due (FR-AG-08).
    async fn flush_output(&self, output: &mut StepOutput, log: &mut JobLog, last: bool) {
        let now = tokio::time::Instant::now();
        if output.progress.is_some()
            && output
                .last_progress
                .is_none_or(|sent| now.duration_since(sent) >= PROGRESS_INTERVAL)
            && let Some(progress) = output.progress.take()
        {
            output.last_progress = Some(now);
            self.send_within(
                AgentMessage {
                    payload: Some(agent_message::Payload::StepProgress(progress)),
                },
                SEND_WAIT,
            )
            .await;
        }
        if log.pending_bytes() > 0
            && (last
                || log.pending_bytes() >= LOG_FLUSH_BYTES
                || now.duration_since(output.last_log) >= LOG_INTERVAL)
        {
            output.last_log = now;
            self.send_log(log).await;
        }
    }

    /// Sends the log chunks that are due.
    ///
    /// A control plane that acknowledges chunks gets them through the spool: a chunk that finds no
    /// room within a moment stays there and goes out again after the next connection (FR-LG-07).
    /// Otherwise a chunk without room is lost; the journal tail still keeps the newest ones
    /// (AC-PR-07).
    pub(super) async fn send_log(&self, log: &mut JobLog) {
        let chunks = log.take_pending();
        if !self.log_acks.load(Ordering::SeqCst) {
            for chunk in chunks {
                self.send_within(
                    AgentMessage {
                        payload: Some(agent_message::Payload::LogChunk(chunk)),
                    },
                    SEND_WAIT,
                )
                .await;
            }
            return;
        }
        for chunk in chunks {
            self.spool_chunk(log, chunk);
        }
        self.flush_log(&log.job_id().to_owned(), true).await;
    }

    /// Puts a chunk in the spool. A full spool drops the chunk and counts its lines; the next
    /// chunk that fits is preceded by one marker line with the count (FR-LG-07).
    fn spool_chunk(&self, log: &mut JobLog, chunk: LogChunk) {
        let lost = log.lost_lines();
        if let Some(marker) = log.take_loss_marker(&chunk.step_id)
            && !self.keep_in_spool(log.job_id(), &marker)
        {
            log.note_lost(lost);
        }
        if !self.keep_in_spool(log.job_id(), &chunk) {
            log.note_lost(crate::log_spool::line_count(&chunk));
        }
    }

    fn keep_in_spool(&self, job_id: &str, chunk: &LogChunk) -> bool {
        match self.log_spool.append(job_id, chunk) {
            Ok(Stored::Kept) => true,
            Ok(Stored::Full) => false,
            Err(error) => {
                tracing::warn!(%error, "cannot write the log spool");
                false
            }
        }
    }

    /// Sends the spooled chunks that the open stream did not get yet, in order (FR-LG-07).
    ///
    /// The live call never waits for the order lock or for room in the stream buffer, so a slow
    /// control plane never blocks a cancel (FR-AG-10). A failed send leaves the chunk in the spool
    /// for the next call.
    pub(super) async fn flush_log(&self, job_id: &str, live: bool) {
        let _order = if live {
            match self.log_order.try_lock() {
                Ok(guard) => guard,
                Err(_) => return,
            }
        } else {
            self.log_order.lock().await
        };
        loop {
            let batch = self.log_spool.claim_unsent(job_id);
            if batch.is_empty() {
                return;
            }
            let total = batch.len();
            for (index, chunk) in batch.into_iter().enumerate() {
                let message = AgentMessage {
                    payload: Some(agent_message::Payload::LogChunk(chunk)),
                };
                let sent = if live {
                    self.send_live(message, SEND_WAIT).await
                } else {
                    self.send(message).await
                };
                if !sent {
                    self.log_spool.unclaim(job_id, total - index);
                    return;
                }
            }
            if live {
                return;
            }
        }
    }
}

/// Sends the graceful stop signal to the child of a step (FR-AG-10).
pub(super) fn interrupt(child: &Mutex<Option<JobChild>>) {
    if let Some(child) = lock(child).as_ref()
        && let Err(error) = child.interrupt()
    {
        tracing::warn!(%error, "cannot stop the restic child");
    }
}

/// Kills the whole process tree of the child of a step (FR-AG-10).
pub(super) fn kill_tree(child: &Mutex<Option<JobChild>>) {
    if let Some(child) = lock(child).as_mut()
        && let Err(error) = child.kill_tree()
    {
        tracing::warn!(%error, "cannot kill the restic child");
    }
}

#[cfg(test)]
mod tests {
    use restichub_proto::agent::v1::StepStatus;

    use super::{LimitStop, StepError, StepOutcome, status_for_exit};

    #[test]
    fn ac_ag_05_maps_the_restic_exit_codes() {
        assert_eq!(status_for_exit(0), StepStatus::Success);
        assert_eq!(status_for_exit(1), StepStatus::Failed);
        assert_eq!(status_for_exit(3), StepStatus::Warning);
        assert_eq!(status_for_exit(10), StepStatus::ConfigError);
        assert_eq!(status_for_exit(11), StepStatus::Retry);
        assert_eq!(status_for_exit(12), StepStatus::ConfigError);
        assert_eq!(status_for_exit(130), StepStatus::Cancelled);
        assert_eq!(status_for_exit(99), StepStatus::Failed);
    }

    #[test]
    fn fr_ag_09_a_step_that_did_not_succeed_has_a_stable_error_code() {
        assert_eq!(StepOutcome::from_exit(0).error, None);
        assert_eq!(StepOutcome::from_exit(3).error, Some(StepError::Warning));
        assert_eq!(
            StepOutcome::from_exit(11)
                .error
                .map(StepError::detail)
                .map(|detail| detail.code),
            Some("repository_locked".to_owned())
        );
        assert_eq!(StepOutcome::skipped().error, None);
        assert_eq!(
            StepOutcome::limit_exceeded(LimitStop::MaxDuration, true).error,
            Some(StepError::MaxDuration)
        );
        assert_eq!(
            StepOutcome::limit_exceeded(LimitStop::Stalled, false).error,
            Some(StepError::Stalled)
        );
        assert!(StepOutcome::limit_exceeded(LimitStop::Stalled, true).possible_stale_lock);
    }
}
