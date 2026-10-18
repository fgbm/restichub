//! The job list, the cancel action, the manual plan run and the `unlock_all` action (FR-SC-06,
//! FR-SC-09).
//!
//! An Operator runs a plan by hand and cancels a job. An Admin runs `unlock_all` after a typed
//! confirmation. Each action writes a `queued` row and wakes the dispatcher; the dispatcher sends
//! the job (ADR-0010). A job of another tenant gives HTTP 404 (AC-TN-01).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::audit::{AuditAction, diff};
use restichub_db::job_search::{
    self, GroupCounts, JobCursor, JobFilter, JobGroup, JobQuery, JobSort, JobWindow,
};
use restichub_db::jobs::{self as job_repo, NewJob};
use restichub_db::plans as plans_repo;
use restichub_db::queue as queue_repo;
use restichub_db::repositories as repo;
use restichub_db::{
    AgentId, JobId, JobKind, JobStatus, PlanId, RepositoryId, agents as agents_repo,
    profiles as profiles_repo,
};
use restichub_proto::agent::v1::{CancelJob, ControlMessage, control_message};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::context::{Operation, RequestContext};
use crate::error::{ApiError, map_write_error};
use crate::jobs::{JobView, ensure_agent_active, parse_job_id, resolve_agent};
use crate::json::ApiJson;
use crate::list_query::{
    ApiQuery, SortOrder, TabCount, check_period, check_window, parse_list, parse_search,
    parse_time, split_list,
};
use crate::server::AppState;
use crate::validation::Reason;

/// How long a manual run waits in the queue for its agent.
const MANUAL_RUN_LIFETIME: time::Duration = time::Duration::hours(1);

/// The seconds that the agent waits before it kills the process tree of a cancelled job (FR-AG-10).
pub(crate) const CANCEL_GRACE_SECONDS: u32 = 30;

