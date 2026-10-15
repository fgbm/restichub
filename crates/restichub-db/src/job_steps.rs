//! The typed step results and the last progress of a job (FR-MN-01, FR-MN-02, FR-AG-09).
//!
//! The agent sends each step result live and again in the job result. A second write of the same
//! step changes no row, except that a summary replaces a missing one: the journal copy of a result
//! drops the large outputs. The summaries are typed. The backup summary has its own columns; the
//! other summaries are a jsonb value that this module reads back into a struct that rejects an
//! unknown field.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::DbError;
use crate::enums::StepStatus;
use crate::ids::JobId;
use crate::tx::TenantTx;

/// The most steps that the control plane stores for one job.
pub const MAX_STEPS_PER_JOB: i64 = 64;

/// The most files in the progress of a running step.
pub const MAX_CURRENT_FILES: usize = 5;

/// The summary of a `restic backup` run (FR-MN-02). The counts are never negative.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct BackupSummary {
    pub files_new: i64,
    pub files_changed: i64,
    pub files_unmodified: i64,
    pub dirs_new: i64,
    pub dirs_changed: i64,
    pub dirs_unmodified: i64,
    pub data_blobs: i64,
    pub tree_blobs: i64,
    /// Bytes added to the repository.
    pub data_added: i64,
    /// Bytes added to the repository, packed.
    pub data_added_packed: i64,
    pub total_files_processed: i64,
    pub total_bytes_processed: i64,
    pub total_duration_seconds: f64,
    /// Empty for a dry run.
    pub snapshot_id: String,
    pub dry_run: bool,
}

/// The snapshot count of a `restic snapshots` step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotsSummary {
    pub count: i64,
    /// True when the agent dropped snapshots to fit the size limit of a result.
    pub truncated: bool,
}

/// Counts that `restic forget` reported, summed over the groups.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ForgetSummary {
    pub groups: i64,
    pub kept: i64,
    pub removed: i64,
    /// True when the agent dropped groups to fit the size limit of a result.
    pub truncated: bool,
}

/// What `restic check --json` reported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckSummary {
    pub num_errors: i64,
    pub repair_suggested: bool,
    pub blobs_checked: i64,
    pub packs_checked: i64,
}

/// What `restic stats --json` reported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StatsSummary {
    pub total_size: i64,
    pub total_uncompressed_size: i64,
    pub total_file_count: i64,
    pub snapshots_count: i64,
    pub compression_ratio: f64,
}

/// What `restic restore --json` reported (FR-RS-04).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoreSummary {
    pub total_files: i64,
    pub files_restored: i64,
    pub files_skipped: i64,
    pub files_deleted: i64,
    pub total_bytes: i64,
    pub bytes_restored: i64,
    pub bytes_skipped: i64,
    pub seconds_elapsed: f64,
    /// True when the run was a dry run: restic wrote nothing.
    pub dry_run: bool,
}

/// The typed output of a step. A listing of files carries no stored summary.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepOutput {
    None,
    Backup(BackupSummary),
    Snapshots(SnapshotsSummary),
    Forget(ForgetSummary),
    Check(CheckSummary),
    Stats(StatsSummary),
    Restore(RestoreSummary),
    Ls,
}

impl StepOutput {
    fn kind(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Backup(_) => "backup",
            Self::Snapshots(_) => "snapshots",
            Self::Forget(_) => "forget",
            Self::Check(_) => "check",
            Self::Stats(_) => "stats",
            Self::Restore(_) => "restore",
            Self::Ls => "ls",
        }
    }

    fn summary_json(&self) -> Result<Option<serde_json::Value>, DbError> {
        let value = match self {
            Self::Snapshots(summary) => serde_json::to_value(summary),
            Self::Forget(summary) => serde_json::to_value(summary),
            Self::Check(summary) => serde_json::to_value(summary),
            Self::Stats(summary) => serde_json::to_value(summary),
            Self::Restore(summary) => serde_json::to_value(summary),
            Self::None | Self::Backup(_) | Self::Ls => return Ok(None),
        };
        value.map(Some).map_err(decode_error)
    }
}

