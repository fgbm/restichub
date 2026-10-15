//! The job detail and the job log endpoints (FR-MN-02, FR-LG-01, AC-MN-04).
//!
//! The detail holds the typed steps, the summary of the job and the progress of a running job. The
//! log endpoint pages through the stored lines with a line number as the keyset. Both endpoints
//! need the Viewer role. A job of another tenant gives HTTP 404 (AC-TN-01).

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use restichub_db::job_logs;
use restichub_db::job_steps::{self, JobProgress, JobStep, StepOutput};
use restichub_db::jobs as job_repo;
use restichub_db::{
    JobKind, JobStatus, LogLevel, LogStream, PlannedKind, PlannedPhase, StepStatus,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::jobs::{JobView, format_time, parse_job_id};
use crate::log_store::{DatabaseLogStore, LogStore};
use crate::server::AppState;
use crate::validation::Reason;

/// The lines of a log page when the request names no limit.
const DEFAULT_LOG_LIMIT: i64 = 500;

/// One step of a job with its typed result.
#[derive(Debug, Serialize, ToSchema)]
pub struct StepView {
    pub step_id: String,
    /// Absent for a job from an older version.
    pub kind: Option<PlannedKind>,
    /// The name of a script or the identifier of a hook, or empty.
    pub name: String,
    pub phase: Option<PlannedPhase>,
    pub status: StepStatus,
    /// The exit code of restic, or -1 when no process ran.
    pub exit_code: i32,
    /// In RFC 3339.
    pub started_at: Option<String>,
    /// In RFC 3339.
    pub finished_at: Option<String>,
    /// Translated by the user interface (NFR-DOC-04).
    pub error_code: Option<String>,
    pub error_params: BTreeMap<String, String>,
    /// True after a hard kill that can leave a stale lock.
    pub possible_stale_lock: bool,
    pub output: StepOutput,
}

impl StepView {
    fn new(step: &JobStep, planned: &[job_steps::PlannedStep]) -> Self {
        let described = planned.iter().find(|entry| entry.step_id == step.step_id);
        let mut view = Self::from(step);
        if let Some(entry) = described {
            view.kind = entry.kind;
            view.name.clone_from(&entry.name);
            view.phase = entry.phase;
        }
        view
    }
}

impl From<&JobStep> for StepView {
    fn from(step: &JobStep) -> Self {
        Self {
            step_id: step.step_id.clone(),
            kind: None,
            name: String::new(),
            phase: None,
            status: step.status,
            exit_code: step.exit_code,
            started_at: step.started_at.map(format_time),
            finished_at: step.finished_at.map(format_time),
            error_code: step.error_code.clone(),
            error_params: step.error_params.clone(),
            possible_stale_lock: step.possible_stale_lock,
            output: step.output.clone(),
        }
    }
}

/// One planned step of a job (FR-HK-11).
#[derive(Debug, Serialize, ToSchema)]
pub struct PlannedStepView {
    pub step_id: String,
    /// Absent for a job from an older version.
    pub kind: Option<PlannedKind>,
    /// The name of a script or the identifier of a hook, or empty.
    pub name: String,
    pub phase: Option<PlannedPhase>,
    /// The SHA-256 hash of the script body, in lowercase hex. Present for a script step only.
    pub content_sha256: Option<String>,
}

/// Last progress of the running step. It holds no value once the job ends.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProgressView {
    pub step_id: String,
    pub bytes_done: i64,
    pub total_bytes: i64,
    pub files_done: i64,
    pub total_files: i64,
    /// From 0 to 1.
    pub percent_done: f64,
    pub seconds_elapsed: f64,
    /// An estimate. Zero when restic gives none.
    pub seconds_remaining: i64,
    pub current_files: Vec<String>,
    /// In RFC 3339.
    pub updated_at: String,
}

impl From<&JobProgress> for ProgressView {
    fn from(progress: &JobProgress) -> Self {
        Self {
            step_id: progress.step_id.clone(),
            bytes_done: progress.bytes_done,
            total_bytes: progress.total_bytes,
            files_done: progress.files_done,
            total_files: progress.total_files,
            percent_done: progress.percent_done,
            seconds_elapsed: progress.seconds_elapsed,
            seconds_remaining: progress.seconds_remaining,
            current_files: progress.current_files.clone(),
            updated_at: format_time(progress.updated_at),
        }
    }
}

