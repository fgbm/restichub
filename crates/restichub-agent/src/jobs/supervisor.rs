//! The supervisor of a job task: it ends a job whose task panics without a result (FR-AG-09,
//! FR-AG-10, ADR-0020, ADR-0043).

use restichub_compiler::ValidatedJob;
use restichub_compiler::arg::build_command;
use restichub_proto::signing::JobJournal as _;

use super::outcome::task_failed_error;
use super::runner::{CancelHandle, MAX_REASON_CHARS, StepOutcome};
use super::{Jobs, JournalState, compact, failed_result, lock, now_timestamp};
use crate::scrub::Scrubber;

impl Jobs {
    /// Runs the job in its own task and ends the job when a panic kills that task.
    ///
    /// A panic in a dependency unwinds the task of the job. Without this wait the job would stay in
    /// `running` for good: it keeps a slot, and the agent lists it in each hello, so the control
    /// plane never fails it (ADR-0039, ADR-0043).
    pub(super) async fn run_supervised(&self, validated: ValidatedJob, cancel: CancelHandle) {
        let job_id = validated.job().job_id.clone();
        let steps_total = u32::try_from(validated.job().steps.len()).unwrap_or(u32::MAX);
        let jobs = self.clone();
        let task_cancel = cancel.clone();
        let task_job = validated.clone();
        let task = tokio::spawn(async move { jobs.execute(task_job, task_cancel).await });
        #[cfg(feature = "test-util")]
        if self.abort_job_task {
            task.abort();
        }
        let Err(error) = task.await else {
            return;
        };
        // Only the shutdown of the runtime cancels the task. A stop of the service or a reboot
        // raises no failure: the journal entry stays and the next start handles the job as an
        // orphan (ADR-0020, FR-IN-08).
        let Ok(payload) = error.try_into_panic() else {
            return;
        };
        tracing::error!(
            %job_id,
            cause = %scrubbed_panic(&validated, payload.as_ref()),
            "the task of a job panicked"
        );
        // The child belongs to the dead task, and nothing else would stop it (FR-AG-10).
        let orphan = cancel.stop_orphan().await;
        self.finish_log(&job_id);
        let stored = match lock(&self.journal).lookup(&job_id) {
            JournalState::Completed(result) => Some(result),
            _ => None,
        };
        if let Some(stored) = stored {
            // The task died after it stored the result, maybe before the disk write or the send.
            // Both repeat; the stored result stands (ADR-0020).
            self.finish(stored).await;
            return;
        }
        let mut result = failed_result(&job_id, steps_total, task_failed_error());
        if let Some(orphan) = orphan {
            // The killed restic may have left its lock; the control plane unlocks it (FR-AG-10).
            let outcome = StepOutcome {
                possible_stale_lock: orphan.possible_stale_lock,
                ..StepOutcome::failed()
            };
            let step = self
                .send_step_result_of(
                    &job_id,
                    &orphan.step_id,
                    &outcome,
                    orphan.started_at,
                    now_timestamp(),
                )
                .await;
            result.steps = vec![compact(&step)];
            result.steps_failed = 1;
        }
        self.finish(result).await;
    }
}

/// The panic text with the secrets of the job masked and a bounded length. A panic in the code that
/// handles a restic line can carry the line.
pub(super) fn scrubbed_panic(job: &ValidatedJob, payload: &(dyn std::any::Any + Send)) -> String {
    let mut text = panic_text(payload);
    for step in &job.job().steps {
        if let Ok(line) = build_command(job, step) {
            text = Scrubber::for_command(&line).scrub(&text);
        }
    }
    text.chars().take(MAX_REASON_CHARS).collect()
}

/// A panic carries a `&str` or a `String`. A payload of another type has no text.
fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic without a text".to_owned())
}