/// One step result as the control plane stores it.
#[derive(Debug, Clone, PartialEq)]
pub struct JobStep {
    pub step_id: String,
    pub status: StepStatus,
    pub exit_code: i32,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
    /// Translated by the user interface (NFR-DOC-04). Never a secret.
    pub error_code: Option<String>,
    pub error_params: BTreeMap<String, String>,
    pub possible_stale_lock: bool,
    pub output: StepOutput,
}

/// The last progress of the running step of a job (FR-AG-08).
#[derive(Debug, Clone, PartialEq)]
pub struct JobProgress {
    pub step_id: String,
    pub bytes_done: i64,
    pub total_bytes: i64,
    pub files_done: i64,
    pub total_files: i64,
    /// From 0 to 1.
    pub percent_done: f64,
    pub seconds_elapsed: f64,
    pub seconds_remaining: i64,
    pub current_files: Vec<String>,
    pub updated_at: OffsetDateTime,
}

fn decode_error(error: impl std::error::Error + Send + Sync + 'static) -> DbError {
    DbError::Query(sqlx::Error::Decode(Box::new(error)))
}

#[derive(sqlx::FromRow)]
struct StepRow {
    step_id: String,
    status: StepStatus,
    exit_code: i32,
    started_at: Option<OffsetDateTime>,
    finished_at: Option<OffsetDateTime>,
    error_code: Option<String>,
    error_params: Option<Json<BTreeMap<String, String>>>,
    possible_stale_lock: bool,
    output_kind: String,
    files_new: Option<i64>,
    files_changed: Option<i64>,
    files_unmodified: Option<i64>,
    dirs_new: Option<i64>,
    dirs_changed: Option<i64>,
    dirs_unmodified: Option<i64>,
    data_blobs: Option<i64>,
    tree_blobs: Option<i64>,
    data_added: Option<i64>,
    data_added_packed: Option<i64>,
    total_files_processed: Option<i64>,
    total_bytes_processed: Option<i64>,
    total_duration_seconds: Option<f64>,
    snapshot_id: Option<String>,
    dry_run: Option<bool>,
    summary: Option<Json<serde_json::Value>>,
}

#[derive(Debug, thiserror::Error)]
#[error("the stored step output `{0}` is not valid")]
struct InvalidOutput(String);

impl StepRow {
    fn backup(&self) -> BackupSummary {
        BackupSummary {
            files_new: self.files_new.unwrap_or_default(),
            files_changed: self.files_changed.unwrap_or_default(),
            files_unmodified: self.files_unmodified.unwrap_or_default(),
            dirs_new: self.dirs_new.unwrap_or_default(),
            dirs_changed: self.dirs_changed.unwrap_or_default(),
            dirs_unmodified: self.dirs_unmodified.unwrap_or_default(),
            data_blobs: self.data_blobs.unwrap_or_default(),
            tree_blobs: self.tree_blobs.unwrap_or_default(),
            data_added: self.data_added.unwrap_or_default(),
            data_added_packed: self.data_added_packed.unwrap_or_default(),
            total_files_processed: self.total_files_processed.unwrap_or_default(),
            total_bytes_processed: self.total_bytes_processed.unwrap_or_default(),
            total_duration_seconds: self.total_duration_seconds.unwrap_or_default(),
            snapshot_id: self.snapshot_id.clone().unwrap_or_default(),
            dry_run: self.dry_run.unwrap_or_default(),
        }
    }

