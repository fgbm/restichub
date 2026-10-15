//! Dashboard overview (FR-MN-01, ADR-0035).
//!
//! One read gives the overview of the tenant: the hosts that are online and offline, each plan with
//! the time of its last successful backup and its RPO state, the repositories with the state of
//! their lock, the recent failed jobs and the open alerts.
//!
//! A host counts as online when the control plane saw it within [`ONLINE_WINDOW`]. The test reads
//! `agents.last_seen_at`, not the stream registry of one instance, so each instance answers alike.

use axum::Json;
use axum::extract::State;
use restichub_db::alerts as alerts_repo;
use restichub_db::dashboard as repo;
use restichub_db::{AgentStatus, JobKind, JobStatus};
use serde::Serialize;
use time::{Duration, OffsetDateTime};
use utoipa::ToSchema;

use crate::alerts::rpo::{self, RpoState};
use crate::alerts_api::{AlertView, views};
use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::server::AppState;

/// The time within which a host counts as online. The agent streams flush `last_seen_at` every 30
/// seconds, so the window holds three flushes.
pub const ONLINE_WINDOW: Duration = Duration::seconds(90);

/// Failed jobs older than this stay out of the list.
const FAILED_WINDOW: Duration = Duration::days(7);

const FAILED_JOBS: i64 = 10;
const OPEN_ALERTS: usize = 10;
const LIST_LIMIT: usize = 50;

