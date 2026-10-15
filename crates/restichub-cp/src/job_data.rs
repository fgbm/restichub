//! The progress, the step results and the log chunks that the agent streams (FR-AG-08, FR-AG-09,
//! FR-MN-01, FR-MN-02, FR-LG-01).
//!
//! The identity of the agent comes from the certificate, never from the message (FR-PR-08): a
//! message for a job of another agent or another tenant changes no row. Every value is bounded
//! before it reaches the database, because the message is untrusted input.
//!
//! The agent sends each step result live and again in the job result, and it sends the log tail
//! again after a restart (AC-PR-07). All writes are idempotent.

use std::collections::BTreeMap;

use restichub_db::job_logs::NewChunk;
use restichub_db::job_steps::{
    self, BackupSummary, CheckSummary, ForgetSummary, JobProgress, JobStep, MAX_CURRENT_FILES,
    RestoreSummary, SnapshotsSummary, StatsSummary, StepOutput,
};
use restichub_db::jobs::{self as job_repo, Job};
use restichub_db::{AgentId, JobId, JobStatus, LogStream, StepStatus, TenantId};
use restichub_proto::agent::v1::{
    ErrorDetail, JobResult, LogChunk, LogStream as ProtoLogStream, StepProgress, StepResult,
    StepStatus as ProtoStepStatus, step_result,
};
use time::OffsetDateTime;

use crate::browse;
use crate::jobs::parse_job_id_value;
use crate::log_store::{DatabaseLogStore, LogStore, MAX_CHUNK_BYTES, lines_of};
use crate::server::AppState;

const MAX_STEP_ID_LEN: usize = 64;
const MAX_ERROR_CODE_LEN: usize = 64;
const MAX_ERROR_PARAMS: usize = 8;
const MAX_PARAM_KEY_LEN: usize = 32;
const MAX_PARAM_VALUE_LEN: usize = 200;
const MAX_CURRENT_FILE_LEN: usize = 512;

/// Records the progress of a running step.
pub async fn handle_progress(
    state: &AppState,
    tenant: TenantId,
    agent: AgentId,
    progress: &StepProgress,
) {
    let Some(job_id) = parse_job_id_value(&progress.job_id) else {
        return;
    };
    if !valid_step_id(&progress.step_id) {
        return;
    }
    let now = state.auth().clock.now();
    let row = JobProgress {
        step_id: progress.step_id.clone(),
        bytes_done: count(progress.bytes_done),
        total_bytes: count(progress.total_bytes),
        files_done: count(progress.files_done),
        total_files: count(progress.total_files),
        percent_done: fraction(progress.percent_done),
        seconds_elapsed: non_negative(progress.seconds_elapsed),
        seconds_remaining: count(progress.seconds_remaining),
        current_files: progress
            .current_files
            .iter()
            .take(MAX_CURRENT_FILES)
            .map(|file| clean(file, MAX_CURRENT_FILE_LEN))
            .collect(),
        updated_at: now,
    };
    let cutoff = write_cutoff(state);
    let Ok(mut tx) = state.db().begin_tenant(tenant).await else {
        tracing::warn!(job = %job_id, "the progress could not open a transaction");
        return;
    };
    let result = async {
        let Some(job) = owned_job(&mut tx, job_id, agent, cutoff).await? else {
            return Ok(());
        };
        // A job that is not running shows no progress.
        if !matches!(job.status, JobStatus::Sent | JobStatus::Running) {
            return Ok(());
        }
        job_steps::upsert_progress(&mut tx, job_id, &row).await
    }
    .await;
    finish(tx, result, job_id, "progress").await;
    state.events().job_progress(tenant, job_id);
}

/// Records the result of one step.
pub async fn handle_step_result(
    state: &AppState,
    tenant: TenantId,
    agent: AgentId,
    result: &StepResult,
) {
    let Some(job_id) = parse_job_id_value(&result.job_id) else {
        return;
    };
    let Some(step) = step_of(result) else {
        return;
    };
    let cutoff = write_cutoff(state);
    let now = state.auth().clock.now();
    let Ok(mut tx) = state.db().begin_tenant(tenant).await else {
        tracing::warn!(job = %job_id, "the step result could not open a transaction");
        return;
    };
    let outcome = async {
        let Some(job) = owned_job(&mut tx, job_id, agent, cutoff).await? else {
            return Ok(());
        };
        job_steps::upsert(&mut tx, job_id, &step).await?;
        if step.status == StepStatus::Success {
            browse::store_listing(&mut tx, &job, result.output.as_ref(), now).await?;
        }
        Ok(())
    }
    .await;
    finish(tx, outcome, job_id, "step result").await;
    state.events().job_progress(tenant, job_id);
}

