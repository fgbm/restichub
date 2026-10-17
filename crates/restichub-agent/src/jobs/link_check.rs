//! The link check of a restore target before restic writes (FR-RS-03, threat T31).
//!
//! The agent lists the snapshot with `restic ls` and checks the paths that the restore can write
//! ([`crate::restore_links`]). The check runs under the cancel and the maximum duration of the
//! step (FR-AG-10, ADR-0039).

use std::path::PathBuf;
use std::time::Duration;

use restichub_compiler::ValidatedJob;
use restichub_compiler::arg::build_command;
use restichub_proto::agent::v1::{LogStream, LsStep, ProcessPriority, Step, step};
use restichub_proto::restore_path::PathFlavor;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::command_text::command_line_text;
use super::log::JobLog;
use super::runner::{
    CancelHandle, DRAIN_TIMEOUT, LINES_PER_TURN, LimitStop, POLL_INTERVAL, StepOutcome, interrupt,
    kill_tree,
};
use super::{Jobs, lock};
use crate::output::{Line, MAX_LINE, Pipe, spawn_reader};
use crate::restore_links::{Selection, TargetCheck};
use crate::scrub::Scrubber;

/// The most snapshot nodes that one check takes. A larger restore needs narrower includes or an
/// empty target.
const MAX_CHECKED_NODES: usize = 1_000_000;

/// The nodes that one blocking turn of the check takes.
const BATCH: usize = 1024;

#[derive(serde::Deserialize)]
struct ListedNode {
    #[serde(default)]
    message_type: String,
    #[serde(default)]
    struct_type: String,
    #[serde(default)]
    path: String,
}

/// How a blocking part of the check ended.
enum Waited<T> {
    Done(T),
    Cancelled,
    TimedOut,
}

/// Why the check stopped the listing before its end.
enum Stop {
    Cancelled,
    TimedOut,
    Refused(String),
}

