//! The read queries behind the alert evaluation and the dashboard (FR-MN-01, FR-MN-03, ADR-0036).
//!
//! Every query runs under the tenant policy, so a transaction sees the rows of its own tenant only.

use time::OffsetDateTime;

use crate::DbError;
use crate::enums::{AgentStatus, JobKind, JobStatus};
use crate::ids::{AgentId, JobId, PlanId, RepositoryId};
use crate::tx::TenantTx;

/// The rows that one query of this module takes at a time. The public functions read every row,
/// page by page, so a tenant with many rows is evaluated completely.
const PAGE: i64 = 500;

/// A plan with the facts that the recovery point objective needs.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RpoPlan {
    pub id: PlanId,
    pub name: String,
    pub cron: String,
    pub time_zone: String,
    pub enabled: bool,
    pub rpo_seconds: Option<i32>,
    pub created_at: OffsetDateTime,
    pub agent_id: AgentId,
    pub agent_hostname: String,
    pub agent_status: AgentStatus,
    pub repository_id: RepositoryId,
    pub repository_name: String,
    /// The finish time of the latest succeeded backup job of the plan.
    pub last_success_at: Option<OffsetDateTime>,
    pub last_success_job_id: Option<JobId>,
}

/// Lists the plans of the tenant with the time of the last successful backup.
pub async fn rpo_plans(tx: &mut TenantTx<'_>) -> Result<Vec<RpoPlan>, DbError> {
    let mut all = Vec::new();
    loop {
        let after = all.last().map(|plan: &RpoPlan| (plan.created_at, plan.id));
        let page = rpo_plans_page(tx, after).await?;
        let full = page.len() as i64 == PAGE;
        all.extend(page);
        if !full {
            return Ok(all);
        }
    }
}

async fn rpo_plans_page(
    tx: &mut TenantTx<'_>,
    after: Option<(OffsetDateTime, PlanId)>,
) -> Result<Vec<RpoPlan>, DbError> {
    sqlx::query_as::<_, RpoPlan>(
        "SELECT p.id, p.name, p.cron, p.time_zone, p.enabled, p.rpo_seconds, p.created_at, \
                p.agent_id, a.hostname AS agent_hostname, a.status AS agent_status, \
                pr.repository_id, r.name AS repository_name, s.finished_at AS last_success_at, \
                s.id AS last_success_job_id \
         FROM plans p \
         JOIN agents a ON a.tenant_id = p.tenant_id AND a.id = p.agent_id \
         JOIN profiles pr ON pr.tenant_id = p.tenant_id AND pr.id = p.profile_id \
         JOIN repositories r ON r.tenant_id = p.tenant_id AND r.id = pr.repository_id \
         LEFT JOIN LATERAL ( \
             SELECT j.id, j.finished_at FROM jobs j \
             WHERE j.plan_id = p.id AND j.kind = 'backup' AND j.status = 'succeeded' \
               AND j.finished_at IS NOT NULL \
             ORDER BY j.finished_at DESC LIMIT 1) s ON true \
         WHERE ($1::timestamptz IS NULL OR (p.created_at, p.id) > ($1::timestamptz, $2::uuid)) \
         ORDER BY p.created_at, p.id LIMIT $3",
    )
    .bind(after.map(|(time, _)| time))
    .bind(after.map(|(_, id)| id))
    .bind(PAGE)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// An agent with the facts that the offline alert and the dashboard need.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentFacts {
    pub id: AgentId,
    pub hostname: String,
    pub status: AgentStatus,
    pub last_seen_at: Option<OffsetDateTime>,
    pub first_connected_at: Option<OffsetDateTime>,
    pub stream_open: bool,
    pub created_at: OffsetDateTime,
}

impl AgentFacts {
    /// The time of the last sign of life. An agent that never connected counts from its creation.
    pub fn last_contact(&self) -> OffsetDateTime {
        self.last_seen_at.unwrap_or(self.created_at)
    }
}

