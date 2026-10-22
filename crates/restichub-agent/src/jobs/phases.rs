//! The script steps of a job: the phases, the stop rules and the result (FR-HK-05, AC-HK-13,
//! ADR-0037).
//!
//! The agent derives the order from the phase of each script, not from its position in the job.
//! The sequence is: the `before` scripts, the other steps, the `after` scripts, then the
//! `on_success` scripts when nothing failed or the `on_error` scripts when something failed, and
//! the `finally` scripts last. A failed `before` script stops the sequence at once: the other
//! steps and the `after` and `on_success` scripts do not run, and the `on_error` and `finally`
//! scripts do. A script of any other phase that fails does not stop the scripts that follow, so a
//! cleanup always runs. A cancel stops the running script and runs the `finally` scripts only.
//!
//! Every script starts only when the host trusts it at that moment (AC-HK-07): the setting file,
//! the interpreter and the values are checked again before the process starts. A script that
//! cannot start fails like a script that exits with an error.

use std::sync::Arc;
use std::time::Duration;

use restichub_compiler::HookPhase;
use restichub_compiler::phase::phase_of;
use restichub_proto::agent::v1::{
    LogStream, ScriptFailurePolicy, ScriptStep, Step, StepResult, StepStatus, step,
};
use tokio::sync::mpsc;

use super::log::JobLog;
use super::runner::{
    CancelHandle, DRAIN_TIMEOUT, LINES_PER_TURN, LOG_FLUSH_BYTES, LOG_INTERVAL, POLL_INTERVAL,
    StepOutcome, interrupt, kill_tree,
};
use super::{Jobs, lock, now_timestamp};
use crate::exec::Invocation;
use crate::output::{Line, MAX_LINE, Pipe, spawn_reader};
use crate::scripts::ScriptRefusal;
use crate::scrub::Scrubber;

/// The most script output bytes that one script adds to the job log. The log of the job has its own
/// limit; this one stops a chatty script from using all of it (FR-AG-08).
const SCRIPT_OUTPUT_LIMIT: usize = 256 * 1024;

/// The steps of a job, grouped by the phase of the scripts (FR-HK-05).
pub(super) struct Phased<'a> {
    pub(super) before: Vec<&'a Step>,
    /// Every step that is not a script, in the order of the job.
    pub(super) main: Vec<&'a Step>,
    pub(super) after: Vec<&'a Step>,
    pub(super) on_success: Vec<&'a Step>,
    pub(super) on_error: Vec<&'a Step>,
    pub(super) finally: Vec<&'a Step>,
}

impl<'a> Phased<'a> {
    pub(super) fn new(steps: &'a [Step]) -> Self {
        let mut phased = Self {
            before: Vec::new(),
            main: Vec::new(),
            after: Vec::new(),
            on_success: Vec::new(),
            on_error: Vec::new(),
            finally: Vec::new(),
        };
        for step in steps {
            let phase = match step.kind.as_ref() {
                Some(step::Kind::Script(script)) => phase_of(script.phase),
                _ => None,
            };
            match phase {
                Some(HookPhase::Before) => phased.before.push(step),
                Some(HookPhase::After) => phased.after.push(step),
                Some(HookPhase::OnSuccess) => phased.on_success.push(step),
                Some(HookPhase::OnError) => phased.on_error.push(step),
                Some(HookPhase::Finally) => phased.finally.push(step),
                // The admission refuses a script without a phase, so this arm holds the other steps.
                None => phased.main.push(step),
            }
        }
        phased
    }
}

/// The first script that failed, for the error of the job (AC-HK-13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ScriptFailure {
    /// Shown in the job error.
    pub(super) script_name: String,
    pub(super) phase: HookPhase,
}

/// How one phase runs.
#[derive(Clone, Copy)]
pub(super) struct PhaseMode {
    /// Stop at the first failed script and skip the rest of the phase.
    pub(super) stop_on_failure: bool,
    /// Stop a running script when the job is cancelled.
    pub(super) honor_cancel: bool,
    /// Do not start a script after the deadline of the job.
    pub(super) honor_deadline: bool,
    /// A cleanup script runs after the deadline too: the deadline passed to the phase is the end of
    /// the cleanup budget and bounds the run time of the script without blocking its start.
    pub(super) clamp_to_deadline: bool,
}