impl Jobs {
    /// Runs the link check of a restore step, or gives `None` when restic may start.
    ///
    /// `max` is the maximum duration of the step. Only Windows needs the check: restic on Linux
    /// replaces an existing link with the node of the snapshot.
    pub(super) async fn restore_link_check(
        &self,
        validated: &ValidatedJob,
        step: &Step,
        cancel: &CancelHandle,
        max: Option<Duration>,
        log: &mut JobLog,
    ) -> Option<StepOutcome> {
        let Some(step::Kind::Restore(restore)) = step.kind.as_ref() else {
            return None;
        };
        // Nothing is written in a dry run, so no link can redirect it.
        if !self.restore_link_rule || restore.dry_run {
            return None;
        }
        let deadline = max.map(|max| Instant::now() + max);
        let target = PathBuf::from(&restore.target);
        let opened = match wait_blocking(
            tokio::task::spawn_blocking(move || TargetCheck::open(&target)),
            cancel,
            deadline,
        )
        .await
        {
            Waited::Done(Ok(Ok(opened))) => opened,
            Waited::Done(Ok(Err(error))) => return Some(cannot_check(&format!("{error:#}"))),
            Waited::Done(Err(error)) => return Some(cannot_check(&error.to_string())),
            Waited::Cancelled => return Some(StepOutcome::cancelled(false)),
            Waited::TimedOut => {
                return Some(StepOutcome::limit_exceeded(LimitStop::MaxDuration, false));
            }
        };
        let check = opened?;
        let ls = Step {
            step_id: step.step_id.clone(),
            kind: Some(step::Kind::Ls(LsStep {
                snapshot_id: restore.snapshot_id.clone(),
                path: String::new(),
                recursive: true,
                repository_id: restore.repository_id.clone(),
                offset: 0,
                limit: 0,
            })),
            ..Step::default()
        };
        let selection = Selection::new(&restore.includes, &restore.excludes, PathFlavor::host());
        self.list_and_check(validated, &ls, selection, check, cancel, deadline, log)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn list_and_check(
        &self,
        validated: &ValidatedJob,
        ls: &Step,
        selection: Selection,
        check: TargetCheck,
        cancel: &CancelHandle,
        deadline: Option<Instant>,
        log: &mut JobLog,
    ) -> Option<StepOutcome> {
        let line = match build_command(validated, ls) {
            Ok(line) => line.with_json(),
            Err(error) => return Some(cannot_check(&error.to_string())),
        };
        let step_id = ls.step_id.as_str();
        let scrubber = Scrubber::for_command(&line);
        log.push(
            step_id,
            LogStream::Agent,
            "checking the paths of the restore target for links",
        );
        log.push(
            step_id,
            LogStream::Agent,
            &command_line_text(&scrubber, &line),
        );
        let spawned = self.expected_sha256().and_then(|expected| {
            crate::exec::spawn_job_restic(
                &self.program,
                &expected,
                &line,
                &self.cache,
                ProcessPriority::Unspecified,
            )
        });
        drop(line);
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => return Some(cannot_check(&scrubber.scrub(&format!("{error:#}")))),
        };
        let (stdout, stderr) = child.take_output();
        let (tx, mut lines) = mpsc::channel::<Line>(64);
        if let Some(stdout) = stdout {
            spawn_reader(stdout, Pipe::Stdout, MAX_LINE, tx.clone());
        }
        if let Some(stderr) = stderr {
            spawn_reader(stderr, Pipe::Stderr, MAX_LINE, tx.clone());
        }
        drop(tx);
        *lock(&cancel.child) = Some(child);

        let mut nodes = Nodes {
            selection,
            check: Some(check),
            batch: Vec::new(),
            taken: 0,
        };
        let mut stop: Option<Stop> = None;
        let mut stopped_at: Option<Instant> = None;
        let mut hard_killed = false;
        let mut lines_open = true;
        let code = loop {
            if let Some(code) = reap(cancel) {
                break code;
            }
            if stop.is_none() {
                if cancel.is_cancelled() {
                    stop = Some(Stop::Cancelled);
                } else if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    stop = Some(Stop::TimedOut);
                }
                if stop.is_some() {
                    interrupt(&cancel.child);
                    stopped_at = Some(Instant::now());
                }
            } else if !hard_killed
                && stopped_at.is_some_and(|since| since.elapsed() >= self.stop_grace)
            {
                kill_tree(&cancel.child);
                hard_killed = true;
            }
            if !lines_open {
                tokio::time::sleep(POLL_INTERVAL).await;
                continue;
            }
            let received = tokio::select! {
                line = lines.recv() => line,
                () = tokio::time::sleep(POLL_INTERVAL) => continue,
            };
            let Some(first) = received else {
                lines_open = false;
                continue;
            };
            if stop.is_some() {
                continue;
            }
            let mut turn = vec![first];
            while turn.len() < LINES_PER_TURN
                && let Ok(line) = lines.try_recv()
            {
                turn.push(line);
            }
            for line in turn {
                if let Some(found) = nodes.line(&line, &scrubber, log, step_id) {
                    stop = Some(found);
                    break;
                }
            }
            if stop.is_none() && nodes.batch.len() >= BATCH {
                stop = nodes.flush(cancel, deadline).await;
            }
            if stop.is_some() {
                interrupt(&cancel.child);
                stopped_at = Some(Instant::now());
            }
        };
        let drain = Instant::now() + DRAIN_TIMEOUT;
        while lines_open {
            match tokio::time::timeout_at(drain, lines.recv()).await {
                Ok(Some(line)) => {
                    if stop.is_none()
                        && let Some(found) = nodes.line(&line, &scrubber, log, step_id)
                    {
                        stop = Some(found);
                    }
                }
                Ok(None) => lines_open = false,
                Err(_) => break,
            }
        }
        // Unread nodes were never checked, so restic must not start.
        if stop.is_none() && lines_open {
            stop = Some(Stop::Refused(cannot_check_text(
                "the listing of the snapshot did not end",
            )));
        }
        if stop.is_none() && code == 0 {
            stop = nodes.flush(cancel, deadline).await;
        }
        Some(match stop {
            Some(Stop::Cancelled) => StepOutcome::cancelled(hard_killed),
            Some(Stop::TimedOut) => {
                StepOutcome::limit_exceeded(LimitStop::MaxDuration, hard_killed)
            }
            Some(Stop::Refused(reason)) => {
                log.push(step_id, LogStream::Agent, &reason);
                StepOutcome::not_started_because(&reason)
            }
            None if code == 0 => return None,
            // The codes that name the repository or its lock keep their meaning (AC-AG-05).
            None if matches!(code, 10..=12) => {
                log.push(
                    step_id,
                    LogStream::Agent,
                    &cannot_check_text(&format!("restic ls exited with the code {code}")),
                );
                StepOutcome::from_exit(code)
            }
            None => cannot_check(&format!("restic ls exited with the code {code}")),
        })
    }
}