pub async fn agent_facts(tx: &mut TenantTx<'_>) -> Result<Vec<AgentFacts>, DbError> {
    let mut all = Vec::new();
    loop {
        let after = all
            .last()
            .map(|agent: &AgentFacts| (agent.hostname.clone(), agent.id));
        let page = agent_facts_page(tx, after).await?;
        let full = page.len() as i64 == PAGE;
        all.extend(page);
        if !full {
            return Ok(all);
        }
    }
}

async fn agent_facts_page(
    tx: &mut TenantTx<'_>,
    after: Option<(String, AgentId)>,
) -> Result<Vec<AgentFacts>, DbError> {
    sqlx::query_as::<_, AgentFacts>(
        "SELECT id, hostname, status, last_seen_at, first_connected_at, \
                stream_id IS NOT NULL AS stream_open, created_at FROM agents \
         WHERE ($1::text IS NULL OR (hostname, id) > ($1::text, $2::uuid)) \
         ORDER BY hostname, id LIMIT $3",
    )
    .bind(after.as_ref().map(|(name, _)| name.as_str()))
    .bind(after.as_ref().map(|(_, id)| *id))
    .bind(PAGE)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RepositoryLockFacts {
    pub id: RepositoryId,
    pub name: String,
    /// True when a job that waits or runs reported a held lock.
    pub lock_held: bool,
    /// The earliest time at which a waiting job found the lock stale.
    pub stale_since: Option<OffsetDateTime>,
    pub stale_job_id: Option<JobId>,
    pub stale_agent_id: Option<AgentId>,
    pub stale_agent_hostname: Option<String>,
}

pub async fn repository_locks(tx: &mut TenantTx<'_>) -> Result<Vec<RepositoryLockFacts>, DbError> {
    let mut all = Vec::new();
    loop {
        let after = all
            .last()
            .map(|repository: &RepositoryLockFacts| (repository.name.clone(), repository.id));
        let page = repository_locks_page(tx, after).await?;
        let full = page.len() as i64 == PAGE;
        all.extend(page);
        if !full {
            return Ok(all);
        }
    }
}

async fn repository_locks_page(
    tx: &mut TenantTx<'_>,
    after: Option<(String, RepositoryId)>,
) -> Result<Vec<RepositoryLockFacts>, DbError> {
    sqlx::query_as::<_, RepositoryLockFacts>(
        "SELECT r.id, r.name, \
                EXISTS (SELECT 1 FROM jobs j WHERE j.repository_id = r.id \
                          AND j.status IN ('queued', 'sent', 'running') \
                          AND (j.lock_reported OR j.attempt > 0 OR j.stale_lock_since IS NOT NULL)) \
                    AS lock_held, \
                s.stale_lock_since AS stale_since, s.id AS stale_job_id, \
                s.agent_id AS stale_agent_id, a.hostname AS stale_agent_hostname \
         FROM repositories r \
         LEFT JOIN LATERAL ( \
             SELECT j.id, j.agent_id, j.stale_lock_since FROM jobs j \
             WHERE j.repository_id = r.id AND j.stale_lock_since IS NOT NULL \
               AND j.status IN ('queued', 'sent', 'running') \
             ORDER BY j.stale_lock_since, j.id LIMIT 1) s ON true \
         LEFT JOIN agents a ON a.tenant_id = r.tenant_id AND a.id = s.agent_id \
         WHERE ($1::text IS NULL OR (r.name, r.id) > ($1::text, $2::uuid)) \
         ORDER BY r.name, r.id LIMIT $3",
    )
    .bind(after.as_ref().map(|(name, _)| name.as_str()))
    .bind(after.as_ref().map(|(_, id)| *id))
    .bind(PAGE)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// The latest finished job of one subject: a plan, or a repository and kind, or an agent and kind.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LatestJob {
    pub subject: String,
    pub id: JobId,
    pub kind: JobKind,
    pub status: JobStatus,
    pub result_code: Option<String>,
    pub result_message: Option<String>,
    pub agent_id: AgentId,
    pub agent_hostname: String,
    pub plan_id: Option<PlanId>,
    pub plan_name: Option<String>,
    pub repository_id: Option<RepositoryId>,
    pub repository_name: Option<String>,
    pub finished_at: OffsetDateTime,
}

/// Finds what each plan, repository or agent did last. A cancelled and an expired job do not count.
/// Only the jobs that finished after `since` count, so the query stays bounded.
pub async fn latest_jobs(
    tx: &mut TenantTx<'_>,
    since: OffsetDateTime,
) -> Result<Vec<LatestJob>, DbError> {
    let mut all = Vec::new();
    loop {
        let after = all.last().map(|job: &LatestJob| job.subject.clone());
        let page = latest_jobs_page(tx, since, after).await?;
        let full = page.len() as i64 == PAGE;
        all.extend(page);
        if !full {
            return Ok(all);
        }
    }
}

macro_rules! subject_expr {
    () => {
        "COALESCE('plan:' || j.plan_id::text, \
                                    'repo:' || j.repository_id::text || ':' || j.kind, \
                                    'agent:' || j.agent_id::text || ':' || j.kind)"
    };
}

async fn latest_jobs_page(
    tx: &mut TenantTx<'_>,
    since: OffsetDateTime,
    after: Option<String>,
) -> Result<Vec<LatestJob>, DbError> {
    sqlx::query_as::<_, LatestJob>(concat!(
        "SELECT * FROM ( \
             SELECT DISTINCT ON (subject) \
                    ",
        subject_expr!(),
        " AS subject, \
                    j.id, j.kind, j.status, j.result_code, j.result_message, j.agent_id, \
                    a.hostname AS agent_hostname, j.plan_id, p.name AS plan_name, \
                    j.repository_id, r.name AS repository_name, \
                    COALESCE(j.finished_at, j.updated_at) AS finished_at \
             FROM jobs j \
             JOIN agents a ON a.tenant_id = j.tenant_id AND a.id = j.agent_id \
             LEFT JOIN plans p ON p.tenant_id = j.tenant_id AND p.id = j.plan_id \
             LEFT JOIN repositories r ON r.tenant_id = j.tenant_id AND r.id = j.repository_id \
             WHERE j.status IN ('succeeded', 'failed', 'dead_letter') \
               AND j.result_code IS DISTINCT FROM 'cancelled' \
               AND COALESCE(j.finished_at, j.updated_at) > $1 \
               AND ($2::text IS NULL OR ",
        subject_expr!(),
        " > $2) \
             ORDER BY subject, COALESCE(j.finished_at, j.updated_at) DESC, j.id DESC) latest \
         ORDER BY subject LIMIT $3"
    ))
    .bind(since)
    .bind(after)
    .bind(PAGE)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FailedJob {
    pub id: JobId,
    pub kind: JobKind,
    pub status: JobStatus,
    pub result_code: Option<String>,
    pub agent_hostname: String,
    pub plan_name: Option<String>,
    pub repository_name: Option<String>,
    pub finished_at: OffsetDateTime,
}

/// Lists the failed and dead letter jobs that finished after `since`, newest first.
pub async fn recent_failed_jobs(
    tx: &mut TenantTx<'_>,
    since: OffsetDateTime,
    limit: i64,
) -> Result<Vec<FailedJob>, DbError> {
    sqlx::query_as::<_, FailedJob>(
        "SELECT j.id, j.kind, j.status, j.result_code, a.hostname AS agent_hostname, \
                p.name AS plan_name, r.name AS repository_name, \
                COALESCE(j.finished_at, j.updated_at) AS finished_at \
         FROM jobs j \
         JOIN agents a ON a.tenant_id = j.tenant_id AND a.id = j.agent_id \
         LEFT JOIN plans p ON p.tenant_id = j.tenant_id AND p.id = j.plan_id \
         LEFT JOIN repositories r ON r.tenant_id = j.tenant_id AND r.id = j.repository_id \
         WHERE j.status IN ('failed', 'dead_letter') \
           AND j.result_code IS DISTINCT FROM 'cancelled' \
           AND COALESCE(j.finished_at, j.updated_at) > $1 \
         ORDER BY COALESCE(j.finished_at, j.updated_at) DESC, j.id DESC LIMIT $2",
    )
    .bind(since)
    .bind(limit)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}