    fn into_step(self) -> Result<JobStep, DbError> {
        let summary = |row: &Self| {
            row.summary
                .as_ref()
                .map(|json| json.0.clone())
                .ok_or_else(|| decode_error(InvalidOutput(row.output_kind.clone())))
        };
        let output = match self.output_kind.as_str() {
            "none" => StepOutput::None,
            "ls" => StepOutput::Ls,
            "backup" => StepOutput::Backup(self.backup()),
            "snapshots" => StepOutput::Snapshots(
                serde_json::from_value(summary(&self)?).map_err(decode_error)?,
            ),
            "forget" => {
                StepOutput::Forget(serde_json::from_value(summary(&self)?).map_err(decode_error)?)
            }
            "check" => {
                StepOutput::Check(serde_json::from_value(summary(&self)?).map_err(decode_error)?)
            }
            "stats" => {
                StepOutput::Stats(serde_json::from_value(summary(&self)?).map_err(decode_error)?)
            }
            "restore" => {
                StepOutput::Restore(serde_json::from_value(summary(&self)?).map_err(decode_error)?)
            }
            other => return Err(decode_error(InvalidOutput(other.to_owned()))),
        };
        Ok(JobStep {
            step_id: self.step_id,
            status: self.status,
            exit_code: self.exit_code,
            started_at: self.started_at,
            finished_at: self.finished_at,
            error_code: self.error_code,
            error_params: self.error_params.map(|json| json.0).unwrap_or_default(),
            possible_stale_lock: self.possible_stale_lock,
            output,
        })
    }
}

macro_rules! step_columns {
    () => {
        "step_id, status, exit_code, started_at, finished_at, error_code, \
    error_params, possible_stale_lock, output_kind, files_new, files_changed, files_unmodified, \
    dirs_new, dirs_changed, dirs_unmodified, data_blobs, tree_blobs, data_added, \
    data_added_packed, total_files_processed, total_bytes_processed, total_duration_seconds, \
    snapshot_id, dry_run, summary"
    };
}

/// Stores one step result of a job.
///
/// A second write of a step keeps the first row. It only adds an output that the first row lacked.
/// Returns `false` when the job already holds [`MAX_STEPS_PER_JOB`] other steps.
pub async fn upsert(tx: &mut TenantTx<'_>, job: JobId, step: &JobStep) -> Result<bool, DbError> {
    // The row lock makes the count and the insert one step, so parallel writes keep the cap.
    sqlx::query("SELECT 1 FROM jobs WHERE id = $1 FOR UPDATE")
        .bind(job)
        .fetch_optional(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    let known: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM job_steps WHERE job_id = $1 AND step_id = $2)",
    )
    .bind(job)
    .bind(&step.step_id)
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    if !known {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM job_steps WHERE job_id = $1")
            .bind(job)
            .fetch_one(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
        if count >= MAX_STEPS_PER_JOB {
            return Ok(false);
        }
    }
    let backup = match &step.output {
        StepOutput::Backup(summary) => Some(summary),
        _ => None,
    };
    let error_params = Json(&step.error_params);
    let summary = step.output.summary_json()?;
    sqlx::query(
        "INSERT INTO job_steps \
         (tenant_id, job_id, step_id, status, exit_code, started_at, finished_at, error_code, \
          error_params, possible_stale_lock, output_kind, files_new, files_changed, \
          files_unmodified, dirs_new, dirs_changed, dirs_unmodified, data_blobs, tree_blobs, \
          data_added, data_added_packed, total_files_processed, total_bytes_processed, \
          total_duration_seconds, snapshot_id, dry_run, summary) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, \
                 $18, $19, $20, $21, $22, $23, $24, $25, $26, $27) \
         ON CONFLICT (tenant_id, job_id, step_id) DO UPDATE SET \
           output_kind = EXCLUDED.output_kind, files_new = EXCLUDED.files_new, \
           files_changed = EXCLUDED.files_changed, files_unmodified = EXCLUDED.files_unmodified, \
           dirs_new = EXCLUDED.dirs_new, dirs_changed = EXCLUDED.dirs_changed, \
           dirs_unmodified = EXCLUDED.dirs_unmodified, data_blobs = EXCLUDED.data_blobs, \
           tree_blobs = EXCLUDED.tree_blobs, data_added = EXCLUDED.data_added, \
           data_added_packed = EXCLUDED.data_added_packed, \
           total_files_processed = EXCLUDED.total_files_processed, \
           total_bytes_processed = EXCLUDED.total_bytes_processed, \
           total_duration_seconds = EXCLUDED.total_duration_seconds, \
           snapshot_id = EXCLUDED.snapshot_id, dry_run = EXCLUDED.dry_run, \
           summary = EXCLUDED.summary \
         WHERE job_steps.output_kind = 'none' AND EXCLUDED.output_kind <> 'none'",
    )
    .bind(tx.tenant())
    .bind(job)
    .bind(&step.step_id)
    .bind(step.status)
    .bind(step.exit_code)
    .bind(step.started_at)
    .bind(step.finished_at)
    .bind(step.error_code.as_deref())
    .bind(if step.error_params.is_empty() {
        None
    } else {
        Some(error_params)
    })
    .bind(step.possible_stale_lock)
    .bind(step.output.kind())
    .bind(backup.map(|b| b.files_new))
    .bind(backup.map(|b| b.files_changed))
    .bind(backup.map(|b| b.files_unmodified))
    .bind(backup.map(|b| b.dirs_new))
    .bind(backup.map(|b| b.dirs_changed))
    .bind(backup.map(|b| b.dirs_unmodified))
    .bind(backup.map(|b| b.data_blobs))
    .bind(backup.map(|b| b.tree_blobs))
    .bind(backup.map(|b| b.data_added))
    .bind(backup.map(|b| b.data_added_packed))
    .bind(backup.map(|b| b.total_files_processed))
    .bind(backup.map(|b| b.total_bytes_processed))
    .bind(backup.map(|b| b.total_duration_seconds))
    .bind(backup.map(|b| b.snapshot_id.as_str()))
    .bind(backup.map(|b| b.dry_run))
    .bind(summary.map(Json))
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(true)
}