/// What the listing gave so far, and the state of the check.
struct Nodes {
    selection: Selection,
    /// `None` after a blocking turn that the cancel or the deadline left behind.
    check: Option<TargetCheck>,
    batch: Vec<String>,
    taken: usize,
}

impl Nodes {
    fn line(
        &mut self,
        line: &Line,
        scrubber: &Scrubber,
        log: &mut JobLog,
        step_id: &str,
    ) -> Option<Stop> {
        let text = String::from_utf8_lossy(&line.bytes);
        if line.pipe == Pipe::Stderr {
            let text = crate::output::stderr_text(&text, line.truncated);
            log.push(
                step_id,
                LogStream::Stderr,
                &scrubber.scrub_line(&text, line.truncated),
            );
            return None;
        }
        if line.truncated {
            return Some(Stop::Refused(
                "a path of the snapshot is too long for the link check of the restore target"
                    .to_owned(),
            ));
        }
        let node: ListedNode = serde_json::from_str(&text).ok()?;
        if node.message_type != "node" && node.struct_type != "node" {
            return None;
        }
        let parts: Vec<&str> = node
            .path
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        if parts.is_empty() || !self.selection.writes(&parts) {
            return None;
        }
        self.taken += 1;
        if self.taken > MAX_CHECKED_NODES {
            return Some(Stop::Refused(format!(
                "the restore writes more than {MAX_CHECKED_NODES} entries into a target that is not empty, too many for the link check; narrow the includes or restore into an empty directory"
            )));
        }
        self.batch.push(node.path);
        None
    }

    async fn flush(&mut self, cancel: &CancelHandle, deadline: Option<Instant>) -> Option<Stop> {
        let mut check = self.check.take()?;
        let batch = std::mem::take(&mut self.batch);
        let task = tokio::task::spawn_blocking(move || {
            let mut found = Ok(None);
            for path in &batch {
                let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
                found = check.node(&parts);
                if !matches!(found, Ok(None)) {
                    break;
                }
            }
            (check, found)
        });
        match wait_blocking(task, cancel, deadline).await {
            Waited::Done(Ok((check, found))) => {
                self.check = Some(check);
                match found {
                    Ok(None) => None,
                    Ok(Some(refusal)) => Some(Stop::Refused(refusal.to_string())),
                    Err(error) => Some(Stop::Refused(cannot_check_text(&format!("{error:#}")))),
                }
            }
            Waited::Done(Err(error)) => Some(Stop::Refused(cannot_check_text(&error.to_string()))),
            Waited::Cancelled => Some(Stop::Cancelled),
            Waited::TimedOut => Some(Stop::TimedOut),
        }
    }
}

fn cannot_check_text(detail: &str) -> String {
    format!("cannot check the links in the restore target: {detail}")
}

fn cannot_check(detail: &str) -> StepOutcome {
    StepOutcome::not_started_because(&cannot_check_text(detail))
}

/// The exit code of the listing child, once it exited.
fn reap(cancel: &CancelHandle) -> Option<i32> {
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

/// Waits for a blocking task, but no longer than the cancel and the deadline allow. A file system
/// that hangs keeps its thread, not the step.
async fn wait_blocking<T>(
    mut task: JoinHandle<T>,
    cancel: &CancelHandle,
    deadline: Option<Instant>,
) -> Waited<Result<T, tokio::task::JoinError>> {
    loop {
        tokio::select! {
            result = &mut task => return Waited::Done(result),
            () = tokio::time::sleep(POLL_INTERVAL) => {
                if cancel.is_cancelled() {
                    return Waited::Cancelled;
                }
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return Waited::TimedOut;
                }
            }
        }
    }
}