/// What a phase did.
#[derive(Debug, Default)]
pub(super) struct PhaseReport {
    pub(super) failure: Option<ScriptFailure>,
    pub(super) cancelled: bool,
    /// A script with the policy `continue` failed: the job goes on and ends with warnings.
    pub(super) warned: bool,
}

/// A failed script with the policy `continue` does not stop the sequence and does not fail the
/// job (FR-HK-09).
fn tolerates_failure(script: &ScriptStep) -> bool {
    script.failure_policy == ScriptFailurePolicy::Continue as i32
}

/// Where the results of the steps of one job collect.
pub(super) struct Sink<'a> {
    pub(super) job_id: &'a str,
    pub(super) results: &'a mut Vec<StepResult>,
    pub(super) succeeded: &'a mut u32,
    pub(super) failed: &'a mut u32,
    pub(super) log: &'a mut JobLog,
}

impl Jobs {
    /// Executes the scripts of one phase one by one and reports the first failure (FR-HK-05).
    pub(super) async fn run_script_steps(
        &self,
        steps: &[&Step],
        mode: PhaseMode,
        cancel: &CancelHandle,
        deadline: Option<tokio::time::Instant>,
        sink: &mut Sink<'_>,
    ) -> PhaseReport {
        let mut report = PhaseReport::default();
        for (index, step) in steps.iter().enumerate() {
            let Some(step::Kind::Script(script)) = step.kind.as_ref() else {
                continue;
            };
            if mode.honor_cancel && cancel.is_cancelled() {
                report.cancelled = true;
                self.skip_steps(&steps[index..], sink).await;
                break;
            }
            let started_at = now_timestamp();
            let outcome = self
                .run_script(step, script, mode, cancel, deadline, sink.log)
                .await;
            let finished_at = now_timestamp();
            sink.results.push(
                self.send_step_result(sink.job_id, step, &outcome, started_at, finished_at)
                    .await,
            );
            match outcome.status {
                StepStatus::Success => *sink.succeeded += 1,
                StepStatus::Cancelled => {
                    report.cancelled = true;
                    self.skip_steps(&steps[index + 1..], sink).await;
                    break;
                }
                _ if tolerates_failure(script) => {
                    *sink.failed += 1;
                    report.warned = true;
                }
                _ => {
                    *sink.failed += 1;
                    if report.failure.is_none() {
                        report.failure = Some(ScriptFailure {
                            script_name: script.name.clone(),
                            phase: phase_of(script.phase).unwrap_or(HookPhase::Before),
                        });
                    }
                    if mode.stop_on_failure {
                        self.skip_steps(&steps[index + 1..], sink).await;
                        break;
                    }
                }
            }
        }
        report
    }

    /// Reports the steps that did not run, so the user sees why the sequence ended (AC-HK-13).
    pub(super) async fn skip_steps(&self, steps: &[&Step], sink: &mut Sink<'_>) {
        for step in steps {
            let now = now_timestamp();
            sink.results.push(
                self.send_step_result(sink.job_id, step, &StepOutcome::skipped(), now, now)
                    .await,
            );
        }
    }

