//! The maintenance endpoints of a repository (FR-RP-08, FR-SC-04, FR-SC-09, ADR-0034).
//!
//! A repository has two schedules, the retention cycle (`forget` and `prune`) and the `check`. The
//! maintenance agent of the repository runs both. An Admin reads and replaces the schedules; an
//! Operator starts a task by hand. A repository without a maintenance agent runs no maintenance,
//! and the view says so. A repository of another tenant gives HTTP 404 (AC-TN-01).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::audit::{AuditAction, diff};
use restichub_db::jobs::{self as job_repo, NewJob};
use restichub_db::maintenance::{self as schedules, MaintenanceSchedule, NewMaintenanceSchedule};
use restichub_db::queue as queue_repo;
use restichub_db::repositories as repo;
use restichub_db::{JobId, MaintenanceTask};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use crate::context::{Operation, RequestContext};
use crate::error::{ApiError, map_write_error};
use crate::jobs::{JobView, ensure_agent_active, format_time};
use crate::json::ApiJson;
use crate::maintenance_scheduler::{DEFAULT_TIME_ZONE, default_cron};
use crate::plans::next_run_at;
use crate::repositories::parse_repository_id;
use crate::schedule::{Schedule, ScheduleError};
use crate::server::AppState;
use crate::validation::Reason;

/// How long a manual maintenance run waits in the queue for its agent.
const MANUAL_RUN_LIFETIME: time::Duration = time::Duration::hours(1);

