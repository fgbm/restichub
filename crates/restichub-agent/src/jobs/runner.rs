//! The run of one restic step: start, output, stop and the typed outcome (FR-AG-05, FR-AG-06,
//! FR-AG-08, FR-AG-09, FR-AG-10, FR-AG-15).
//!
//! The agent reads the standard output and the standard error of restic through bounded readers
//! ([`crate::output`]), scrubs every line ([`crate::scrub`]) and sends progress and log chunks while
//! the step runs. A cancel, a stall or the maximum duration stops the step in two stages: the
//! graceful signal, then the forced kill of the process tree after the grace period (ADR-0039).

use std::process::{ChildStderr, ChildStdout};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use prost_types::Timestamp;
use restichub_compiler::ValidatedJob;
use restichub_compiler::arg::build_command;
use restichub_proto::agent::v1::{
    AgentMessage, ErrorDetail, KeyValue, LogStream, ProcessPriority, Step, StepProgress,
    StepStatus, agent_message, step, step_result,
};
use restichub_proto::restore_path::{PathFlavor, components};
use tokio::sync::mpsc;

use super::command_text::command_line_text;
use super::log::JobLog;
use super::watchdog::{StepLimits, Watchdog};
use super::{Jobs, lock};
use crate::exec::JobChild;
use crate::output::{
    Accepted, Collector, Line, OutputKind, Pipe, fill_link_targets, has_symlink, spawn_reader,
};
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
pub(super) const SEND_WAIT: Duration = Duration::from_millis(250);

/// The lines that the agent takes before it looks at the child again.
pub(super) const LINES_PER_TURN: usize = 256;

/// The parameter of `step_not_started` that holds the scrubbed cause. The control plane stores a
/// parameter value of up to this many characters.
pub(super) const REASON_PARAM: &str = "reason";
pub(super) const MAX_REASON_CHARS: usize = 500;

/// Why a step is not a success. The stable code reaches the control plane (NFR-DOC-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepError {
    Warning,
    LockHeld,
    ConfigError,
    /// restic exit code 10: no repository at the location. Before an init it is the expected
    /// answer (FR-RP-02).
    RepositoryMissing,
    /// restic could not write a local repository because the path is read-only for the agent
    /// (ADR-0045).
    ReadOnly,
    Failed,
    Cancelled,
    /// The step ran longer than its maximum duration (ADR-0039).
    MaxDuration,
    /// restic made no progress for the stall timeout (ADR-0039).
    Stalled,
    /// A script ran longer than its own timeout (FR-HK-09).
    TimedOut,
    NotStarted,
}