/// Why the control plane never stores a log chunk (FR-LG-07).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkRefusal {
    TooLarge,
    /// The chunk has no valid job, stream, step or sequence.
    Malformed,
    /// The job is not a job of this agent, or its log retention is over.
    UnknownJob,
}

/// What became of a log chunk (FR-LG-07).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkOutcome {
    Stored,
    /// The chunk will never be stored, so the agent may drop it.
    Refused(ChunkRefusal),
    /// The store failed for now. The agent has to send the chunk again.
    Failed,
}

/// Records one log chunk and tells the agent whether it may drop its copy (FR-LG-07).
pub async fn handle_log_chunk(
    state: &AppState,
    tenant: TenantId,
    agent: AgentId,
    chunk: &LogChunk,
) -> ChunkOutcome {
    let Some(job_id) = parse_job_id_value(&chunk.job_id) else {
        return ChunkOutcome::Refused(ChunkRefusal::Malformed);
    };
    let new_chunk = match chunk_of(chunk) {
        Ok(new_chunk) => new_chunk,
        Err(refusal) => {
            tracing::warn!(job = %job_id, ?refusal, "the log chunk is refused");
            return ChunkOutcome::Refused(refusal);
        }
    };
    match ownership(state, tenant, agent, job_id).await {
        Ownership::Owned => {}
        Ownership::Refused => return ChunkOutcome::Refused(ChunkRefusal::UnknownJob),
        Ownership::Retry => return ChunkOutcome::Failed,
    }
    let store = DatabaseLogStore::new(state.db().clone());
    match store
        .append(tenant, job_id, &new_chunk, state.auth().clock.now())
        .await
    {
        Ok(_) => {
            state.events().job_log(tenant, job_id);
            ChunkOutcome::Stored
        }
        Err(error) => {
            tracing::warn!(job = %job_id, %error, "the log chunk could not be stored");
            ChunkOutcome::Failed
        }
    }
}

/// Records the steps and the log tail of a job result, and removes the progress.
///
/// The control plane keeps them also when the job ends with a lock retry or a failure. The write
/// is idempotent: the live messages stored the same values before.
pub async fn handle_job_result_data(
    state: &AppState,
    tenant: TenantId,
    agent: AgentId,
    result: &JobResult,
) {
    let Some(job_id) = parse_job_id_value(&result.job_id) else {
        return;
    };
    if !job_is_owned(state, tenant, agent, job_id).await {
        return;
    }
    let Ok(mut tx) = state.db().begin_tenant(tenant).await else {
        tracing::warn!(job = %job_id, "the job result data could not open a transaction");
        return;
    };
    let outcome = async {
        for step in result.steps.iter().filter_map(step_of) {
            if !job_steps::upsert(&mut tx, job_id, &step).await? {
                break;
            }
        }
        job_steps::delete_progress(&mut tx, job_id).await
    }
    .await;
    finish(tx, outcome, job_id, "job result data").await;

    let store = DatabaseLogStore::new(state.db().clone());
    for chunk in result
        .log_tail
        .iter()
        .filter_map(|chunk| chunk_of(chunk).ok())
    {
        if let Err(error) = store
            .append(tenant, job_id, &chunk, state.auth().clock.now())
            .await
        {
            tracing::warn!(job = %job_id, %error, "the log tail could not be stored");
            break;
        }
    }
}

/// Whether a job takes writes from the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ownership {
    Owned,
    /// The job is unknown, belongs to another agent, or its retention is over. This stays so.
    Refused,
    /// The job may take writes later, or the database did not answer.
    Retry,
}

async fn job_is_owned(state: &AppState, tenant: TenantId, agent: AgentId, job: JobId) -> bool {
    ownership(state, tenant, agent, job).await == Ownership::Owned
}

async fn ownership(state: &AppState, tenant: TenantId, agent: AgentId, job: JobId) -> Ownership {
    let cutoff = write_cutoff(state);
    let Ok(mut tx) = state.db().begin_tenant(tenant).await else {
        return Ownership::Retry;
    };
    let found = job_repo::get(&mut tx, job).await;
    let _ = tx.commit().await;
    match found {
        Ok(row) => classify(row.as_ref(), agent, cutoff),
        Err(_) => Ownership::Retry,
    }
}

fn classify(row: Option<&Job>, agent: AgentId, cutoff: OffsetDateTime) -> Ownership {
    let Some(row) = row else {
        return Ownership::Refused;
    };
    if row.agent_id != agent || row.finished_at.is_some_and(|finished| finished < cutoff) {
        Ownership::Refused
    } else if row.status == JobStatus::Queued {
        Ownership::Retry
    } else {
        Ownership::Owned
    }
}

/// The earliest end time of a finished job that still takes writes: the log retention window.
///
/// After the window the sweep removes the log text, so a late write would only bring back what the
/// sweep deleted (FR-LG-02).
fn write_cutoff(state: &AppState) -> OffsetDateTime {
    state.auth().clock.now() - time::Duration::days(i64::from(state.log_retention_days()))
}