/// One schedule of a repository.
#[derive(Debug, Serialize, ToSchema)]
pub struct MaintenanceScheduleView {
    /// Five fields: minute, hour, day of month, month, day of week.
    pub cron: String,
    /// IANA name, for example `Europe/Berlin`.
    pub time_zone: String,
    pub enabled: bool,
    /// In UTC. Null when the schedule is disabled or the scheduler has not set it yet.
    #[schema(format = DateTime)]
    pub next_run_at: Option<String>,
    /// Holds the last fire time that the scheduler processed, in UTC.
    #[schema(format = DateTime)]
    pub last_fire_time: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MaintenanceView {
    pub repository_id: String,
    /// Null when the repository has no maintenance agent. The control plane then creates no
    /// maintenance job.
    pub maintenance_agent_id: Option<String>,
    pub retention: MaintenanceScheduleView,
    pub check: MaintenanceScheduleView,
    /// The latest `forget` job of the repository. Null when none ran.
    pub last_retention: Option<JobView>,
    /// The latest `check` job of the repository. Null when none ran.
    pub last_check: Option<JobView>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceScheduleInput {
    pub cron: String,
    pub enabled: bool,
}

/// The body of a schedule update. It replaces both schedules.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateMaintenanceRequest {
    /// An IANA name, such as `Europe/Berlin`, for both schedules.
    pub time_zone: String,
    pub retention: MaintenanceScheduleInput,
    pub check: MaintenanceScheduleInput,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RunMaintenanceRequest {
    pub task: MaintenanceTask,
}

fn schedule_view(
    row: Option<&MaintenanceSchedule>,
    task: MaintenanceTask,
) -> MaintenanceScheduleView {
    match row {
        Some(row) => MaintenanceScheduleView {
            cron: row.cron.clone(),
            time_zone: row.time_zone.clone(),
            enabled: row.enabled,
            next_run_at: row.next_run_at.map(format_time),
            last_fire_time: row.last_fire_time.map(format_time),
        },
        // A repository that exists before the migration has no row until the next scheduler pass.
        None => MaintenanceScheduleView {
            cron: default_cron(task).to_owned(),
            time_zone: DEFAULT_TIME_ZONE.to_owned(),
            enabled: true,
            next_run_at: None,
            last_fire_time: None,
        },
    }
}

async fn read_view(
    tx: &mut restichub_db::TenantTx<'_>,
    repository: &repo::Repository,
) -> Result<MaintenanceView, ApiError> {
    let rows = schedules::list_for_repository(tx, repository.id).await?;
    let find = |task: MaintenanceTask| rows.iter().find(|row| row.task == task);
    let last_retention =
        queue_repo::latest_of_kind(tx, repository.id, MaintenanceTask::Retention.job_kind())
            .await?;
    let last_check =
        queue_repo::latest_of_kind(tx, repository.id, MaintenanceTask::Check.job_kind()).await?;
    Ok(MaintenanceView {
        repository_id: repository.id.to_string(),
        maintenance_agent_id: repository.maintenance_agent_id.map(|id| id.to_string()),
        retention: schedule_view(find(MaintenanceTask::Retention), MaintenanceTask::Retention),
        check: schedule_view(find(MaintenanceTask::Check), MaintenanceTask::Check),
        last_retention: last_retention.as_ref().map(JobView::from),
        last_check: last_check.as_ref().map(JobView::from),
    })
}

/// Reads the maintenance schedules and the last maintenance jobs of a repository.
#[utoipa::path(
    get,
    path = "/api/v1/repositories/{id}/maintenance",
    tags = ["repositories"],
    summary = "Reads the maintenance of a repository.",
    description = "The view holds the retention and check schedules and the latest job of each task. Without a maintenance agent the control plane creates no maintenance job.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository.")),
    responses(
        (status = 200, description = "The maintenance of the repository.", body = MaintenanceView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository is not found.", body = crate::error::ProblemBody, content_type = "application/problem+json")
    )
)]
pub async fn get_maintenance(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<MaintenanceView>, ApiError> {
    context.authorize(Operation::RepositoriesRead)?;
    let repository_id = parse_repository_id(&id)?;
    let view = context
        .tenant_tx(&state, async |tx| {
            let Some(repository) = repo::get(tx, repository_id).await? else {
                return Err(ApiError::not_found());
            };
            read_view(tx, &repository).await
        })
        .await?;
    Ok(Json(view))
}

/// Replaces the maintenance schedules of a repository (FR-RP-08).
#[utoipa::path(
    put,
    path = "/api/v1/repositories/{id}/maintenance",
    tags = ["repositories"],
    summary = "Replaces the maintenance schedules of a repository.",
    description = "The body holds one time zone and the cron expression of each task. The control plane validates the expressions before the write.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository.")),
    request_body = UpdateMaintenanceRequest,
    responses(
        (status = 200, description = "The updated maintenance.", body = MaintenanceView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody, content_type = "application/problem+json"),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository is not found.", body = crate::error::ProblemBody, content_type = "application/problem+json")
    )
)]
pub async fn update_maintenance(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateMaintenanceRequest>,
) -> Result<Json<MaintenanceView>, ApiError> {
    context.authorize(Operation::RepositoriesWrite)?;
    let repository_id = parse_repository_id(&id)?;
    let now = state.auth().clock.now();
    let time_zone = body.time_zone.trim().to_owned();
    let inputs = [
        (
            MaintenanceTask::Retention,
            "retention.cron",
            body.retention.cron.trim().to_owned(),
            body.retention.enabled,
        ),
        (
            MaintenanceTask::Check,
            "check.cron",
            body.check.cron.trim().to_owned(),
            body.check.enabled,
        ),
    ];
    // The parse names the cron or the time zone field before the transaction opens.
    let mut parsed = Vec::new();
    for (task, field, cron, enabled) in &inputs {
        let schedule = parse_task_schedule(cron, &time_zone, field)?;
        parsed.push((*task, cron.clone(), *enabled, schedule));
    }

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(repository) = repo::get_for_update(&mut tx, repository_id).await? else {
        return Err(ApiError::not_found());
    };
    let mut after = serde_json::Map::new();
    for (task, cron, enabled, schedule) in &parsed {
        let next = next_run_at(schedule, *enabled, now)?;
        let current = schedules::get_for_update(&mut tx, repository_id, *task).await?;
        match current {
            Some(row) => {
                // An unchanged schedule keeps its pending fire time.
                let changed =
                    row.cron != *cron || row.time_zone != time_zone || row.enabled != *enabled;
                let next_run = if changed { next } else { row.next_run_at };
                schedules::update(&mut tx, row.id, cron, &time_zone, *enabled, next_run).await?;
            }
            None => {
                schedules::create(
                    &mut tx,
                    &NewMaintenanceSchedule {
                        repository_id,
                        task: *task,
                        cron,
                        time_zone: &time_zone,
                        enabled: *enabled,
                        next_run_at: next,
                    },
                )
                .await
                .map_err(map_write_error)?;
            }
        }
        after.insert(
            task.as_str().to_owned(),
            json!({"cron": cron, "enabled": enabled}),
        );
    }
    after.insert("time_zone".to_owned(), json!(time_zone));

    let mut event = context.audit_event(
        AuditAction::RepositoryUpdate,
        "repository",
        repository_id.as_uuid(),
    );
    event.diff = Some(diff(
        None,
        Some(&json!({"maintenance": serde_json::Value::Object(after)})),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    let view = read_view(&mut tx, &repository).await?;
    tx.commit().await?;
    Ok(Json(view))
}

/// Parses a schedule and names the invalid request field.
fn parse_task_schedule(
    cron: &str,
    time_zone: &str,
    cron_field: &'static str,
) -> Result<Schedule, ApiError> {
    Schedule::parse(cron, time_zone).map_err(|error| match error {
        ScheduleError::TimeZone => ApiError::invalid_field("time_zone", Reason::Unknown),
        ScheduleError::Cron | ScheduleError::Search => {
            ApiError::invalid_field(cron_field, Reason::InvalidFormat)
        }
    })
}

/// Starts a maintenance task of a repository now (FR-SC-09).
#[utoipa::path(
    post,
    path = "/api/v1/repositories/{id}/maintenance/run",
    tags = ["repositories"],
    summary = "Runs a maintenance task now.",
    description = "The job runs on the maintenance agent and follows the barriers of the queue. The retention task needs a profile with a retention that a plan runs. A repository with an active exclusive job gives HTTP 409.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository.")),
    request_body = RunMaintenanceRequest,
    responses(
        (status = 202, description = "The job is queued.", body = JobView),
        (status = 400, description = "The repository has no maintenance agent.", body = crate::error::ProblemBody, content_type = "application/problem+json"),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository is not found.", body = crate::error::ProblemBody, content_type = "application/problem+json"),
        (status = 409, description = "The repository is not initialized, has nothing to forget, or holds an active exclusive job.", body = crate::error::ProblemBody, content_type = "application/problem+json")
    )
)]
pub async fn run_maintenance(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RunMaintenanceRequest>,
) -> Result<(StatusCode, Json<JobView>), ApiError> {
    context.authorize(Operation::JobsWrite)?;
    let repository_id = parse_repository_id(&id)?;
    let now = state.auth().clock.now();
    let job_id = JobId::new();
    let created_by = crate::secrets::actor_user(&context.actor);

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(repository) = repo::get_for_update(&mut tx, repository_id).await? else {
        return Err(ApiError::not_found());
    };
    let agent_id = repository
        .maintenance_agent_id
        .ok_or_else(|| ApiError::invalid_field("maintenance_agent_id", Reason::Empty))?;
    ensure_agent_active(&mut tx, agent_id).await?;
    if !repository.initialized {
        return Err(ApiError::conflict());
    }
    if body.task == MaintenanceTask::Retention
        && !schedules::has_retention_in_use(&mut tx, repository_id).await?
    {
        return Err(ApiError::conflict());
    }
    if queue_repo::has_active_exclusive_for_repository(&mut tx, repository_id).await? {
        return Err(ApiError::conflict());
    }
    job_repo::create(
        &mut tx,
        &NewJob {
            id: job_id,
            kind: body.task.job_kind(),
            agent_id,
            repository_id: Some(repository_id),
            plan_id: None,
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
            ApiError::conflict()
        } else {
            map_write_error(error)
        }
    })?;
    let mut event = context.audit_event(AuditAction::JobRun, "job", job_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&json!({
            "kind": body.task.job_kind().as_str(),
            "agent_id": agent_id.to_string(),
            "repository_id": repository_id.to_string(),
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
        .job_status(context.tenant_id, job_id, restichub_db::JobStatus::Queued);
    let Some(detail) = detail else {
        return Err(ApiError::internal("the job is not found after the create"));
    };
    Ok((StatusCode::ACCEPTED, Json(JobView::from(&detail))))
}
