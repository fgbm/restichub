//! The plan endpoints (FR-SC-01, FR-SC-07, FR-SC-11, ADR-0006, ADR-0010).
//!
//! An Admin and an Operator create, update and delete the plans of the tenant (requirements
//! section 2). A Viewer reads. A plan of another tenant gives HTTP 404 (AC-TN-01). The platform
//! tenant refuses every plan operation with HTTP 403.
//!
//! The control plane validates a plan before a write (FR-PF-05): the cron expression and the IANA
//! time zone, the profile, the agent and the repository inside the tenant, and the compiler accepts
//! the stored profile for the operating system of the agent. A rejected compile names the profile
//! field (AC-SC-01). A local repository binds the plan to its repository host (ADR-0046).
//!
//! The schedule belongs to the plan (FR-SC-01). The endpoint `GET /api/v1/plans/{id}/next-runs`
//! returns the next fire times in UTC, for the user interface preview in the plan time zone.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use restichub_db::agents::{self as agents_repo};
use restichub_db::audit::{AuditAction, diff};
use restichub_db::plans::{self as repo, Plan, PlanCursor, PlanPage};
use restichub_db::profiles::{self as profiles_repo};
use restichub_db::{AgentId, AgentStatus, MissedRunPolicy, PlanId, ProfileId, RepositoryId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::Duration;
use utoipa::{IntoParams, ToSchema};

use crate::alerts::rpo::{self, RpoState};
use crate::context::{Operation, RequestContext};
use crate::error::{ApiError, map_write_error};
use crate::json::ApiJson;
use crate::parameters::{self, ParameterValueDef, PlanParameterValueView};
use crate::schedule::{Schedule, ScheduleError};
use crate::server::AppState;
use crate::validation::Reason;

/// The largest number of fire times that the preview returns (FR-SC-01).
const MAX_PREVIEW_RUNS: usize = 20;

/// The default number of fire times that the preview returns.
const DEFAULT_PREVIEW_RUNS: usize = 5;

/// Query parameters of the list endpoint (FR-SC-01).
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct PlanListQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

/// Query parameters of the next-runs preview (FR-SC-01).
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct NextRunsQuery {
    pub count: Option<usize>,
}

/// The request body of a plan create (FR-SC-01, FR-SC-07, FR-SC-11).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePlanRequest {
    /// Free text of 1 to 100 characters. Unique in the tenant, ignoring the letter case. Holds no control or format character.
    #[schema(max_length = 100)]
    pub name: String,
    pub profile_id: String,
    pub agent_id: String,
    /// The repository that the plan backs up into (ADR-0047).
    #[schema(required)]
    #[serde(default)]
    pub repository_id: String,
    pub cron: String,
    /// An IANA name, such as `Europe/Berlin`, validated before the write.
    pub time_zone: String,
    pub missed_run_policy: MissedRunPolicy,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// The recovery point objective in seconds, from 900 to 34560000. Absent or null uses the
    /// default of the schedule (ADR-0036).
    #[serde(default)]
    pub rpo_seconds: Option<i32>,
    /// The values that the plan sets instead of the values of the host (FR-PF-10). At most 64.
    #[serde(default)]
    #[schema(max_items = 64)]
    pub parameter_values: Vec<ParameterValueDef>,
}

/// The request body of a plan update. An absent name keeps the name.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlanRequest {
    /// Free text of 1 to 100 characters. Unique in the tenant, ignoring the letter case. Holds no control or format character.
    #[schema(max_length = 100)]
    #[serde(default)]
    pub name: Option<String>,
    pub profile_id: String,
    pub agent_id: String,
    /// The repository that the plan backs up into (ADR-0047). An absent value is refused.
    #[schema(required)]
    #[serde(default)]
    pub repository_id: String,
    pub cron: String,
    pub time_zone: String,
    pub missed_run_policy: MissedRunPolicy,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// The recovery point objective in seconds. Null restores the default of the schedule.
    #[serde(default)]
    pub rpo_seconds: Option<i32>,
    /// Replaces the whole list of the values of the plan (FR-PF-10).
    #[serde(default)]
    #[schema(max_items = 64)]
    pub parameter_values: Vec<ParameterValueDef>,
}

fn default_enabled() -> bool {
    true
}