    /// Writes one script, runs it and removes the file (FR-HK-09, AC-HK-09).
    async fn run_script(
        &self,
        step: &Step,
        script: &ScriptStep,
        mode: PhaseMode,
        cancel: &CancelHandle,
        deadline: Option<tokio::time::Instant>,
        log: &mut JobLog,
    ) -> StepOutcome {
        let step_id = step.step_id.as_str();
        let label = format!("script {}", script.name);
        let store = Arc::clone(&self.scripts);
        let request = script.clone();
        let timeout = step.timeout_seconds;
        let prepared = tokio::task::spawn_blocking(move || store.prepare(&request, timeout)).await;
        let prepared = match prepared {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(refusal)) => {
                log.push(
                    step_id,
                    LogStream::Agent,
                    &script_refusal_text(&label, &refusal),
                );
                self.send_log(log).await;
                return StepOutcome::not_started();
            }
            Err(error) => {
                tracing::warn!(%error, "the script check stopped");
                return StepOutcome::not_started();
            }
        };
        let crate::scripts::PreparedScript {
            invocation,
            scratch,
            dropped_path,
        } = prepared;
        if let Some(line) = dropped_path {
            log.note_search_path(step_id, &line);
        }
        let outcome = self
            .supervise(step, &label, invocation, mode, cancel, deadline, log)
            .await;
        // The process tree is gone, so the file can go (AC-HK-09).
        drop(scratch);
        outcome
    }

    /// Starts the process of an invocation, supervises it and returns its outcome (FR-HK-09,
    /// FR-AG-10).
    #[allow(clippy::too_many_arguments)]
    async fn supervise(
        &self,
        step: &Step,
        label: &str,
        invocation: Invocation,
        mode: PhaseMode,
        cancel: &CancelHandle,
        deadline: Option<tokio::time::Instant>,
        log: &mut JobLog,
    ) -> StepOutcome {
        let step_id = step.step_id.as_str();
        let mut limit = invocation.timeout;
        if step.timeout_seconds > 0 {
            limit = limit.min(Duration::from_secs(u64::from(step.timeout_seconds)));
        }
        if mode.honor_deadline
            && let Some(deadline) = deadline
        {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                log.push(
                    step_id,
                    LogStream::Agent,
                    &format!("{label} did not start: the job deadline passed"),
                );
                self.send_log(log).await;
                return StepOutcome::not_started();
            }
            limit = limit.min(remaining);
        } else if mode.clamp_to_deadline
            && let Some(deadline) = deadline
        {
            limit = limit.min(deadline.saturating_duration_since(tokio::time::Instant::now()));
        }
        let scrubber = Scrubber::for_secrets(&invocation.secrets);
        match crate::exec::spawn_job_script(&invocation) {
            Ok(child) => cancel.adopt(child, step_id, false),
            Err(error) => {
                tracing::warn!(step = %label, error = %format!("{error:#}"), "cannot start a script");
                log.push(step_id, LogStream::Agent, &format!("{label} did not start"));
                self.send_log(log).await;
                return StepOutcome::not_started();
            }
        }
        #[cfg(feature = "test-util")]
        assert!(!self.panic_after_spawn, "the job task panics on purpose");
        // The secret values leave the memory of the agent with the invocation (FR-HK-09).
        drop(invocation);

        let (stdout, stderr) = cancel.take_output();
        let (tx, mut lines) = mpsc::channel::<Line>(256);
        if let Some(stdout) = stdout {
            spawn_reader(stdout, Pipe::Stdout, MAX_LINE, tx.clone());
        }
        if let Some(stderr) = stderr {
            spawn_reader(stderr, Pipe::Stderr, MAX_LINE, tx.clone());
        }
        drop(tx);

        let mut output = ScriptOutput {
            step_id,
            scrubber,
            budget: SCRIPT_OUTPUT_LIMIT,
            truncated: false,
            last_flush: tokio::time::Instant::now(),
        };
        let started = tokio::time::Instant::now();
        let mut lines_open = true;
        let mut stopped_at: Option<tokio::time::Instant> = None;
        let mut hard_killed = false;
        let mut timed_out = false;
        let mut cancelled = false;
        let cancels_at_start = cancel.cancel_count();
        let mut cancels_at_stop = cancels_at_start;
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
                            tracing::warn!(%error, "cannot wait for a script");
                            drop(slot);
                            kill_tree(&cancel.child);
                            *lock(&cancel.child) = None;
                            return StepOutcome::failed();
                        }
                    },
                    None => None,
                }
            };
            if let Some(code) = reaped {
                break code;
            }
            // A cleanup step that ignores an earlier cancel still stops for one that arrives while
            // it runs.
            let cancel_seen = if mode.honor_cancel {
                cancel.is_cancelled()
            } else {
                cancel.cancel_count() > cancels_at_start
            };
            if stopped_at.is_none() {
                if cancel_seen {
                    cancelled = true;
                    cancels_at_stop = cancel.cancel_count();
                    interrupt(&cancel.child);
                    stopped_at = Some(tokio::time::Instant::now());
                } else if started.elapsed() >= limit {
                    timed_out = true;
                    interrupt(&cancel.child);
                    stopped_at = Some(tokio::time::Instant::now());
                }
            } else if !hard_killed
                && (stopped_at.is_some_and(|since| since.elapsed() >= self.stop_grace)
                    || cancel.cancel_count() > cancels_at_stop)
            {
                kill_tree(&cancel.child);
                hard_killed = true;
            }
            if lines_open {
                tokio::select! {
                    line = lines.recv() => match line {
                        Some(line) => {
                            output.accept(line, log);
                            for _ in 0..LINES_PER_TURN {
                                match lines.try_recv() {
                                    Ok(line) => output.accept(line, log),
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
            if log.pending_bytes() > 0
                && (log.pending_bytes() >= LOG_FLUSH_BYTES
                    || output.last_flush.elapsed() >= LOG_INTERVAL)
            {
                output.last_flush = tokio::time::Instant::now();
                self.send_log(log).await;
            }
        };

        // A grandchild can keep the pipes open, so the wait for the rest of the output is bounded.
        let drain = tokio::time::Instant::now() + DRAIN_TIMEOUT;
        while lines_open && let Ok(Some(line)) = tokio::time::timeout_at(drain, lines.recv()).await
        {
            output.accept(line, log);
        }
        log.push(
            step_id,
            LogStream::Agent,
            &format!("{label} finished with code {code}"),
        );
        self.send_log(log).await;

        if cancelled {
            StepOutcome::cancelled(false)
        } else if timed_out {
            StepOutcome::timed_out()
        } else if code == 0 {
            StepOutcome::new(StepStatus::Success, 0)
        } else {
            StepOutcome::new(StepStatus::Failed, code)
        }
    }
}

/// The script output of one run: scrubbed and bounded.
struct ScriptOutput<'a> {
    step_id: &'a str,
    scrubber: Scrubber,
    budget: usize,
    truncated: bool,
    last_flush: tokio::time::Instant,
}

impl ScriptOutput<'_> {
    fn accept(&mut self, line: Line, log: &mut JobLog) {
        let text = String::from_utf8_lossy(&line.bytes);
        let (stream, text) = match line.pipe {
            Pipe::Stdout => (LogStream::Stdout, text.into_owned()),
            Pipe::Stderr => (
                LogStream::Stderr,
                crate::output::stderr_text(&text, line.truncated),
            ),
        };
        let text = self.scrubber.scrub_line(&text, line.truncated);
        if self.truncated {
            return;
        }
        if text.len() + 1 > self.budget {
            self.truncated = true;
            log.push(
                self.step_id,
                LogStream::Agent,
                &format!(
                    "script output truncated: the limit of {SCRIPT_OUTPUT_LIMIT} bytes is reached"
                ),
            );
            return;
        }
        self.budget -= text.len() + 1;
        log.push(self.step_id, stream, &text);
    }
}

/// The agent line that tells why a script did not start. It holds no value.
fn script_refusal_text(label: &str, refusal: &ScriptRefusal) -> String {
    match refusal {
        ScriptRefusal::Disabled => {
            format!("{label} did not start: the host does not trust scripts from the control plane")
        }
        ScriptRefusal::Invalid(text) => format!("{label} did not start: {text}"),
        ScriptRefusal::Unavailable(_) => {
            format!("{label} did not start: the agent cannot write the script file")
        }
    }
}

/// The name of a phase in the job error and the log.
pub(super) fn phase_name(phase: HookPhase) -> &'static str {
    match phase {
        HookPhase::Before => "before",
        HookPhase::After => "after",
        HookPhase::OnSuccess => "on_success",
        HookPhase::OnError => "on_error",
        HookPhase::Finally => "finally",
    }
}
