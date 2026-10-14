//! The job list, the cancel action, the manual plan run and the `unlock_all` action (FR-SC-06,
//! FR-SC-09).
//!
//! An Operator runs a plan by hand and cancels a job. An Admin runs `unlock_all` after a typed
//! confirmation. Each action writes a `queued` row and wakes the dispatcher; the dispatcher sends
//! the job (ADR-0010). A job of another tenant gives HTTP 404 (AC-TN-01).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use restichub_db::audit::{AuditAction, diff};
use restichub_db::jobs::{self as job_repo, NewJob};
use restichub_db::plans as plans_repo;
use restichub_db::queue::{self as queue_repo, JobCursor, JobFilter, JobPage};
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
use crate::server::AppState;
use crate::validation::Reason;

/// How long a manual run waits in the queue for its agent.
const MANUAL_RUN_LIFETIME: time::Duration = time::Duration::hours(1);

/// The seconds that the agent waits before it kills the process tree of a cancelled job (FR-AG-10).
const CANCEL_GRACE_SECONDS: u32 = 30;

/// How long an `unlock_all` job waits in the queue for its agent.
const UNLOCK_ALL_LIFETIME: time::Duration = time::Duration::hours(1);

/// Query parameters of the job list (FR-SC-09).
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct JobListQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    pub plan_id: Option<String>,
    pub repository_id: Option<String>,
    pub agent_id: Option<String>,
    pub status: Option<JobStatus>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct JobListResponse {
    pub items: Vec<JobView>,
    pub next_cursor: Option<String>,
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

/// Lists the jobs of the tenant, newest first, with keyset pagination.
#[utoipa::path(
    get,
    path = "/api/v1/jobs",
    summary = "Lists the jobs of the tenant.",
    description = "The list is newest first. Filter by plan, repository, agent and status.",
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
    Query(query): Query<JobListQuery>,
) -> Result<Json<JobListResponse>, ApiError> {
    context.authorize(Operation::JobsRead)?;
    let filter = JobFilter {
        plan_id: parse_filter(query.plan_id.as_deref(), "plan_id", PlanId::from_uuid)?,
        repository_id: parse_filter(
            query.repository_id.as_deref(),
            "repository_id",
            RepositoryId::from_uuid,
        )?,
        agent_id: parse_filter(query.agent_id.as_deref(), "agent_id", AgentId::from_uuid)?,
        status: query.status,
    };
    let page = JobPage {
        limit: query.limit.unwrap_or(queue_repo::DEFAULT_PAGE_LIMIT),
        after: query
            .cursor
            .as_deref()
            .map(decode_cursor)
            .transpose()
            .map_err(|cause| {
                ApiError::invalid_field_cause("cursor", Reason::InvalidFormat, cause)
            })?,
    };
    let result = context
        .tenant_tx(&state, async |tx| {
            Ok(queue_repo::list(tx, &filter, &page).await?)
        })
        .await?;
    let next_cursor = if result.has_more {
        result.jobs.last().map(|detail| {
            encode_cursor(JobCursor {
                created_at: detail.job.created_at,
                id: detail.job.id,
            })
        })
    } else {
        None
    };
    let items = result.jobs.iter().map(JobView::from).collect();
    Ok(Json(JobListResponse { items, next_cursor }))
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
            if !state.agent_registry().send_to_agent(job.agent_id, message) {
                return Err(ApiError::agent_offline());
            }
            // A job that the agent never acknowledged must not go out again after the cancel.
            queue_repo::stop_redelivery(&mut tx, job_id).await?;
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
    description = "The job waits in the queue and starts as soon as the agent is online and the limits allow it.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the plan.")),
    responses(
        (status = 202, description = "The job is queued.", body = JobView),
        (status = 400, description = "The profile does not compile for the agent.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The plan is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The agent of the plan is revoked.", body = crate::error::ProblemBody)
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
    // The job builder compiles again at send time. The early check gives the user the field error.
    crate::profiles::compile_for_agent(&profile, &agent, now)?;

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
    .map_err(map_write_error)?;
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
    let Some(detail) = detail else {
        return Err(ApiError::internal("the job is not found after the create"));
    };
    Ok((StatusCode::ACCEPTED, Json(JobView::from(&detail))))
}

fn parse_filter<T>(
    value: Option<&str>,
    field: &'static str,
    wrap: fn(Uuid) -> T,
) -> Result<Option<T>, ApiError> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) => Uuid::parse_str(value).map(wrap).map(Some).map_err(|error| {
            ApiError::invalid_field_cause(field, Reason::InvalidFormat, error.to_string())
        }),
    }
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