impl StepError {
    fn code(self) -> &'static str {
        match self {
            Self::Warning => "step_warning",
            Self::LockHeld => "repository_locked",
            Self::ConfigError => "step_config_error",
            Self::RepositoryMissing => "repository_missing",
            Self::ReadOnly => "repository_read_only",
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
    /// Why restic did not start, scrubbed and bounded. It reaches the control plane even when the
    /// log cannot, for example when the log spool cannot be written.
    pub(super) reason: Option<String>,
}

impl StepOutcome {
    pub(super) fn new(status: StepStatus, exit_code: i32) -> Self {
        Self {
            status,
            exit_code,
            possible_stale_lock: false,
            error: StepError::for_status(status),
            output: None,
            reason: None,
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

    /// `reason` is already scrubbed. It holds no secret, and a path is fine.
    pub(super) fn not_started_because(reason: &str) -> Self {
        Self {
            reason: Some(reason.chars().take(MAX_REASON_CHARS).collect()),
            ..Self::not_started()
        }
    }

    pub(super) fn error_detail(&self) -> Option<ErrorDetail> {
        let mut detail = self.error?.detail();
        if let Some(reason) = &self.reason {
            detail.params.push(KeyValue {
                key: REASON_PARAM.to_owned(),
                value: reason.clone(),
            });
        }
        Some(detail)
    }

    pub(super) fn from_exit(code: i32) -> Self {
        let outcome = Self::new(status_for_exit(code), code);
        if code == 10 {
            Self {
                error: Some(StepError::RepositoryMissing),
                ..outcome
            }
        } else {
            outcome
        }
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
    step: Arc<Mutex<Option<ChildStep>>>,
    cancels: Arc<AtomicU32>,
}

/// The step that owns the child in the slot of a [`CancelHandle`].
struct ChildStep {
    step_id: String,
    started_at: Timestamp,
    restic: bool,
}

/// A child that the task of a job left running, after the agent stopped it.
pub(super) struct Orphan {
    pub(super) step_id: String,
    pub(super) started_at: Timestamp,
    /// Restic may have left its lock in the repository (FR-AG-10, AC-AG-07).
    pub(super) possible_stale_lock: bool,
}

impl CancelHandle {
    pub(super) fn new() -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            step: Arc::new(Mutex::new(None)),
            cancels: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Puts the child of a step in the slot. It runs right after the spawn, so a panic later in the
    /// step still leaves a child that the supervisor of the job can stop.
    pub(super) fn adopt(&self, child: JobChild, step_id: &str, restic: bool) {
        *lock(&self.step) = Some(ChildStep {
            step_id: step_id.to_owned(),
            started_at: super::now_timestamp(),
            restic,
        });
        *lock(&self.child) = Some(child);
    }

    /// Takes the output pipes of the child in the slot.
    pub(super) fn take_output(&self) -> (Option<ChildStdout>, Option<ChildStderr>) {
        lock(&self.child)
            .as_mut()
            .map_or((None, None), JobChild::take_output)
    }

    /// Stops the child that the dead task of a job left in the slot and waits until it exits, so
    /// no zombie stays behind (FR-AG-10). `None` when the slot holds no child.
    pub(super) async fn stop_orphan(&self) -> Option<Orphan> {
        let mut child = lock(&self.child).take()?;
        let step = lock(&self.step).take()?;
        let left_lock = match child.try_wait() {
            Ok(Some(status)) => lock_possibly_left(&status),
            Ok(None) | Err(_) => {
                if let Err(error) = child.kill_tree() {
                    tracing::warn!(%error, "cannot kill the restic child");
                }
                let deadline = tokio::time::Instant::now() + DRAIN_TIMEOUT;
                while !matches!(child.try_wait(), Ok(Some(_)) | Err(_))
                    && tokio::time::Instant::now() < deadline
                {
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
                true
            }
        };
        Some(Orphan {
            step_id: step.step_id,
            started_at: step.started_at,
            possible_stale_lock: step.restic && left_lock,
        })
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

/// True when restic probably ended without removing its lock (AC-SC-06).
///
/// Exit code 2 is a panic of the Go runtime on every platform. A process that Windows terminates
/// from outside has no signal: it carries the code that the killer chose (1, -1 or an NTSTATUS),
/// so any code that restic does not use itself counts. An extra unlock is safe, because the
/// step removes only stale locks.
fn lock_possibly_left(status: &std::process::ExitStatus) -> bool {
    #[cfg(unix)]
    if std::os::unix::process::ExitStatusExt::signal(status).is_some() {
        return true;
    }
    let code = status.code().unwrap_or(-1);
    code == 2 || (cfg!(windows) && !matches!(code, 0 | 3 | 10..=12 | 130))
}

/// Starts the collector of a step with the parameters that shape its output (AC-RS-02, FR-RS-04).
fn collector_for(step: &Step, kind: OutputKind, job_id: &str) -> Collector {
    let collector = Collector::new(kind, job_id, &step.step_id);
    match step.kind.as_ref() {
        Some(step::Kind::Ls(ls)) => collector.with_window(&ls.path, ls.offset, ls.limit),
        Some(step::Kind::Restore(restore)) => collector
            .with_dry_run(restore.dry_run)
            .with_verify(restore.verify),
        _ => collector,
    }
}

/// The priority that a step asks for (FR-AG-15).
pub(super) fn priority_of(step: &Step) -> ProcessPriority {
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
    stderr_is_progress: bool,
    last_progress: Option<tokio::time::Instant>,
    last_log: tokio::time::Instant,
    /// The local repository locations of the job, to recognize a write that the sandbox refused.
    local_repositories: Vec<String>,
    read_only: bool,
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
                    self.progress = Some(progress.message);
                }
                Accepted::Verifying => self.watchdog.verify(now),
                Accepted::Status(line) => self.watchdog.status_line(&line, now),
                Accepted::Consumed => self.watchdog.output(now),
            },
            Pipe::Stderr => {
                if self.stderr_is_progress {
                    self.watchdog.output(now);
                }
                self.read_only |= names_read_only_repository(&text, &self.local_repositories);
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
            Ok(line) => line,
            Err(error) => {
                tracing::warn!(%error, "cannot build the restic command");
                return StepOutcome::not_started_because(&format!(
                    "cannot build the restic command: {error}"
                ));
            }
        };
        let step_id = step.step_id.as_str();
        let line = line.with_json();
        let scrubber = Scrubber::for_command(&line);
        log.push(
            step_id,
            LogStream::Agent,
            &command_line_text(&scrubber, &line),
        );
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
        match spawned {
            Ok(child) => cancel.adopt(child, step_id, true),
            Err(error) => {
                let detail = format!("{error:#}");
                tracing::warn!(%detail, "cannot start restic");
                log.push(
                    step_id,
                    LogStream::Agent,
                    &scrubber.scrub(&format!("restic did not start: {detail}")),
                );
                self.send_log(log).await;
                return StepOutcome::not_started_because(&scrubber.scrub(&detail));
            }
        }
        #[cfg(feature = "test-util")]
        self.panic_if_asked().await;
        // The secrets leave the memory of the agent with the command line (FR-AG-07).
        drop(line);

        let kind = step.kind.as_ref().map_or(OutputKind::Plain, OutputKind::of);
        let (stdout, stderr) = cancel.take_output();
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
            // A backup and a restore report their progress as status messages on the standard
            // output. A line on the standard error is a warning or a retry of an unreachable
            // backend, so it does not restart the stall timeout (ADR-0051, AC-AG-12).
            stderr_is_progress: !matches!(
                step.kind,
                Some(step::Kind::Backup(_) | step::Kind::Restore(_))
            ),
            last_progress: None,
            last_log: tokio::time::Instant::now(),
            local_repositories: validated
                .job()
                .repositories
                .iter()
                .map(|repository| repository.location.as_str())
                .filter(|location| crate::repository_roots::is_local(location))
                .map(clean_path)
                .collect(),
            read_only: false,
        };

        let started = tokio::time::Instant::now();
        let step_deadline = limits.max.map(|max| started + max);
        let mut limit_stop: Option<LimitStop> = None;
        let mut lines_open = true;
        // The stop is two stages: the graceful signal first, then the forced kill of the tree after
        // the grace period (FR-AG-10, AC-AG-07).
        let mut stopped_at: Option<tokio::time::Instant> = None;
        let mut hard_killed = false;
        // An `ls` page that is full needs no more of the listing, so the agent stops restic.
        let mut page_done = false;
        let (code, left_lock) = loop {
            let reaped = {
                let mut slot = lock(&cancel.child);
                match slot.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(Some(status)) => {
                            let code = status.code().unwrap_or(-1);
                            *slot = None;
                            Some((code, lock_possibly_left(&status)))
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
            if let Some(exit) = reaped {
                break exit;
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
            // A signal from outside kills restic before it can remove its lock (AC-SC-06).
            StepOutcome {
                possible_stale_lock: left_lock,
                ..StepOutcome::from_exit(code)
            }
        };
        if output.read_only
            && outcome.status == StepStatus::Failed
            && outcome.error == Some(StepError::Failed)
        {
            outcome.error = Some(StepError::ReadOnly);
        }
        outcome.output = output.collector.finish();
        // A step that stopped by its limit or a cancel keeps its links without a target, and the
        // second process never gets a budget of its own (FR-RS-02, ADR-0039).
        if let Some(step::Kind::Ls(ls)) = step.kind.as_ref()
            && let Some(step_result::Output::LsPage(page)) = outcome.output.as_mut()
            && has_symlink(page)
            && limit_stop.is_none()
            && !cancel.is_cancelled()
            && let Some(tree) = self
                .ls_tree(validated, step, cancel, limits, step_deadline, log)
                .await
        {
            fill_link_targets(page, &ls.path, &tree);
        }
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
}

/// True for a restic error line that reports `EROFS` on a path of a local repository (ADR-0045).
///
/// `repositories` holds paths in the form of [`clean_path`], the form that restic prints.
fn names_read_only_repository(line: &str, repositories: &[String]) -> bool {
    line.contains("read-only file system")
        && repositories.iter().any(|repository| {
            line.match_indices(repository.as_str()).any(|(at, _)| {
                matches!(
                    line[at + repository.len()..].chars().next(),
                    None | Some('/' | ':' | ' ' | '"' | '\'')
                )
            })
        })
}

/// A POSIX path without repeated and trailing separators. Another form stays as it is.
fn clean_path(location: &str) -> String {
    match components(PathFlavor::Posix, location) {
        Some(parts) => format!("/{}", parts.join("/")),
        None => location.to_owned(),
    }
}

/// The exit code of the child of a step, once it exited. `None` while it runs or when this process
/// started no child.
pub(super) fn reap(cancel: &CancelHandle) -> Option<i32> {
    let mut slot = lock(&cancel.child);
    let child = slot.as_mut()?;
    match child.try_wait() {
        Ok(Some(status)) => {
            *slot = None;
            Some(status.code().unwrap_or(-1))
        }
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(%error, "cannot wait for restic");
            *slot = None;
            Some(-1)
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

    use super::{
        LimitStop, REASON_PARAM, StepError, StepOutcome, clean_path, names_read_only_repository,
        status_for_exit,
    };

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

    #[test]
    fn fr_ag_09_a_step_that_did_not_start_sends_its_bounded_reason() {
        let plain = StepOutcome::not_started()
            .error_detail()
            .expect("a not-started step has an error");
        assert_eq!(plain.code, "step_not_started");
        assert!(plain.params.is_empty());

        let reason = format!(
            "cannot protect the directory C:\\ProgramData\\ResticHub {}",
            "x".repeat(1000)
        );
        let detail = StepOutcome::not_started_because(&reason)
            .error_detail()
            .expect("a not-started step has an error");
        assert_eq!(detail.code, "step_not_started");
        let [param] = detail.params.as_slice() else {
            panic!("one parameter: {:?}", detail.params);
        };
        assert_eq!(param.key, REASON_PARAM);
        assert_eq!(param.value.chars().count(), 500);
        assert!(param.value.starts_with("cannot protect the directory"));
        assert_eq!(StepOutcome::from_exit(0).error_detail(), None);
    }

    #[test]
    fn ac_rp_01_exit_code_10_names_the_missing_repository() {
        let outcome = StepOutcome::from_exit(10);
        assert_eq!(outcome.status, StepStatus::ConfigError);
        assert_eq!(
            outcome
                .error
                .map(StepError::detail)
                .map(|detail| detail.code),
            Some("repository_missing".to_owned())
        );
        assert_eq!(
            StepOutcome::from_exit(12).error,
            Some(StepError::ConfigError)
        );
    }

    #[test]
    fn adr_0045_a_read_only_error_on_a_local_repository_is_recognized() {
        let repositories = ["/var/spool/restichub/local-test".to_owned()];
        for line in [
            "Fatal: create repository at /var/spool/restichub/local-test failed: mkdir /var/spool/restichub: read-only file system",
            "Fatal: create repository at /var/spool/restichub/local-test failed: mkdir /var/spool/restichub/local-test/data: read-only file system",
        ] {
            assert!(names_read_only_repository(line, &repositories), "{line}");
        }
        assert!(!names_read_only_repository(
            "open /srv/restore/a.txt: read-only file system",
            &repositories
        ));
        assert!(!names_read_only_repository(
            "Fatal: unable to open config file: stat /var/spool/restichub/local-test/config: no such file or directory",
            &repositories
        ));
        assert!(!names_read_only_repository(
            "Fatal: create repository at /var/spool/restichub/local-test2 failed: mkdir /var/spool/restichub/local-test2: read-only file system",
            &repositories
        ));
    }

    #[test]
    fn adr_0045_a_read_only_error_matches_a_location_with_extra_separators() {
        for location in [
            "/var/spool/restichub/local-test/",
            "/var/spool//restichub/local-test",
            "/var/spool/restichub/local-test//",
        ] {
            let repositories = [clean_path(location)];
            assert!(
                names_read_only_repository(
                    "Fatal: create repository at /var/spool/restichub/local-test failed: mkdir /var/spool/restichub/local-test: read-only file system",
                    &repositories
                ),
                "{location}"
            );
        }
        assert_eq!(clean_path(r"C:\repo\"), r"C:\repo\");
    }
}