/// Reads the job when it belongs to the agent, has left the queue and takes writes.
async fn owned_job(
    tx: &mut restichub_db::TenantTx<'_>,
    job: JobId,
    agent: AgentId,
    cutoff: OffsetDateTime,
) -> Result<Option<Job>, restichub_db::DbError> {
    let found = job_repo::get(tx, job).await?;
    Ok(found.filter(|row| classify(Some(row), agent, cutoff) == Ownership::Owned))
}

async fn finish(
    tx: restichub_db::TenantTx<'_>,
    result: Result<(), restichub_db::DbError>,
    job: JobId,
    what: &str,
) {
    match result {
        Ok(()) => {
            if let Err(error) = tx.commit().await {
                tracing::warn!(%job, %error, "the {what} could not commit");
            }
        }
        Err(error) => tracing::warn!(%job, %error, "the {what} could not be stored"),
    }
}

fn chunk_of(chunk: &LogChunk) -> Result<NewChunk, ChunkRefusal> {
    if chunk.data.len() > MAX_CHUNK_BYTES {
        return Err(ChunkRefusal::TooLarge);
    }
    if chunk.sequence == 0 {
        return Err(ChunkRefusal::Malformed);
    }
    let stream = match ProtoLogStream::try_from(chunk.stream) {
        Ok(ProtoLogStream::Stdout) => LogStream::Stdout,
        Ok(ProtoLogStream::Stderr) => LogStream::Stderr,
        Ok(ProtoLogStream::Agent) => LogStream::Agent,
        Ok(ProtoLogStream::Unspecified) | Err(_) => return Err(ChunkRefusal::Malformed),
    };
    if chunk.step_id.chars().count() > MAX_STEP_ID_LEN {
        return Err(ChunkRefusal::Malformed);
    }
    Ok(NewChunk {
        step_id: clean(&chunk.step_id, MAX_STEP_ID_LEN),
        stream,
        chunk: count(chunk.sequence),
        lines: lines_of(&chunk.data),
    })
}

/// Converts a step result of the agent to the stored form. `None` for an invalid result.
fn step_of(result: &StepResult) -> Option<JobStep> {
    if !valid_step_id(&result.step_id) {
        return None;
    }
    let status = match ProtoStepStatus::try_from(result.status).ok()? {
        ProtoStepStatus::Success => StepStatus::Success,
        ProtoStepStatus::Warning => StepStatus::Warning,
        ProtoStepStatus::Retry => StepStatus::Retry,
        ProtoStepStatus::ConfigError => StepStatus::ConfigError,
        ProtoStepStatus::Cancelled => StepStatus::Cancelled,
        ProtoStepStatus::Failed => StepStatus::Failed,
        ProtoStepStatus::Skipped => StepStatus::Skipped,
        ProtoStepStatus::Unspecified => return None,
    };
    let (error_code, error_params) = match &result.error {
        Some(error) => error_of(error),
        None => (None, BTreeMap::new()),
    };
    Some(JobStep {
        step_id: result.step_id.clone(),
        status,
        exit_code: result.exit_code,
        started_at: result.started_at.as_ref().and_then(time_of),
        finished_at: result.finished_at.as_ref().and_then(time_of),
        error_code,
        error_params,
        possible_stale_lock: result.possible_stale_lock,
        output: output_of(result.output.as_ref()),
    })
}

fn error_of(error: &ErrorDetail) -> (Option<String>, BTreeMap<String, String>) {
    if error.code.is_empty() {
        return (None, BTreeMap::new());
    }
    let params = error
        .params
        .iter()
        .take(MAX_ERROR_PARAMS)
        .map(|param| {
            (
                clean(&param.key, MAX_PARAM_KEY_LEN),
                clean(&param.value, MAX_PARAM_VALUE_LEN),
            )
        })
        .collect();
    (Some(clean(&error.code, MAX_ERROR_CODE_LEN)), params)
}