/// One plan with its display values (FR-SC-01).
#[derive(Debug, Serialize, ToSchema)]
pub struct PlanView {
    pub id: String,
    pub name: String,
    pub profile_id: String,
    pub profile_name: String,
    pub repository_id: String,
    pub agent_id: String,
    pub agent_hostname: String,
    pub cron: String,
    pub time_zone: String,
    pub missed_run_policy: MissedRunPolicy,
    pub enabled: bool,
    /// The stored recovery point objective in seconds. Null means the default of the schedule.
    pub rpo_seconds: Option<i32>,
    /// The objective that the control plane applies, in seconds (ADR-0036).
    pub effective_rpo_seconds: i64,
    /// In UTC, in RFC 3339. Null when the plan is disabled.
    pub next_run_at: Option<String>,
    /// The last fire time that the scheduler processed, in RFC 3339.
    pub last_fire_time: Option<String>,
    /// The finish time of the last succeeded backup, in RFC 3339. Null when the plan never
    /// succeeded.
    pub last_success_at: Option<String>,
    /// The state of the dashboard (ADR-0036). `unknown` when the host of the plan is revoked.
    pub rpo_state: PlanRpoState,
    /// The values that the plan sets instead of the values of the host (FR-PF-10). For a Viewer, a
    /// text value is empty with the flag `hidden` (FR-PF-12).
    pub parameter_values: Vec<PlanParameterValueView>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanRpoState {
    Ok,
    AtRisk,
    Violated,
    Unknown,
}

impl From<RpoState> for PlanRpoState {
    fn from(state: RpoState) -> Self {
        match state {
            RpoState::Ok => Self::Ok,
            RpoState::AtRisk => Self::AtRisk,
            RpoState::Violated => Self::Violated,
        }
    }
}

impl PlanView {
    /// `now` comes from the clock of the control plane, so the RPO state agrees with the dashboard.
    /// `parameter_values` are the overrides of the plan, which the caller reads with the row.
    fn of(
        plan: &Plan,
        parameter_values: Vec<PlanParameterValueView>,
        now: time::OffsetDateTime,
    ) -> Self {
        let rpo = rpo::effective_rpo(&plan.cron, &plan.time_zone, plan.rpo_seconds, now);
        // The dashboard leaves out a plan whose host cannot run a backup any more.
        let rpo_state = if plan.agent_status == AgentStatus::Active {
            let age = (now - plan.last_success_at.unwrap_or(plan.created_at)).max(Duration::ZERO);
            PlanRpoState::from(rpo::state_of(age, rpo))
        } else {
            PlanRpoState::Unknown
        };
        Self {
            id: plan.id.to_string(),
            name: plan.name.clone(),
            profile_id: plan.profile_id.to_string(),
            profile_name: plan.profile_name.clone(),
            repository_id: plan.repository_id.to_string(),
            agent_id: plan.agent_id.to_string(),
            agent_hostname: plan.agent_hostname.clone(),
            cron: plan.cron.clone(),
            time_zone: plan.time_zone.clone(),
            missed_run_policy: plan.missed_run_policy,
            enabled: plan.enabled,
            rpo_seconds: plan.rpo_seconds,
            effective_rpo_seconds: rpo.whole_seconds(),
            next_run_at: plan.next_run_at.map(format_time),
            last_fire_time: plan.last_fire_time.map(format_time),
            last_success_at: plan.last_success_at.map(format_time),
            rpo_state,
            parameter_values,
            created_at: format_time(plan.created_at),
            updated_at: format_time(plan.updated_at),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PlanListResponse {
    pub items: Vec<PlanView>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PlanResponse {
    pub plan: PlanView,
}

/// The next fire times of a plan, in UTC (FR-SC-01).
#[derive(Debug, Serialize, ToSchema)]
pub struct NextRunsResponse {
    pub plan_id: String,
    /// The time zone of the plan; the user interface renders each run in it.
    pub time_zone: String,
    /// The fire times in UTC, formatted as RFC 3339.
    pub runs: Vec<String>,
}

/// Lists the plans of the tenant with keyset pagination.
#[utoipa::path(
    get,
    path = "/api/v1/plans",
    security(("cookie" = []), ("bearer" = [])),
    params(PlanListQuery),
    responses(
        (status = 200, description = "The plans of the tenant.", body = PlanListResponse),
        (status = 400, description = "A query parameter is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_plans(
    State(state): State<AppState>,
    context: RequestContext,
    Query(query): Query<PlanListQuery>,
) -> Result<Json<PlanListResponse>, ApiError> {
    context.authorize(Operation::PlansRead)?;
    let page = PlanPage {
        limit: query.limit.unwrap_or(repo::DEFAULT_PAGE_LIMIT),
        after: query
            .cursor
            .as_deref()
            .map(decode_cursor)
            .transpose()
            .map_err(|cause| {
                ApiError::invalid_field_cause("cursor", Reason::InvalidFormat, cause)
            })?,
    };
    let (result, values) = context
        .tenant_tx(&state, async |tx| {
            let result = repo::list(tx, &page).await?;
            let values = views(tx, &result.plans).await?;
            Ok((result, values))
        })
        .await?;
    let may_read_text = parameters::may_read_text(&context);
    let next_cursor = if result.has_more {
        result.plans.last().map(|plan| {
            encode_cursor(PlanCursor {
                created_at: plan.created_at,
                id: plan.id,
            })
        })
    } else {
        None
    };
    let now = state.auth().clock.now();
    let items = result
        .plans
        .iter()
        .map(|plan| {
            let overrides = views_of(&values, plan.id, may_read_text);
            PlanView::of(plan, overrides, now)
        })
        .collect();
    Ok(Json(PlanListResponse { items, next_cursor }))
}

/// Reads one plan of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/plans/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the plan.")),
    responses(
        (status = 200, description = "The plan.", body = PlanView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The plan is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_plan(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<PlanView>, ApiError> {
    context.authorize(Operation::PlansRead)?;
    let plan_id = parse_plan_id(&id)?;
    let (plan, values) = context
        .tenant_tx(&state, async |tx| {
            let plan = repo::get(tx, plan_id).await?;
            let values = match &plan {
                Some(plan) => views(tx, std::slice::from_ref(plan)).await?,
                None => Vec::new(),
            };
            Ok((plan, values))
        })
        .await?;
    let Some(plan) = plan else {
        return Err(ApiError::not_found());
    };
    let may_read_text = parameters::may_read_text(&context);
    let overrides = values
        .iter()
        .map(|stored| PlanParameterValueView::of(stored, may_read_text))
        .collect();
    Ok(Json(PlanView::of(
        &plan,
        overrides,
        state.auth().clock.now(),
    )))
}

/// The stored overrides of a set of plans, in the order of the plan and of the name.
async fn views(
    tx: &mut restichub_db::TenantTx<'_>,
    plans: &[Plan],
) -> Result<Vec<restichub_db::parameters::PlanValue>, ApiError> {
    let ids: Vec<PlanId> = plans.iter().map(|plan| plan.id).collect();
    Ok(restichub_db::parameters::values_of_plans(tx, &ids).await?)
}

/// The overrides of one plan as the reader sees them.
fn views_of(
    values: &[restichub_db::parameters::PlanValue],
    plan_id: PlanId,
    may_read_text: bool,
) -> Vec<PlanParameterValueView> {
    values
        .iter()
        .filter(|stored| stored.plan_id == plan_id)
        .map(|stored| PlanParameterValueView::of(stored, may_read_text))
        .collect()
}

/// Creates a plan and validates its schedule (FR-SC-01, FR-PF-05).
#[utoipa::path(
    post,
    path = "/api/v1/plans",
    summary = "Creates a plan in the tenant.",
    description = "The control plane validates the cron expression, the time zone and the stored profile against the agent before the write (FR-PF-05).",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreatePlanRequest,
    responses(
        (status = 201, description = "The plan is created.", body = PlanResponse),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 409, description = "The name exists.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_plan(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CreatePlanRequest>,
) -> Result<(StatusCode, Json<PlanResponse>), ApiError> {
    context.authorize(Operation::PlansWrite)?;
    let mut input = validate_input(PlanPayload {
        name: Some(body.name),
        profile_id: &body.profile_id,
        agent_id: &body.agent_id,
        repository_id: &body.repository_id,
        cron: &body.cron,
        time_zone: &body.time_zone,
        missed_run_policy: body.missed_run_policy,
        enabled: body.enabled,
    })?;
    input.rpo_seconds = validate_rpo(body.rpo_seconds)?;
    input.parameter_values =
        parameters::validate_values("parameter_values", body.parameter_values)?;
    // Only an Admin sets a secret value, because it chooses the credential that a script receives
    // (ADR-0047). An Operator still saves the text values.
    if parameters::changes_secret(&[], &input.parameter_values) {
        context.authorize(Operation::ParameterSecretWrite)?;
    }
    let now = state.auth().clock.now();
    let schedule = parse_schedule(&input.cron, &input.time_zone)?;
    let plan_id = PlanId::new();
    let created_by = crate::secrets::actor_user(&context.actor);

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    check_references(
        &mut tx,
        input.profile_id,
        input.agent_id,
        input.repository_id,
        &input.parameter_values,
        now,
    )
    .await?;
    let next_run_at = next_run_at(&schedule, input.enabled, now)?;
    repo::create(
        &mut tx,
        &repo::NewPlan {
            id: plan_id,
            name: &input.name,
            profile_id: input.profile_id,
            agent_id: input.agent_id,
            repository_id: input.repository_id,
            cron: &input.cron,
            time_zone: &input.time_zone,
            missed_run_policy: input.missed_run_policy,
            enabled: input.enabled,
            rpo_seconds: input.rpo_seconds,
            next_run_at,
            created_by,
        },
    )
    .await
    .map_err(map_write_error)?;
    restichub_db::parameters::replace_plan_values(
        &mut tx,
        plan_id,
        &input.parameter_values,
        created_by,
    )
    .await?;

    let mut event = context.audit_event(AuditAction::PlanCreate, "plan", plan_id.as_uuid());
    event.diff = Some(diff(None, Some(&plan_value(&input, None)), &[]));
    restichub_db::audit::insert(&mut tx, &event).await?;

    let created = repo::get(&mut tx, plan_id).await?;
    let values = match &created {
        Some(plan) => views(&mut tx, std::slice::from_ref(plan)).await?,
        None => Vec::new(),
    };
    tx.commit().await?;
    let Some(created) = created else {
        return Err(ApiError::internal(
            "the created plan is not found after the read",
        ));
    };
    let may_read_text = parameters::may_read_text(&context);
    Ok((
        StatusCode::CREATED,
        Json(PlanResponse {
            plan: PlanView::of(
                &created,
                views_of(&values, plan_id, may_read_text),
                state.auth().clock.now(),
            ),
        }),
    ))
}

/// Updates a plan. The body replaces the schedule; an absent name keeps the name.
#[utoipa::path(
    patch,
    path = "/api/v1/plans/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the plan.")),
    request_body = UpdatePlanRequest,
    responses(
        (status = 200, description = "The updated plan.", body = PlanResponse),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The plan is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn update_plan(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdatePlanRequest>,
) -> Result<Json<PlanResponse>, ApiError> {
    context.authorize(Operation::PlansWrite)?;
    let plan_id = parse_plan_id(&id)?;
    let now = state.auth().clock.now();
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    // The read locks the row: a concurrent delete cannot remove it between the read and the write.
    let Some(current) = repo::get_for_update(&mut tx, plan_id).await? else {
        return Err(ApiError::not_found());
    };
    // The values are read before the replace: the write removes the old rows, and the audit diff
    // and the check of a secret value need them.
    let before_values = restichub_db::parameters::plan_values(&mut tx, plan_id).await?;
    let mut input = validate_input(PlanPayload {
        name: body.name.or(Some(current.name.clone())),
        profile_id: &body.profile_id,
        agent_id: &body.agent_id,
        repository_id: &body.repository_id,
        cron: &body.cron,
        time_zone: &body.time_zone,
        missed_run_policy: body.missed_run_policy,
        enabled: body.enabled,
    })?;
    input.rpo_seconds = validate_rpo(body.rpo_seconds)?;
    input.parameter_values =
        parameters::validate_values("parameter_values", body.parameter_values)?;
    // Only an Admin sets, changes or removes a secret value, because it chooses the credential that
    // a script receives (ADR-0047). An entry that the body leaves as it was is no change.
    if parameters::changes_secret(&before_values, &input.parameter_values) {
        context.authorize(Operation::ParameterSecretWrite)?;
    }
    let schedule = parse_schedule(&input.cron, &input.time_zone)?;
    check_references(
        &mut tx,
        input.profile_id,
        input.agent_id,
        input.repository_id,
        &input.parameter_values,
        now,
    )
    .await?;
    // A rename or a policy change keeps the pending fire time. The schedule is recomputed only when
    // the cron, the time zone or the enabled flag changes (FR-SC-01).
    let schedule_changed = input.cron != current.cron
        || input.time_zone != current.time_zone
        || input.enabled != current.enabled;
    repo::update(
        &mut tx,
        plan_id,
        &repo::PlanUpdate {
            name: &input.name,
            profile_id: input.profile_id,
            agent_id: input.agent_id,
            repository_id: input.repository_id,
            cron: &input.cron,
            time_zone: &input.time_zone,
            missed_run_policy: input.missed_run_policy,
            enabled: input.enabled,
            rpo_seconds: input.rpo_seconds,
        },
    )
    .await
    .map_err(map_write_error)?;
    restichub_db::parameters::replace_plan_values(
        &mut tx,
        plan_id,
        &input.parameter_values,
        crate::secrets::actor_user(&context.actor),
    )
    .await?;
    if schedule_changed {
        // The edited schedule starts from the next fire time after now.
        repo::set_next_run_at(
            &mut tx,
            plan_id,
            next_run_at(&schedule, input.enabled, now)?,
        )
        .await?;
    }

    let mut event = context.audit_event(AuditAction::PlanUpdate, "plan", plan_id.as_uuid());
    event.diff = Some(diff(
        Some(&stored_value(&current, &before_values)),
        Some(&plan_value(&input, Some(&before_values))),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;

    let updated = repo::get(&mut tx, plan_id).await?;
    let values = match &updated {
        Some(plan) => views(&mut tx, std::slice::from_ref(plan)).await?,
        None => Vec::new(),
    };
    tx.commit().await?;
    let Some(updated) = updated else {
        return Err(ApiError::internal(
            "the updated plan is not found after the read",
        ));
    };
    let may_read_text = parameters::may_read_text(&context);
    Ok(Json(PlanResponse {
        plan: PlanView::of(
            &updated,
            views_of(&values, plan_id, may_read_text),
            state.auth().clock.now(),
        ),
    }))
}

/// Removes a plan.
#[utoipa::path(
    delete,
    path = "/api/v1/plans/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the plan.")),
    responses(
        (status = 204, description = "The plan is removed."),
        crate::error::ProtectedResponses,
        (status = 404, description = "The plan is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn delete_plan(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::PlansWrite)?;
    let plan_id = parse_plan_id(&id)?;
    let now = state.auth().clock.now();
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(current) = repo::get_for_update(&mut tx, plan_id).await? else {
        return Err(ApiError::not_found());
    };
    // The queued jobs of the plan end as cancelled in the delete transaction, so no queued job
    // outlives its plan (FR-SC-07). A sent or running job keeps its lifecycle.
    restichub_db::jobs::cancel_for_plan(&mut tx, plan_id, now).await?;
    // The values are read before the delete: the foreign key removes them with the plan.
    let values = restichub_db::parameters::plan_values(&mut tx, plan_id).await?;
    repo::delete(&mut tx, plan_id).await?;
    let mut event = context.audit_event(AuditAction::PlanDelete, "plan", plan_id.as_uuid());
    event.diff = Some(diff(Some(&stored_value(&current, &values)), None, &[]));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Returns the next fire times of a plan, in UTC (FR-SC-01).
#[utoipa::path(
    get,
    path = "/api/v1/plans/{id}/next-runs",
    summary = "Previews the next fire times of a plan.",
    description = "The fire times are in UTC; the user interface renders them in the plan time zone.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the plan."), NextRunsQuery),
    responses(
        (status = 200, description = "The next fire times.", body = NextRunsResponse),
        (status = 400, description = "A query parameter is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The plan is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn next_runs(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    Query(query): Query<NextRunsQuery>,
) -> Result<Json<NextRunsResponse>, ApiError> {
    context.authorize(Operation::PlansRead)?;
    let plan_id = parse_plan_id(&id)?;
    let count = query.count.unwrap_or(DEFAULT_PREVIEW_RUNS);
    if count == 0 || count > MAX_PREVIEW_RUNS {
        return Err(ApiError::invalid_field("count", Reason::TooLong));
    }
    let plan = context
        .tenant_tx(&state, async |tx| Ok(repo::get(tx, plan_id).await?))
        .await?;
    let Some(plan) = plan else {
        return Err(ApiError::not_found());
    };
    let schedule = parse_schedule(&plan.cron, &plan.time_zone)?;
    let now = state.auth().clock.now();
    let runs = schedule
        .next_runs(now, count)
        .map_err(|error| schedule_error_to_api("cron", error))?;
    Ok(Json(NextRunsResponse {
        plan_id: plan.id.to_string(),
        time_zone: plan.time_zone.clone(),
        runs: runs.into_iter().map(format_time).collect(),
    }))
}

/// The request body of the schedule preview (FR-SC-01).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewScheduleRequest {
    pub cron: String,
    pub time_zone: String,
    #[serde(default = "default_preview_count")]
    pub count: usize,
}

fn default_preview_count() -> usize {
    DEFAULT_PREVIEW_RUNS
}

/// The next fire times of an unsaved schedule, in UTC (FR-SC-01).
#[derive(Debug, Serialize, ToSchema)]
pub struct SchedulePreviewResponse {
    /// The time zone that the user interface renders each run in.
    pub time_zone: String,
    /// The fire times in UTC, formatted as RFC 3339.
    pub runs: Vec<String>,
}

/// Previews the next fire times of an unsaved schedule (FR-SC-01).
///
/// The create dialog calls it before the save, so it needs the Plans read right only. The count is
/// bounded by `MAX_PREVIEW_RUNS`.
#[utoipa::path(
    post,
    path = "/api/v1/plans/preview",
    summary = "Previews the next fire times of a schedule.",
    description = "The fire times are in UTC; the user interface renders them in the given time zone.",
    security(("cookie" = []), ("bearer" = [])),
    request_body = PreviewScheduleRequest,
    responses(
        (status = 200, description = "The next fire times.", body = SchedulePreviewResponse),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
    )
)]
pub async fn preview_schedule(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<PreviewScheduleRequest>,
) -> Result<Json<SchedulePreviewResponse>, ApiError> {
    context.authorize(Operation::PlansRead)?;
    if body.count == 0 || body.count > MAX_PREVIEW_RUNS {
        return Err(ApiError::invalid_field("count", Reason::TooLong));
    }
    let cron = body.cron.trim().to_owned();
    let time_zone = body.time_zone.trim().to_owned();
    let schedule = Schedule::parse(&cron, &time_zone)
        .map_err(|error| schedule_error_to_api(profile_field(error), error))?;
    let now = state.auth().clock.now();
    let runs = schedule
        .next_runs(now, body.count)
        .map_err(|error| schedule_error_to_api("cron", error))?;
    Ok(Json(SchedulePreviewResponse {
        time_zone,
        runs: runs.into_iter().map(format_time).collect(),
    }))
}

/// The validated fields of a plan create or update.
struct PlanInput {
    name: String,
    profile_id: ProfileId,
    agent_id: AgentId,
    repository_id: RepositoryId,
    cron: String,
    time_zone: String,
    missed_run_policy: MissedRunPolicy,
    enabled: bool,
    rpo_seconds: Option<i32>,
    /// What this plan sets instead of the values on the host (FR-PF-10).
    parameter_values: Vec<restichub_db::parameters::ParameterValue>,
}

/// The smallest and the largest recovery point objective, in seconds (ADR-0036).
const MIN_RPO_SECONDS: i32 = 900;
const MAX_RPO_SECONDS: i32 = 34_560_000;

fn validate_rpo(value: Option<i32>) -> Result<Option<i32>, ApiError> {
    match value {
        Some(seconds) if seconds < MIN_RPO_SECONDS => {
            Err(ApiError::invalid_field("rpo_seconds", Reason::TooShort))
        }
        Some(seconds) if seconds > MAX_RPO_SECONDS => {
            Err(ApiError::invalid_field("rpo_seconds", Reason::TooLong))
        }
        other => Ok(other),
    }
}

/// The request fields that a plan create and a plan update share.
struct PlanPayload<'a> {
    name: Option<String>,
    profile_id: &'a str,
    agent_id: &'a str,
    repository_id: &'a str,
    cron: &'a str,
    time_zone: &'a str,
    missed_run_policy: MissedRunPolicy,
    enabled: bool,
}

fn validate_input(payload: PlanPayload<'_>) -> Result<PlanInput, ApiError> {
    let name = crate::validation::validate_record_name("name", &payload.name.unwrap_or_default())?;
    let profile_id = parse_id("profile_id", payload.profile_id, ProfileId::from_uuid)?;
    let agent_id = parse_id("agent_id", payload.agent_id, AgentId::from_uuid)?;
    // The plan always names its repository, so a request without one is refused here (ADR-0047).
    if payload.repository_id.trim().is_empty() {
        return Err(ApiError::invalid_field("repository_id", Reason::Empty));
    }
    let repository_id = parse_id(
        "repository_id",
        payload.repository_id,
        RepositoryId::from_uuid,
    )?;
    let cron = payload.cron.trim().to_owned();
    let time_zone = payload.time_zone.trim().to_owned();
    // A schedule that the cron parser refuses names the fields here, so the handler and the preview
    // agree on the reason.
    Schedule::parse(&cron, &time_zone)
        .map_err(|error| schedule_error_to_api(profile_field(error), error))?;
    Ok(PlanInput {
        name,
        profile_id,
        agent_id,
        repository_id,
        cron,
        time_zone,
        missed_run_policy: payload.missed_run_policy,
        enabled: payload.enabled,
        rpo_seconds: None,
        parameter_values: Vec::new(),
    })
}

/// Names the invalid field of a schedule error at save time.
fn profile_field(error: ScheduleError) -> &'static str {
    match error {
        ScheduleError::Cron => "cron",
        ScheduleError::TimeZone => "time_zone",
        ScheduleError::Search => "cron",
    }
}

pub(crate) fn parse_schedule(cron: &str, time_zone: &str) -> Result<Schedule, ApiError> {
    Schedule::parse(cron, time_zone)
        .map_err(|error| schedule_error_to_api(profile_field(error), error))
}

fn schedule_error_to_api(field: &'static str, error: ScheduleError) -> ApiError {
    match error {
        ScheduleError::Cron => ApiError::invalid_field(field, Reason::InvalidFormat),
        ScheduleError::TimeZone => ApiError::invalid_field(field, Reason::Unknown),
        // A cron expression that names no instant, such as `0 0 31 2 *`, fails the search for the
        // first fire time. The save-time check refuses it with the cron field (FR-SC-01).
        ScheduleError::Search => ApiError::invalid_field(field, Reason::InvalidFormat),
    }
}

/// The next fire time for an enabled plan. A disabled plan holds none.
pub(crate) fn next_run_at(
    schedule: &Schedule,
    enabled: bool,
    now: time::OffsetDateTime,
) -> Result<Option<time::OffsetDateTime>, ApiError> {
    if !enabled {
        return Ok(None);
    }
    schedule
        .next_after(now)
        .map(Some)
        .map_err(|error| schedule_error_to_api("cron", error))
}

/// Refuses a profile, an agent or a repository that is not in the tenant, an agent that is revoked
/// (FR-TN-01, FR-ID-04), and an agent other than the host of a local repository (ADR-0046).
///
/// The function also keeps the plan complete (FR-PF-11): every submitted value must name a declared
/// parameter with the declared kind, a secret value must name a secret of the purpose `hook_secret`,
/// and each required parameter must have a value from the plan, the host or the default.
async fn check_references(
    tx: &mut restichub_db::TenantTx<'_>,
    profile_id: ProfileId,
    agent_id: AgentId,
    repository_id: RepositoryId,
    plan_values: &[restichub_db::parameters::ParameterValue],
    now: time::OffsetDateTime,
) -> Result<(), ApiError> {
    // The repository row is locked for update, so its host cannot change between this check and the
    // write: the repository update takes the same lock first. The locks of a plan write come in the
    // order plan, profile, agent, repository.
    let profile = match profiles_repo::get_for_share(tx, profile_id).await? {
        Some(profile) => profile,
        None => return Err(ApiError::invalid_field("profile_id", Reason::Unknown)),
    };
    // The key share lock keeps the host for the check of the values and of the compile: a change of
    // its values waits for this transaction (FR-PF-11).
    let agent = agents_repo::get_for_key_share(tx, agent_id).await?;
    let Some(agent) = agent else {
        return Err(ApiError::invalid_field("agent_id", Reason::Unknown));
    };
    if agent.status != AgentStatus::Active {
        return Err(ApiError::invalid_field("agent_id", Reason::Unknown));
    }
    let Some(repository) = restichub_db::repositories::get_for_update(tx, repository_id).await?
    else {
        return Err(ApiError::invalid_field("repository_id", Reason::Unknown));
    };
    // A local repository lives on the disk of its repository host, so a plan of it runs there only
    // (ADR-0046, ADR-0047).
    crate::jobs::ensure_repository_host(&repository, agent_id)?;
    let definitions = parameters::from_row(&profile.parameters)?;
    // A name cannot have two kinds on one host, because the host holds one value for each name
    // (ADR-0047).
    crate::parameters::check_kinds_of_host(
        tx,
        agent_id,
        profile_id,
        "parameter_values",
        &definitions,
    )
    .await?;
    crate::parameters::check_declared_values(&definitions, plan_values, "parameter_values")?;
    crate::parameters::check_secret_values(tx, plan_values, "parameter_values").await?;
    let host_values = restichub_db::parameters::agent_values(tx, agent_id).await?;
    // The value check runs before the compile, so the refusal names every parameter that a source
    // misses (FR-PF-11, AC-PF-06).
    let missing = parameters::missing_required(&definitions, plan_values, &host_values);
    if !missing.is_empty() {
        return Err(ApiError::parameters(
            "parameter_values",
            Reason::ParameterMissing,
            missing,
        ));
    }
    let resolved = parameters::resolve(&definitions, plan_values, &host_values);
    // The stored profile must compile for the operating system of the agent (FR-PF-05). The compile
    // refuses a missing path for that operating system and a parameter that has no value with a
    // field error.
    crate::profiles::compile_for_agent(&profile, repository_id, &agent, resolved, now)?;
    Ok(())
}

fn parse_id<T>(field: &'static str, value: &str, wrap: fn(uuid::Uuid) -> T) -> Result<T, ApiError> {
    uuid::Uuid::parse_str(value.trim())
        .map(wrap)
        .map_err(|error| {
            ApiError::invalid_field_cause(field, Reason::InvalidFormat, error.to_string())
        })
}

pub(crate) fn parse_plan_id(value: &str) -> Result<PlanId, ApiError> {
    parse_id("id", value, PlanId::from_uuid)
}

fn encode_cursor(cursor: PlanCursor) -> String {
    format!("{};{}", cursor.created_at.unix_timestamp_nanos(), cursor.id)
}

fn decode_cursor(value: &str) -> Result<PlanCursor, String> {
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
    Ok(PlanCursor {
        created_at,
        id: PlanId::from_uuid(id),
    })
}

fn plan_value(
    input: &PlanInput,
    previous: Option<&[restichub_db::parameters::ParameterValue]>,
) -> serde_json::Value {
    json!({
        "parameter_values": parameters::audit_values(&input.parameter_values, previous),
        "name": input.name,
        "profile_id": input.profile_id.to_string(),
        "agent_id": input.agent_id.to_string(),
        "repository_id": input.repository_id.to_string(),
        "cron": input.cron,
        "time_zone": input.time_zone,
        "missed_run_policy": input.missed_run_policy.as_str(),
        "enabled": input.enabled,
        "rpo_seconds": input.rpo_seconds,
    })
}

fn stored_value(
    plan: &Plan,
    parameter_values: &[restichub_db::parameters::ParameterValue],
) -> serde_json::Value {
    json!({
        "parameter_values": parameters::audit_values(parameter_values, None),
        "name": plan.name,
        "profile_id": plan.profile_id.to_string(),
        "agent_id": plan.agent_id.to_string(),
        "repository_id": plan.repository_id.to_string(),
        "cron": plan.cron,
        "time_zone": plan.time_zone,
        "missed_run_policy": plan.missed_run_policy.as_str(),
        "enabled": plan.enabled,
        "rpo_seconds": plan.rpo_seconds,
    })
}

fn format_time(value: time::OffsetDateTime) -> String {
    use time::format_description::well_known::Rfc3339;
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}