/// How long an `unlock_all` job waits in the queue for its agent.
const UNLOCK_ALL_LIFETIME: time::Duration = time::Duration::hours(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobSortKey {
    CreatedAt,
    StartedAt,
    FinishedAt,
    Duration,
    Status,
    Plan,
    Host,
}

impl From<JobSortKey> for JobSort {
    fn from(key: JobSortKey) -> Self {
        match key {
            JobSortKey::CreatedAt => Self::Created,
            JobSortKey::StartedAt => Self::Started,
            JobSortKey::FinishedAt => Self::Finished,
            JobSortKey::Duration => Self::Duration,
            JobSortKey::Status => Self::Status,
            JobSortKey::Plan => Self::Plan,
            JobSortKey::Host => Self::Host,
        }
    }
}

/// Query parameters of the job list (FR-SC-09, FR-UI-01).
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct JobListQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    /// The first row of a numbered page, at most 10000. Do not combine with `cursor`.
    pub offset: Option<i64>,
    /// Comma-separated identifiers. Each list holds at most 50 values.
    pub plan_id: Option<String>,
    /// Comma-separated identifiers.
    pub repository_id: Option<String>,
    /// Comma-separated identifiers.
    pub agent_id: Option<String>,
    /// Comma-separated, for example `backup,check`.
    pub kind: Option<String>,
    pub status: Option<JobStatus>,
    /// A state tab. `failed` leaves out the cancelled jobs; `succeeded` leaves out the jobs with
    /// warnings.
    #[param(inline)]
    pub group: Option<JobGroupKey>,
    /// A part of the plan, host or repository name, in any case; at most 200 characters.
    pub q: Option<String>,
    /// In RFC 3339: the first creation time that the list holds.
    pub from: Option<String>,
    /// In RFC 3339: the last creation time that the list holds. Not before `from`.
    pub to: Option<String>,
    /// Default: `created_at`. A cursor takes only the default order. `duration` puts an unfinished
    /// job last.
    #[param(inline)]
    pub sort: Option<JobSortKey>,
    /// Default: `desc`.
    #[param(inline)]
    pub order: Option<SortOrder>,
    /// Adds the state tab totals to the response.
    pub counts: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct JobListResponse {
    pub items: Vec<JobView>,
    /// Null when the list holds no later job, and for a numbered page or another order.
    pub next_cursor: Option<String>,
    /// The number of jobs that match the filters on all pages, at most 10000.
    pub total: i64,
    /// More jobs match than `total` counts.
    pub total_capped: bool,
    /// The jobs of each state tab under the other filters; `status` and `group` do not apply.
    /// Present when the request asks for it with `counts=true`.
    pub counts: Option<JobGroupCounts>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobGroupKey {
    Running,
    Failed,
    Succeeded,
    SucceededWithWarnings,
}

impl From<JobGroupKey> for JobGroup {
    fn from(key: JobGroupKey) -> Self {
        match key {
            JobGroupKey::Running => Self::Running,
            JobGroupKey::Failed => Self::Failed,
            JobGroupKey::Succeeded => Self::Succeeded,
            JobGroupKey::SucceededWithWarnings => Self::SucceededWithWarnings,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct JobGroupCounts {
    /// Every job under the other filters, in each state.
    pub all: TabCount,
    pub running: TabCount,
    pub failed: TabCount,
    pub succeeded: TabCount,
    pub succeeded_with_warnings: TabCount,
}

impl From<&GroupCounts> for JobGroupCounts {
    fn from(counts: &GroupCounts) -> Self {
        Self {
            all: counts.all.into(),
            running: counts.of(Some(JobGroup::Running)).into(),
            failed: counts.of(Some(JobGroup::Failed)).into(),
            succeeded: counts.of(Some(JobGroup::Succeeded)).into(),
            succeeded_with_warnings: counts.of(Some(JobGroup::SucceededWithWarnings)).into(),
        }
    }
}

/// The request body of the `unlock_all` action (FR-SC-06).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UnlockAllRequest {
    /// The name of the repository. The action refuses a different value.
    pub confirm: String,
    /// The agent that runs the step. An absent value uses the maintenance agent of the repository.
    #[serde(default)]
    pub agent_id: Option<String>,
}

/// Lists the jobs of the tenant with filters, a sort order and a cursor or numbered pages.
#[utoipa::path(
    get,
    path = "/api/v1/jobs",
    summary = "Lists the jobs of the tenant.",
    description = "The list is newest first by default. Filter by plan, repository, agent, kind, status, state tab, name search and creation time; sort by a column; read the next page with the cursor or with an offset. With `counts=true` the response counts the jobs of each state tab.",
    security(("cookie" = []), ("bearer" = [])),
    params(JobListQuery),
    responses(
        (status = 200, description = "The jobs of the tenant.", body = JobListResponse),
        (status = 400, description = "A query parameter is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_jobs(
    State(state): State<AppState>,
    context: RequestContext,
    ApiQuery(query): ApiQuery<JobListQuery>,
) -> Result<Json<JobListResponse>, ApiError> {
    context.authorize(Operation::JobsRead)?;
    let filter = JobFilter {
        plan_ids: parse_ids(query.plan_id.as_deref(), "plan_id", PlanId::from_uuid)?,
        repository_ids: parse_ids(
            query.repository_id.as_deref(),
            "repository_id",
            RepositoryId::from_uuid,
        )?,
        agent_ids: parse_ids(query.agent_id.as_deref(), "agent_id", AgentId::from_uuid)?,
        kinds: parse_list("kind", query.kind.as_deref(), JobKind::parse)?,
        status: query.status,
        group: query.group.map(JobGroup::from),
        search: parse_search(query.q.as_deref())?,
        created_from: parse_time("from", query.from.as_deref())?,
        created_to: parse_time("to", query.to.as_deref())?,
    };
    check_period(filter.created_from, filter.created_to)?;
    let sort = query.sort.map_or(JobSort::Created, JobSort::from);
    let order = query.order.unwrap_or_default();
    let default_order = sort == JobSort::Created && order.descending();
    check_window(query.cursor.as_deref(), query.offset, default_order)?;
    let window = match query.cursor.as_deref() {
        Some(cursor) => JobWindow::After(decode_cursor(cursor).map_err(|cause| {
            ApiError::invalid_field_cause("cursor", Reason::InvalidFormat, cause)
        })?),
        None => JobWindow::Offset(query.offset.unwrap_or(0)),
    };
    let page = JobQuery {
        filter: &filter,
        sort,
        descending: order.descending(),
        window,
        limit: query.limit.unwrap_or(queue_repo::DEFAULT_PAGE_LIMIT),
    };
    let with_counts = query.counts.unwrap_or(false);
    let (result, total, counts) = context
        .tenant_tx(&state, async |tx| {
            let result = job_search::search(tx, &page).await?;
            let counts = if with_counts {
                Some(job_search::group_counts(tx, &filter).await?)
            } else {
                None
            };
            // The tabs ignore the status filter, so only without it the total is one of them.
            let total = match &counts {
                Some(counts) if filter.status.is_none() => counts.of(filter.group),
                _ => job_search::total(tx, &filter).await?,
            };
            Ok((result, total, counts))
        })
        .await?;
    let next_cursor = (result.has_more && default_order && query.offset.is_none())
        .then(|| {
            result.jobs.last().map(|detail| {
                encode_cursor(JobCursor {
                    created_at: detail.job.created_at,
                    id: detail.job.id,
                })
            })
        })
        .flatten();
    let items = result.jobs.iter().map(JobView::from).collect();
    Ok(Json(JobListResponse {
        items,
        next_cursor,
        total: total.count,
        total_capped: total.capped,
        counts: counts.as_ref().map(JobGroupCounts::from),
    }))
}

/// Cancels a job (FR-SC-09).
///
/// A queued job is cancelled at once. A sent or running job gets a `CancelJob` message; the agent
/// reports the cancelled result. A job that already finished gives HTTP 409.
#[utoipa::path(
    post,
    path = "/api/v1/jobs/{id}/cancel",
    summary = "Cancels a job.",
    description = "A queued job is cancelled at once. For a sent or running job the control plane sends the cancel message to the agent, and an offline agent gives HTTP 409 `agent_offline`.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the job.")),
    responses(
        (status = 200, description = "The queued job is cancelled.", body = JobView),
        (status = 202, description = "The cancel message is sent to the agent.", body = JobView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The job is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The job finished, or the agent is offline.", body = crate::error::ProblemBody)
    )
)]
pub async fn cancel_job(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<JobView>), ApiError> {
    context.authorize(Operation::JobsWrite)?;
    let job_id = parse_job_id(&id)?;
    let now = state.auth().clock.now();
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(job) = job_repo::get_for_update(&mut tx, job_id).await? else {
        return Err(ApiError::not_found());
    };
    // The `unlock_all` action is an Admin action, so only an Admin cancels it.
    if job.kind == JobKind::UnlockAll {
        context.authorize(Operation::RepositoriesWrite)?;
    }
    let status = match job.status {
        JobStatus::Queued => {
            queue_repo::cancel_queued(&mut tx, job_id, now).await?;
            StatusCode::OK
        }
        JobStatus::Sent | JobStatus::Running => {
            let message = ControlMessage {
                payload: Some(control_message::Payload::CancelJob(CancelJob {
                    job_id: job_id.to_string(),
                    grace_period_seconds: CANCEL_GRACE_SECONDS,
                })),
            };
            let delivered = state.agent_registry().send_to_agent(job.agent_id, message);
            // A running job needs its agent. A sent job that the agent never acknowledged ends with
            // its lease, so an offline agent does not block the cancel.
            if !delivered && job.status == JobStatus::Running {
                return Err(ApiError::agent_offline());
            }
            // The flag keeps the dispatcher from sending the job, and the acknowledgement of a job
            // that the cancel overtook gets the cancel message again.
            queue_repo::request_cancel(&mut tx, job_id).await?;
            StatusCode::ACCEPTED
        }
        JobStatus::Succeeded | JobStatus::Failed | JobStatus::Expired | JobStatus::DeadLetter => {
            return Err(ApiError::conflict());
        }
    };
    let mut event = context.audit_event(AuditAction::JobCancel, "job", job_id.as_uuid());
    event.diff = Some(diff(
        Some(&json!({"status": job.status.as_str()})),
        None,
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    let detail = queue_repo::get_detail(&mut tx, job_id).await?;
    tx.commit().await?;
    state.wake_dispatcher();
    state.events().job_status(
        context.tenant_id,
        job_id,
        if status == StatusCode::OK {
            JobStatus::Failed
        } else {
            job.status
        },
    );
    let Some(detail) = detail else {
        return Err(ApiError::internal("the job is not found after the cancel"));
    };
    Ok((status, Json(JobView::from(&detail))))
}

/// Runs a plan now (FR-SC-09).
///
/// The control plane checks that the stored profile compiles for the agent, writes one queued
/// backup job and wakes the dispatcher. The job follows the barriers and the limits of the queue.
#[utoipa::path(
    post,
    path = "/api/v1/plans/{id}/run",
    summary = "Runs a plan now.",
    description = "The job waits in the queue and starts as soon as the agent is online and the limits allow it. A plan holds at most one queued manual run: a second run gives HTTP 409 `already_queued`.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the plan.")),
    responses(
        (status = 202, description = "The job is queued.", body = JobView),
        (status = 400, description = "The profile does not compile for the agent.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The plan is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The agent of the plan is revoked, a manual run of the plan is already queued (`already_queued`), or the plan runs on another host than its local repository (`not_repository_host`).", body = crate::error::ProblemBody)
    )
)]
pub async fn run_plan(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<JobView>), ApiError> {
    context.authorize(Operation::PlansWrite)?;
    let plan_id = crate::plans::parse_plan_id(&id)?;
    let now = state.auth().clock.now();
    let job_id = JobId::new();
    let created_by = crate::secrets::actor_user(&context.actor);

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(plan) = plans_repo::get(&mut tx, plan_id).await? else {
        return Err(ApiError::not_found());
    };
    let Some(agent) = agents_repo::get(&mut tx, plan.agent_id).await? else {
        return Err(ApiError::conflict());
    };
    if agent.status != restichub_db::AgentStatus::Active {
        return Err(ApiError::conflict());
    }
    let Some(profile) = profiles_repo::get(&mut tx, plan.profile_id).await? else {
        return Err(ApiError::conflict());
    };
    let Some(repository) = repo::get(&mut tx, plan.repository_id).await? else {
        return Err(ApiError::conflict());
    };
    // The request has no agent field, so the refusal is a code of its own, not a field error.
    if crate::jobs::ensure_repository_host(&repository, plan.agent_id).is_err() {
        return Err(ApiError::not_repository_host());
    }
    // The job builder compiles again at send time. The early check gives the user the field error.
    let definitions = crate::parameters::from_row(&profile.parameters)?;
    let parameters =
        crate::parameters::resolved_for_plan(&mut tx, &definitions, plan.id, plan.agent_id).await?;
    crate::profiles::compile_for_agent(&profile, plan.repository_id, &agent, parameters, now)?;
    if queue_repo::has_queued_manual_run(&mut tx, plan_id).await? {
        return Err(ApiError::already_queued());
    }

    job_repo::create(
        &mut tx,
        &NewJob {
            id: job_id,
            kind: JobKind::Backup,
            agent_id: plan.agent_id,
            repository_id: Some(plan.repository_id),
            plan_id: Some(plan_id),
            fire_time: None,
            expires_at: Some(now + MANUAL_RUN_LIFETIME),
            location_fingerprint: None,
            created_by,
        },
    )
    .await
    // The partial unique index is the guard of two runs that race.
    .map_err(|error| {
        if error.is_unique_violation() {
            ApiError::already_queued()
        } else {
            map_write_error(error)
        }
    })?;
    let mut event = context.audit_event(AuditAction::JobRun, "job", job_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&json!({
            "kind": JobKind::Backup.as_str(),
            "plan_id": plan_id.to_string(),
            "agent_id": plan.agent_id.to_string(),
            "repository_id": plan.repository_id.to_string(),
            "manual": true,
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    let detail = queue_repo::get_detail(&mut tx, job_id).await?;
    tx.commit().await?;
    state.wake_dispatcher();
    state
        .events()
        .job_status(context.tenant_id, job_id, JobStatus::Queued);
    let Some(detail) = detail else {
        return Err(ApiError::internal("the job is not found after the create"));
    };
    Ok((StatusCode::ACCEPTED, Json(JobView::from(&detail))))
}

/// Removes every lock of a repository (FR-SC-06).
///
/// Only an Admin runs the action, and only with the repository name as the confirmation. The
/// action removes the locks of a running operation too, so it refuses a repository that has a
/// waiting or running job. The audit log keeps the event.
#[utoipa::path(
    post,
    path = "/api/v1/repositories/{id}/unlock-all",
    summary = "Removes every lock of a repository.",
    description = "The body must hold the repository name as the confirmation. A repository with a waiting or running job gives HTTP 409.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository.")),
    request_body = UnlockAllRequest,
    responses(
        (status = 202, description = "The job is queued.", body = JobView),
        (status = 400, description = "The confirmation or a field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "A job waits or runs on the repository.", body = crate::error::ProblemBody)
    )
)]
pub async fn unlock_all_repository(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UnlockAllRequest>,
) -> Result<(StatusCode, Json<JobView>), ApiError> {
    context.authorize(Operation::RepositoriesWrite)?;
    let repository_id = crate::repositories::parse_repository_id(&id)?;
    let explicit_agent =
        match body.agent_id.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(value) => Some(Uuid::parse_str(value).map(AgentId::from_uuid).map_err(
                |error| {
                    ApiError::invalid_field_cause(
                        "agent_id",
                        Reason::InvalidFormat,
                        error.to_string(),
                    )
                },
            )?),
        };
    let now = state.auth().clock.now();
    let job_id = JobId::new();
    let created_by = crate::secrets::actor_user(&context.actor);

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(repository) = repo::get_for_update(&mut tx, repository_id).await? else {
        return Err(ApiError::not_found());
    };
    if body.confirm != repository.name {
        return Err(ApiError::invalid_field("confirm", Reason::Mismatch));
    }
    let agent_id = resolve_agent(&repository, explicit_agent, JobKind::TestAccess)?;
    ensure_agent_active(&mut tx, agent_id).await?;
    if queue_repo::has_any_active_for_repository(&mut tx, repository_id).await? {
        return Err(ApiError::conflict());
    }
    job_repo::create(
        &mut tx,
        &NewJob {
            id: job_id,
            kind: JobKind::UnlockAll,
            agent_id,
            repository_id: Some(repository_id),
            plan_id: None,
            fire_time: None,
            expires_at: Some(now + UNLOCK_ALL_LIFETIME),
            location_fingerprint: None,
            created_by,
        },
    )
    .await
    .map_err(map_write_error)?;
    let mut event = context.audit_event(
        AuditAction::RepositoryUnlockAll,
        "repository",
        repository_id.as_uuid(),
    );
    event.diff = Some(diff(
        None,
        Some(&json!({
            "job_id": job_id.to_string(),
            "agent_id": agent_id.to_string(),
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    let detail = queue_repo::get_detail(&mut tx, job_id).await?;
    tx.commit().await?;
    state.wake_dispatcher();
    state
        .events()
        .job_status(context.tenant_id, job_id, JobStatus::Queued);
    let Some(detail) = detail else {
        return Err(ApiError::internal("the job is not found after the create"));
    };
    Ok((StatusCode::ACCEPTED, Json(JobView::from(&detail))))
}

fn parse_ids<T>(
    value: Option<&str>,
    field: &'static str,
    wrap: fn(Uuid) -> T,
) -> Result<Vec<T>, ApiError> {
    split_list(field, value)?
        .iter()
        .map(|item| {
            Uuid::parse_str(item).map(wrap).map_err(|error| {
                ApiError::invalid_field_cause(field, Reason::InvalidFormat, error.to_string())
            })
        })
        .collect()
}

fn encode_cursor(cursor: JobCursor) -> String {
    format!("{};{}", cursor.created_at.unix_timestamp_nanos(), cursor.id)
}

fn decode_cursor(value: &str) -> Result<JobCursor, String> {
    let (time, id) = value
        .split_once(';')
        .ok_or_else(|| "the cursor holds no `;`".to_owned())?;
    let nanos: i128 = time
        .parse()
        .map_err(|error: std::num::ParseIntError| error.to_string())?;
    let created_at = time::OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .map_err(|error| error.to_string())?;
    if !(1970..=9999).contains(&created_at.year()) {
        return Err("the time of the cursor is outside the supported range".to_owned());
    }
    let id = id.parse().map_err(|error: uuid::Error| error.to_string())?;
    Ok(JobCursor {
        created_at,
        id: JobId::from_uuid(id),
    })
}