/// Lists the steps of a job in the order in which they first arrived.
pub async fn list(tx: &mut TenantTx<'_>, job: JobId) -> Result<Vec<JobStep>, DbError> {
    let rows = sqlx::query_as::<_, StepRow>(concat!(
        "SELECT ",
        step_columns!(),
        " FROM job_steps WHERE job_id = $1 ORDER BY position"
    ))
    .bind(job)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    rows.into_iter().map(StepRow::into_step).collect()
}

/// One step of the list that the control plane signed into a job (FR-HK-11).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PlannedStep {
    pub step_id: String,
    /// The kind of the step, such as `backup`, `hook` or `script`. Empty for an older job.
    pub kind: String,
    /// The display name: the name of a script or the identifier of a hook. Empty for the others.
    pub name: String,
    /// The phase of a hook or a script, or empty.
    pub phase: String,
}

/// Stores the step list that the control plane signed into a job, in order.
///
/// A second call changes nothing. At most [`MAX_STEPS_PER_JOB`] steps are kept.
pub async fn plan(tx: &mut TenantTx<'_>, job: JobId, step_ids: &[String]) -> Result<(), DbError> {
    let steps: Vec<PlannedStep> = step_ids
        .iter()
        .map(|step_id| PlannedStep {
            step_id: step_id.clone(),
            kind: String::new(),
            name: String::new(),
            phase: String::new(),
        })
        .collect();
    plan_described(tx, job, &steps).await
}

/// Stores the described step list of a job, in order. Same rules as [`plan`].
pub async fn plan_described(
    tx: &mut TenantTx<'_>,
    job: JobId,
    steps: &[PlannedStep],
) -> Result<(), DbError> {
    for (position, step) in steps.iter().take(MAX_STEPS_PER_JOB as usize).enumerate() {
        sqlx::query(
            "INSERT INTO job_planned_steps (tenant_id, job_id, step_id, position, kind, name, phase) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT DO NOTHING",
        )
        .bind(tx.tenant())
        .bind(job)
        .bind(&step.step_id)
        .bind(i32::try_from(position).unwrap_or(i32::MAX))
        .bind(&step.kind)
        .bind(step.name.chars().take(128).collect::<String>())
        .bind(&step.phase)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    }
    Ok(())
}