/// Size and fate of the stored log text.
#[derive(Debug, Serialize, ToSchema)]
pub struct LogInfo {
    pub lines: i64,
    /// True when the control plane refused lines because the log reached its bound.
    pub truncated: bool,
    /// True when the retention sweep deleted the lines.
    pub purged: bool,
}

/// What started a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobTrigger {
    Schedule,
    Manual,
    /// The control plane started a check, a forget or a prune with no fire time and no user.
    Maintenance,
}

impl JobTrigger {
    fn of(job: &restichub_db::jobs::Job) -> Self {
        let maintenance_kind =
            matches!(job.kind, JobKind::Check | JobKind::Forget | JobKind::Prune);
        if job.fire_time.is_some() {
            Self::Schedule
        } else if maintenance_kind && job.created_by.is_none() && job.plan_id.is_none() {
            Self::Maintenance
        } else {
            Self::Manual
        }
    }
}

/// One job with its steps, its summary and its progress (FR-MN-02).
#[derive(Debug, Serialize, ToSchema)]
pub struct JobDetailView {
    #[serde(flatten)]
    pub job: JobView,
    pub trigger: JobTrigger,
    /// The identifiers of all steps that the control plane signed into the job, in run order. Empty
    /// before the control plane sent the job. A step that did not start has no entry in `steps`.
    pub planned_steps: Vec<String>,
    /// The same steps with their kind, name and phase, so the job detail shows every script and
    /// hook in order before it runs (FR-HK-11).
    pub planned: Vec<PlannedStepView>,
    /// Ordered by first arrival from the agent.
    pub steps: Vec<StepView>,
    /// Taken from the first step that carries a typed output. Null when no step has one.
    pub summary: Option<StepOutput>,
    /// Null unless the job is sent or running and the agent reported progress.
    pub progress: Option<ProgressView>,
    pub log: LogInfo,
    pub limits: LimitsView,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LimitsView {
    /// Seconds after which the agent stops the job whatever its progress. Null means no limit.
    pub max_duration_seconds: Option<u32>,
    /// Seconds without progress after which the agent stops a step. Null when the control plane
    /// did not set one and the agent uses the default of the step.
    pub stall_timeout_seconds: Option<u32>,
}

/// Reads one job of the tenant with its steps, summary and progress.
#[utoipa::path(
    get,
    path = "/api/v1/jobs/{id}",
    summary = "Reads one job with its steps, summary and progress.",
    description = "A running job holds the last progress of its step. A finished job holds the typed result of each step and the summary of the run.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the job.")),
    responses(
        (status = 200, description = "The job.", body = JobDetailView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The job is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_job(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<JobDetailView>, ApiError> {
    context.authorize(Operation::JobsRead)?;
    let job_id = parse_job_id(&id)?;
    let now = state.auth().clock.now();
    let found = context
        .tenant_tx(&state, async |tx| {
            // An overdue job moves to `expired` on read, so a poll sees a terminal status.
            job_repo::mark_expired(tx, job_id, now).await?;
            let Some(detail) = restichub_db::queue::get_detail(tx, job_id).await? else {
                return Ok(None);
            };
            let steps = job_steps::list(tx, job_id).await?;
            let planned = job_steps::list_planned_described(tx, job_id).await?;
            let progress = job_steps::get_progress(tx, job_id).await?;
            let log = job_logs::state(tx, job_id).await?;
            Ok(Some((detail, steps, planned, progress, log)))
        })
        .await?;
    let Some((detail, steps, planned, progress, log)) = found else {
        return Err(ApiError::not_found());
    };
    let summary = steps
        .iter()
        .map(|step| &step.output)
        .find(|output| !matches!(output, StepOutput::None | StepOutput::Ls))
        .cloned();
    let running = matches!(detail.job.status, JobStatus::Sent | JobStatus::Running);
    Ok(Json(JobDetailView {
        job: JobView::from(&detail),
        trigger: JobTrigger::of(&detail.job),
        planned_steps: planned.iter().map(|step| step.step_id.clone()).collect(),
        steps: steps
            .iter()
            .map(|step| StepView::new(step, &planned))
            .collect(),
        planned: planned
            .iter()
            .map(|step| PlannedStepView {
                step_id: step.step_id.clone(),
                kind: step.kind,
                name: step.name.clone(),
                phase: step.phase,
                content_sha256: step.content_sha256.clone(),
            })
            .collect(),
        summary,
        progress: progress
            .filter(|_| running)
            .as_ref()
            .map(ProgressView::from),
        limits: LimitsView {
            max_duration_seconds: detail
                .job
                .timeout_seconds
                .and_then(|seconds| u32::try_from(seconds).ok())
                .filter(|seconds| *seconds > 0),
            stall_timeout_seconds: detail
                .job
                .stall_timeout_seconds
                .and_then(|seconds| u32::try_from(seconds).ok())
                .filter(|seconds| *seconds > 0),
        },
        log: LogInfo {
            lines: log.as_ref().map_or(0, |state| state.last_seq),
            truncated: log.as_ref().is_some_and(|state| state.truncated),
            purged: log.as_ref().is_some_and(|state| state.purged_at.is_some()),
        },
    }))
}

/// Query parameters of the log endpoint.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct LogQuery {
    /// The number of the last line that the client holds. The page starts after it. The default
    /// is 0, the start of the log.
    pub after_seq: Option<i64>,
    /// The most lines in the page, from 1 to 2000. The default is 500.
    pub limit: Option<i64>,
    /// Reads the lines of one step only, for example `script-0` (FR-HK-11). The default is every
    /// line of the job. Use an empty value for the lines of the job itself.
    pub step_id: Option<String>,
}

/// One line of a job log.
#[derive(Debug, Serialize, ToSchema)]
pub struct LogLineView {
    /// Keyset for `after_seq`. Counts from 1.
    pub seq: i64,
    /// Empty for a line of the job itself.
    pub step_id: String,
    pub stream: LogStream,
    pub level: LogLevel,
    /// The time at which the control plane stored the line, in RFC 3339.
    pub ts: String,
    pub text: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LogPageView {
    pub items: Vec<LogLineView>,
    /// True when the page holds `limit` lines, so a next page can follow.
    pub has_more: bool,
    pub truncated: bool,
    pub purged: bool,
}

/// Reads a page of the log of a job.
#[utoipa::path(
    get,
    path = "/api/v1/jobs/{id}/logs",
    summary = "Reads a page of the log of a job.",
    description = "The lines are in log order. Pass the `seq` of the last line as `after_seq` to read the next page or to follow a running job. The control plane deletes the lines after the retention period; the job and its summary stay.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the job."), LogQuery),
    responses(
        (status = 200, description = "A page of log lines.", body = LogPageView),
        (status = 400, description = "A query parameter is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The job is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_job_logs(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    Query(query): Query<LogQuery>,
) -> Result<Json<LogPageView>, ApiError> {
    context.authorize(Operation::JobsRead)?;
    let job_id = parse_job_id(&id)?;
    let after_seq = query.after_seq.unwrap_or(0);
    if after_seq < 0 {
        return Err(ApiError::invalid_field("after_seq", Reason::InvalidFormat));
    }
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LOG_LIMIT)
        .clamp(1, job_logs::MAX_READ_LIMIT);
    let exists = context
        .tenant_tx(&state, async |tx| {
            Ok(job_repo::get(tx, job_id).await?.is_some())
        })
        .await?;
    if !exists {
        return Err(ApiError::not_found());
    }
    if query.step_id.as_deref().is_some_and(|step| step.len() > 64) {
        return Err(ApiError::invalid_field("step_id", Reason::TooLong));
    }
    let page = DatabaseLogStore::new(state.db().clone())
        .read(
            context.tenant_id,
            job_id,
            query.step_id.as_deref(),
            after_seq,
            limit,
        )
        .await?;
    let has_more = i64::try_from(page.lines.len()).unwrap_or(i64::MAX) >= limit;
    Ok(Json(LogPageView {
        items: page
            .lines
            .iter()
            .map(|line| LogLineView {
                seq: line.seq,
                step_id: line.step_id.clone(),
                stream: line.stream,
                level: line.level,
                ts: format_time(line.ts),
                text: line.text.clone(),
            })
            .collect(),
        has_more,
        truncated: page.state.as_ref().is_some_and(|state| state.truncated),
        purged: page
            .state
            .as_ref()
            .is_some_and(|state| state.purged_at.is_some()),
    }))
}