#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardHost {
    pub id: String,
    pub hostname: String,
    pub online: bool,
    pub last_seen_at: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardHosts {
    pub total: usize,
    pub online: usize,
    pub offline: usize,
    /// The offline hosts first, then the others, at most 50.
    pub items: Vec<DashboardHost>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardPlan {
    pub id: String,
    pub name: String,
    pub agent_hostname: String,
    pub repository_name: String,
    pub enabled: bool,
    /// The finish time of the last succeeded backup. Null when the plan never succeeded.
    pub last_success_at: Option<String>,
    /// The seconds since the last success, or since the creation of a plan that never succeeded.
    pub age_seconds: i64,
    pub rpo_seconds: i64,
    pub state: RpoState,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardPlans {
    pub total: usize,
    pub at_risk: usize,
    pub violated: usize,
    /// The violated plans first, then the at risk plans, at most 50.
    pub items: Vec<DashboardPlan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LockState {
    Free,
    /// A job that waits or runs reported a held lock.
    Held,
    /// A waiting job found the lock stale (FR-SC-06).
    Stale,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardRepository {
    pub id: String,
    pub name: String,
    pub lock_state: LockState,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardFailedJob {
    pub id: String,
    pub kind: JobKind,
    pub status: JobStatus,
    pub result_code: Option<String>,
    pub agent_hostname: String,
    pub plan_name: Option<String>,
    pub repository_name: Option<String>,
    pub finished_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardAlerts {
    pub total: usize,
    /// Newest first, at most 10.
    pub items: Vec<AlertView>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardResponse {
    pub generated_at: String,
    pub hosts: DashboardHosts,
    pub plans: DashboardPlans,
    pub repositories: Vec<DashboardRepository>,
    pub failed_jobs: Vec<DashboardFailedJob>,
    pub alerts: DashboardAlerts,
}

/// Reads the overview of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/dashboard",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The overview of the tenant.", body = DashboardResponse),
        crate::error::ProtectedResponses,
    )
)]
pub async fn dashboard(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<DashboardResponse>, ApiError> {
    context.authorize(Operation::DashboardRead)?;
    let now = state.auth().clock.now();
    let (agents, plans, repositories, failed, open) = context
        .tenant_tx(&state, async |tx| {
            Ok((
                repo::agent_facts(tx).await?,
                repo::rpo_plans(tx).await?,
                repo::repository_locks(tx).await?,
                repo::recent_failed_jobs(tx, now - FAILED_WINDOW, FAILED_JOBS).await?,
                alerts_repo::open_alerts(tx).await?,
            ))
        })
        .await?;

    let mut hosts: Vec<DashboardHost> = agents
        .iter()
        .filter(|agent| agent.status == AgentStatus::Active)
        .map(|agent| DashboardHost {
            id: agent.id.to_string(),
            hostname: agent.hostname.clone(),
            online: agent
                .last_seen_at
                .is_some_and(|seen| now - seen <= ONLINE_WINDOW),
            last_seen_at: agent.last_seen_at.map(format_time),
        })
        .collect();
    let online = hosts.iter().filter(|host| host.online).count();
    let total = hosts.len();
    hosts.sort_by(|a, b| {
        a.online
            .cmp(&b.online)
            .then_with(|| a.hostname.cmp(&b.hostname))
    });
    hosts.truncate(LIST_LIMIT);

    let mut dashboard_plans: Vec<DashboardPlan> = plans
        .iter()
        .filter(|plan| plan.agent_status == AgentStatus::Active)
        .map(|plan| {
            let status = rpo::evaluate(plan, now);
            DashboardPlan {
                id: plan.id.to_string(),
                name: plan.name.clone(),
                agent_hostname: plan.agent_hostname.clone(),
                repository_name: plan.repository_name.clone(),
                enabled: plan.enabled,
                last_success_at: plan.last_success_at.map(format_time),
                age_seconds: status.age.whole_seconds(),
                rpo_seconds: status.rpo.whole_seconds(),
                state: status.state,
            }
        })
        .collect();
    let plan_total = dashboard_plans.len();
    // A disabled plan raises no alert, so it does not count as at risk or violated.
    let counted = |state: RpoState| {
        dashboard_plans
            .iter()
            .filter(|plan| plan.enabled && plan.state == state)
            .count()
    };
    let (at_risk, violated) = (counted(RpoState::AtRisk), counted(RpoState::Violated));
    dashboard_plans.sort_by_key(|plan| {
        let rank = match (plan.enabled, plan.state) {
            (false, _) => 3,
            (true, RpoState::Violated) => 0,
            (true, RpoState::AtRisk) => 1,
            (true, RpoState::Ok) => 2,
        };
        (rank, std::cmp::Reverse(plan.age_seconds))
    });
    dashboard_plans.truncate(LIST_LIMIT);

    let mut dashboard_repositories: Vec<DashboardRepository> = repositories
        .iter()
        .map(|repository| DashboardRepository {
            id: repository.id.to_string(),
            name: repository.name.clone(),
            lock_state: if repository.stale_since.is_some() {
                LockState::Stale
            } else if repository.lock_held {
                LockState::Held
            } else {
                LockState::Free
            },
        })
        .collect();
    dashboard_repositories.sort_by_key(|repository| match repository.lock_state {
        LockState::Stale => 0,
        LockState::Held => 1,
        LockState::Free => 2,
    });
    dashboard_repositories.truncate(LIST_LIMIT);

    let alert_total = open.len();
    let newest: Vec<_> = open.into_iter().rev().take(OPEN_ALERTS).collect();

    Ok(Json(DashboardResponse {
        generated_at: format_time(now),
        hosts: DashboardHosts {
            total,
            online,
            offline: total - online,
            items: hosts,
        },
        plans: DashboardPlans {
            total: plan_total,
            at_risk,
            violated,
            items: dashboard_plans,
        },
        repositories: dashboard_repositories,
        failed_jobs: failed
            .iter()
            .map(|job| DashboardFailedJob {
                id: job.id.to_string(),
                kind: job.kind,
                status: job.status,
                result_code: job.result_code.clone(),
                agent_hostname: job.agent_hostname.clone(),
                plan_name: job.plan_name.clone(),
                repository_name: job.repository_name.clone(),
                finished_at: format_time(job.finished_at),
            })
            .collect(),
        alerts: DashboardAlerts {
            total: alert_total,
            items: views(&state, &newest),
        },
    }))
}

fn format_time(value: OffsetDateTime) -> String {
    use time::format_description::well_known::Rfc3339;
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}