/// Lists the step identifiers that the control plane signed into a job, in order.
pub async fn list_planned(tx: &mut TenantTx<'_>, job: JobId) -> Result<Vec<String>, DbError> {
    sqlx::query_scalar("SELECT step_id FROM job_planned_steps WHERE job_id = $1 ORDER BY position")
        .bind(job)
        .fetch_all(tx.as_conn())
        .await
        .map_err(DbError::Query)
}

/// Lists the described steps that the control plane signed into a job, in order.
pub async fn list_planned_described(
    tx: &mut TenantTx<'_>,
    job: JobId,
) -> Result<Vec<PlannedStep>, DbError> {
    sqlx::query_as::<_, PlannedStep>(
        "SELECT step_id, kind, name, phase FROM job_planned_steps WHERE job_id = $1 \
         ORDER BY position",
    )
    .bind(job)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// Stores the last progress of a job. The write replaces the previous progress.
pub async fn upsert_progress(
    tx: &mut TenantTx<'_>,
    job: JobId,
    progress: &JobProgress,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO job_progress \
         (tenant_id, job_id, step_id, bytes_done, total_bytes, files_done, total_files, \
          percent_done, seconds_elapsed, seconds_remaining, current_files, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) \
         ON CONFLICT (tenant_id, job_id) DO UPDATE SET \
           step_id = EXCLUDED.step_id, bytes_done = EXCLUDED.bytes_done, \
           total_bytes = EXCLUDED.total_bytes, files_done = EXCLUDED.files_done, \
           total_files = EXCLUDED.total_files, percent_done = EXCLUDED.percent_done, \
           seconds_elapsed = EXCLUDED.seconds_elapsed, \
           seconds_remaining = EXCLUDED.seconds_remaining, \
           current_files = EXCLUDED.current_files, updated_at = EXCLUDED.updated_at",
    )
    .bind(tx.tenant())
    .bind(job)
    .bind(&progress.step_id)
    .bind(progress.bytes_done)
    .bind(progress.total_bytes)
    .bind(progress.files_done)
    .bind(progress.total_files)
    .bind(progress.percent_done)
    .bind(progress.seconds_elapsed)
    .bind(progress.seconds_remaining)
    .bind(&progress.current_files)
    .bind(progress.updated_at)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct ProgressRow {
    step_id: String,
    bytes_done: i64,
    total_bytes: i64,
    files_done: i64,
    total_files: i64,
    percent_done: f64,
    seconds_elapsed: f64,
    seconds_remaining: i64,
    current_files: Vec<String>,
    updated_at: OffsetDateTime,
}

/// Reads the last progress of a job.
pub async fn get_progress(
    tx: &mut TenantTx<'_>,
    job: JobId,
) -> Result<Option<JobProgress>, DbError> {
    let row = sqlx::query_as::<_, ProgressRow>(
        "SELECT step_id, bytes_done, total_bytes, files_done, total_files, percent_done, \
                seconds_elapsed, seconds_remaining, current_files, updated_at \
         FROM job_progress WHERE job_id = $1",
    )
    .bind(job)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(|row| JobProgress {
        step_id: row.step_id,
        bytes_done: row.bytes_done,
        total_bytes: row.total_bytes,
        files_done: row.files_done,
        total_files: row.total_files,
        percent_done: row.percent_done,
        seconds_elapsed: row.seconds_elapsed,
        seconds_remaining: row.seconds_remaining,
        current_files: row.current_files,
        updated_at: row.updated_at,
    }))
}

/// Removes the progress of a job that ended.
pub async fn delete_progress(tx: &mut TenantTx<'_>, job: JobId) -> Result<(), DbError> {
    sqlx::query("DELETE FROM job_progress WHERE job_id = $1")
        .bind(job)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(())
}