fn output_of(output: Option<&step_result::Output>) -> StepOutput {
    match output {
        None => StepOutput::None,
        Some(step_result::Output::BackupSummary(summary)) => StepOutput::Backup(BackupSummary {
            files_new: count(summary.files_new),
            files_changed: count(summary.files_changed),
            files_unmodified: count(summary.files_unmodified),
            dirs_new: count(summary.dirs_new),
            dirs_changed: count(summary.dirs_changed),
            dirs_unmodified: count(summary.dirs_unmodified),
            data_blobs: count(summary.data_blobs),
            tree_blobs: count(summary.tree_blobs),
            data_added: count(summary.data_added),
            data_added_packed: count(summary.data_added_packed),
            total_files_processed: count(summary.total_files_processed),
            total_bytes_processed: count(summary.total_bytes_processed),
            total_duration_seconds: non_negative(summary.total_duration_seconds),
            snapshot_id: clean(&summary.snapshot_id, 64),
            dry_run: summary.dry_run,
        }),
        Some(step_result::Output::SnapshotList(list)) => StepOutput::Snapshots(SnapshotsSummary {
            count: i64::try_from(list.snapshots.len()).unwrap_or(i64::MAX),
            truncated: list.truncated,
        }),
        Some(step_result::Output::LsPage(_)) => StepOutput::Ls,
        Some(step_result::Output::RestoreSummary(restore)) => StepOutput::Restore(RestoreSummary {
            total_files: count(restore.total_files),
            files_restored: count(restore.files_restored),
            files_skipped: count(restore.files_skipped),
            files_deleted: count(restore.files_deleted),
            total_bytes: count(restore.total_bytes),
            bytes_restored: count(restore.bytes_restored),
            bytes_skipped: count(restore.bytes_skipped),
            seconds_elapsed: non_negative(restore.seconds_elapsed),
            dry_run: restore.dry_run,
        }),
        Some(step_result::Output::ForgetResult(forget)) => {
            let sum = |select: fn(&restichub_proto::agent::v1::ForgetGroup) -> usize| {
                i64::try_from(forget.groups.iter().map(select).sum::<usize>()).unwrap_or(i64::MAX)
            };
            StepOutput::Forget(ForgetSummary {
                groups: i64::try_from(forget.groups.len()).unwrap_or(i64::MAX),
                kept: sum(|group| group.kept.len()),
                removed: sum(|group| group.removed.len()),
                truncated: forget.truncated,
            })
        }
        Some(step_result::Output::CheckSummary(check)) => StepOutput::Check(CheckSummary {
            num_errors: count(check.num_errors),
            repair_suggested: check.repair_suggested,
            blobs_checked: count(check.blobs_checked),
            packs_checked: count(check.packs_checked),
        }),
        Some(step_result::Output::StatsSummary(stats)) => StepOutput::Stats(StatsSummary {
            total_size: count(stats.total_size),
            total_uncompressed_size: count(stats.total_uncompressed_size),
            total_file_count: count(stats.total_file_count),
            snapshots_count: count(stats.snapshots_count),
            compression_ratio: non_negative(stats.compression_ratio),
        }),
    }
}

pub(crate) fn time_of(timestamp: &prost_types::Timestamp) -> Option<OffsetDateTime> {
    let base = OffsetDateTime::from_unix_timestamp(timestamp.seconds).ok()?;
    let nanos = u32::try_from(timestamp.nanos)
        .ok()
        .filter(|n| *n < 1_000_000_000)?;
    base.replace_nanosecond(nanos).ok()
}

fn valid_step_id(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= MAX_STEP_ID_LEN
}

/// Clamps a counter into the signed range of the database.
fn count(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Maps NaN, infinity and negative values to zero.
fn non_negative(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

fn fraction(value: f64) -> f64 {
    non_negative(value).min(1.0)
}

/// Cuts a text at a character bound and removes the control characters.
pub(crate) fn clean(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_bounded_before_they_reach_the_database() {
        assert_eq!(count(u64::MAX), i64::MAX);
        assert_eq!(fraction(f64::NAN), 0.0);
        assert_eq!(fraction(7.0), 1.0);
        assert_eq!(non_negative(-1.0), 0.0);
        assert_eq!(clean("a\u{0}b\nc", 2), "ab");
    }

    fn log_chunk(sequence: u64, size: usize) -> LogChunk {
        LogChunk {
            job_id: "job".to_owned(),
            step_id: "backup".to_owned(),
            stream: ProtoLogStream::Stdout as i32,
            sequence,
            data: vec![b'a'; size],
        }
    }

    #[test]
    fn fr_lg_07_a_chunk_refusal_has_a_typed_reason() {
        assert!(chunk_of(&log_chunk(1, MAX_CHUNK_BYTES)).is_ok());
        assert_eq!(
            chunk_of(&log_chunk(1, MAX_CHUNK_BYTES + 1)).err(),
            Some(ChunkRefusal::TooLarge)
        );
        assert_eq!(
            chunk_of(&log_chunk(0, 1)).err(),
            Some(ChunkRefusal::Malformed)
        );
        let mut unknown_stream = log_chunk(1, 1);
        unknown_stream.stream = 0;
        assert_eq!(
            chunk_of(&unknown_stream).err(),
            Some(ChunkRefusal::Malformed)
        );
    }

    #[test]
    fn a_step_result_without_a_status_is_dropped() {
        let result = StepResult {
            step_id: "backup".to_owned(),
            ..StepResult::default()
        };
        assert!(step_of(&result).is_none());
    }
}
