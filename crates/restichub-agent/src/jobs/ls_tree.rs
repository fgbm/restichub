//! The target of the symbolic links of one `ls` page (FR-RS-02).
//!
//! `restic ls --json` names the type of a link but not its target. The tree object of the listed
//! directory holds it, so the agent reads that object one time for a page that names a link. The
//! read is best effort: a cancel, the rest of the budget of the step or a tree above the bound
//! leaves the targets empty, and the page still shows the links.

use restichub_compiler::ValidatedJob;
use restichub_compiler::arg::build_ls_tree_command;
use restichub_proto::agent::v1::{LogStream, Step};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::Jobs;
use super::command_text::command_line_text;
use super::log::JobLog;
use super::runner::{
    CancelHandle, DRAIN_TIMEOUT, POLL_INTERVAL, interrupt, kill_tree, priority_of, reap,
};
use super::watchdog::{StepLimits, Watchdog};
use crate::output::{DOCUMENT_QUEUE, Line, MAX_DOCUMENT, MAX_LINE, Pipe, spawn_reader};
use crate::scrub::Scrubber;

impl Jobs {
    /// Reads the tree object of the directory of an `ls` step.
    ///
    /// `deadline` is the end of the step, so the read never outlives the budget of the step, and
    /// the same stall timeout guards it. Gives `None` when restic does not start, exits with an
    /// error, the step is stopped or the object is above the bound.
    pub(super) async fn ls_tree(
        &self,
        validated: &ValidatedJob,
        step: &Step,
        cancel: &CancelHandle,
        limits: StepLimits,
        deadline: Option<Instant>,
        log: &mut JobLog,
    ) -> Option<Vec<u8>> {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return None;
        }
        let step_id = step.step_id.as_str();
        let line = match build_ls_tree_command(validated, step) {
            Ok(line) => line,
            Err(error) => {
                tracing::warn!(%error, "cannot build the restic command of the link targets");
                return None;
            }
        };
        let scrubber = Scrubber::for_command(&line);
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
                priority_of(step),
            )
        });
        drop(line);
        match spawned {
            Ok(child) => cancel.adopt(child, step_id, true),
            Err(error) => {
                let detail = format!("{error:#}");
                tracing::warn!(%detail, "cannot start restic for the link targets");
                log.push(
                    step_id,
                    LogStream::Agent,
                    &scrubber.scrub(&format!("restic did not start: {detail}")),
                );
                return None;
            }
        }
        let (stdout, stderr) = cancel.take_output();
        let (tx, mut lines) = mpsc::channel::<Line>(DOCUMENT_QUEUE);
        if let Some(stdout) = stdout {
            spawn_reader(stdout, Pipe::Stdout, MAX_DOCUMENT, tx.clone());
        }
        if let Some(stderr) = stderr {
            spawn_reader(stderr, Pipe::Stderr, MAX_LINE, tx.clone());
        }
        drop(tx);

        let started = Instant::now();
        let mut watchdog = Watchdog::new(limits.stall, started);
        let mut document: Vec<u8> = Vec::new();
        let mut stopped_at: Option<Instant> = None;
        let mut hard_killed = false;
        let mut lines_open = true;
        let code = loop {
            let now = Instant::now();
            if let Some(code) = reap(cancel) {
                break code;
            }
            if stopped_at.is_none() {
                if cancel.is_cancelled()
                    || deadline.is_some_and(|deadline| now >= deadline)
                    || watchdog.is_stalled(now)
                {
                    interrupt(&cancel.child);
                    stopped_at = Some(now);
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
            let Some(line) = received else {
                lines_open = false;
                continue;
            };
            watchdog.output(now);
            if line.pipe == Pipe::Stderr {
                log_stderr(&line, &scrubber, log, step_id);
                continue;
            }
            if stopped_at.is_some() {
                continue;
            }
            if line.truncated || document.len() + line.bytes.len() + 1 > MAX_DOCUMENT {
                // A directory above the bound: the agent stops restic and reads no target.
                document.clear();
                interrupt(&cancel.child);
                stopped_at = Some(Instant::now());
                continue;
            }
            document.extend_from_slice(&line.bytes);
            document.push(b'\n');
        };
        let drain = Instant::now() + DRAIN_TIMEOUT;
        while lines_open && let Ok(Some(line)) = tokio::time::timeout_at(drain, lines.recv()).await
        {
            if line.pipe == Pipe::Stderr {
                log_stderr(&line, &scrubber, log, step_id);
            } else if stopped_at.is_none()
                && !line.truncated
                && document.len() + line.bytes.len() < MAX_DOCUMENT
            {
                // A restic that exits fast leaves its object in the pipe after the exit.
                document.extend_from_slice(&line.bytes);
                document.push(b'\n');
            }
        }
        (code == 0 && stopped_at.is_none() && !document.is_empty()).then_some(document)
    }
}

fn log_stderr(line: &Line, scrubber: &Scrubber, log: &mut JobLog, step_id: &str) {
    let text = String::from_utf8_lossy(&line.bytes);
    let text = crate::output::stderr_text(&text, line.truncated);
    log.push(
        step_id,
        LogStream::Stderr,
        &scrubber.scrub_line(&text, line.truncated),
    );
}
