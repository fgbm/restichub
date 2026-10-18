//! The plan rows of a tenant (FR-SC-01, FR-SC-07, FR-SC-11, ADR-0010).
//!
//! One row binds a profile to one agent and one repository with a cron schedule and a time zone
//! (FR-SC-01, ADR-0047). The scheduler reads the due rows and creates one job for each fire time;
//! the unique key
//! `(plan_id, fire_time)` on the `jobs` table makes the create idempotent (FR-SC-02).
//!
//! The row holds no secret and no compiled command: a scheduled job names the plan and the fire
//! time only, and the control plane recompiles the stored profile when it dispatches.
//!
//! The general tenant policy of the `plans` table shows the rows of the tenant of the transaction
//! only (ADR-0023, section Tenant isolation).

use time::OffsetDateTime;

use crate::DbError;
use crate::enums::{AgentStatus, MissedRunPolicy};
use crate::ids::{AgentId, PlanId, ProfileId, RepositoryId, UserId};
use crate::referrers::Referrers;
use crate::tx::TenantTx;

pub const DEFAULT_PAGE_LIMIT: i64 = 50;

pub const MAX_PAGE_LIMIT: i64 = 200;

// `jobs_plan_last_success_idx` (0039) serves the lateral read of the last success.
macro_rules! plan_select {
    () => {
        "SELECT p.id, p.name, p.profile_id, p.agent_id, p.repository_id, p.cron, p.time_zone, \
                p.missed_run_policy, p.enabled, p.next_run_at, p.last_fire_time, p.created_at, \
                p.updated_at, p.created_by, pr.name AS profile_name, \
                a.hostname AS agent_hostname, a.status AS agent_status, p.rpo_seconds, \
                s.finished_at AS last_success_at \
         FROM plans p \
         JOIN profiles pr ON pr.tenant_id = p.tenant_id AND pr.id = p.profile_id \
         JOIN agents a ON a.tenant_id = p.tenant_id AND a.id = p.agent_id \
         LEFT JOIN LATERAL ( \
             SELECT j.finished_at FROM jobs j \
             WHERE j.tenant_id = p.tenant_id AND j.plan_id = p.id AND j.kind = 'backup' \
               AND j.status = 'succeeded' AND j.finished_at IS NOT NULL \
             ORDER BY j.finished_at DESC LIMIT 1) s ON true"
    };
}

/// A schedule that binds a profile to one agent and one repository, with a cron expression and a
/// time zone (FR-SC-01, ADR-0010, ADR-0047).
///
/// The names of the profile and of the agent are read with the row, so the list needs no second
/// lookup. They are display values, not part of the identity of the plan.
#[derive(Debug, Clone)]
pub struct Plan {
    pub id: PlanId,
    pub name: String,
    pub profile_id: ProfileId,
    pub agent_id: AgentId,
    pub cron: String,
    pub time_zone: String,
    pub missed_run_policy: MissedRunPolicy,
    pub enabled: bool,
    pub next_run_at: Option<OffsetDateTime>,
    pub last_fire_time: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub created_by: Option<UserId>,
    pub profile_name: String,
    /// The host name of the bound agent (display only).
    pub agent_hostname: String,
    pub agent_status: AgentStatus,
    pub repository_id: RepositoryId,
    /// The recovery point objective in seconds. `None` means the default of the schedule (ADR-0036).
    pub rpo_seconds: Option<i32>,
    /// The finish time of the latest succeeded backup job of the plan.
    pub last_success_at: Option<OffsetDateTime>,
}

/// The keyset cursor of the plan list. It orders by `(created_at, id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanCursor {
    pub created_at: OffsetDateTime,
    pub id: PlanId,
}

#[derive(Debug, Clone, Copy)]
pub struct PlanPage {
    /// Clamped to [`MAX_PAGE_LIMIT`].
    pub limit: i64,
    pub after: Option<PlanCursor>,
}

impl Default for PlanPage {
    fn default() -> Self {
        Self {
            limit: DEFAULT_PAGE_LIMIT,
            after: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlanPageResult {
    pub plans: Vec<Plan>,
    pub has_more: bool,
}

/// The fields of a new row in the `plans` table.
#[derive(Debug, Clone)]
pub struct NewPlan<'a> {
    pub id: PlanId,
    pub name: &'a str,
    pub profile_id: ProfileId,
    pub agent_id: AgentId,
    pub repository_id: RepositoryId,
    pub cron: &'a str,
    pub time_zone: &'a str,
    pub missed_run_policy: MissedRunPolicy,
    pub enabled: bool,
    pub rpo_seconds: Option<i32>,
    /// The first fire time in UTC. `None` when the control plane computes it later.
    pub next_run_at: Option<OffsetDateTime>,
    /// Absent for an API token actor.
    pub created_by: Option<UserId>,
}

/// Mutable fields of a stored row.
#[derive(Debug, Clone)]
pub struct PlanUpdate<'a> {
    pub name: &'a str,
    pub profile_id: ProfileId,
    pub agent_id: AgentId,
    pub repository_id: RepositoryId,
    pub cron: &'a str,
    pub time_zone: &'a str,
    pub missed_run_policy: MissedRunPolicy,
    pub enabled: bool,
    pub rpo_seconds: Option<i32>,
}

/// One due plan that the scheduler claimed, with the agent status (FR-SC-02, FR-SC-07).
#[derive(Debug, Clone)]
pub struct DuePlan {
    pub id: PlanId,
    pub agent_id: AgentId,
    pub cron: String,
    pub time_zone: String,
    pub missed_run_policy: MissedRunPolicy,
    /// The next fire time in UTC. Not null for a claimed row.
    pub next_run_at: OffsetDateTime,
    pub agent_status: AgentStatus,
    /// The repository of the plan. A scheduled job names it (FR-SC-04).
    pub repository_id: RepositoryId,
}

pub async fn create(tx: &mut TenantTx<'_>, new: &NewPlan<'_>) -> Result<PlanId, DbError> {
    sqlx::query(
        "INSERT INTO plans \
         (id, tenant_id, name, profile_id, agent_id, repository_id, cron, time_zone, \
          missed_run_policy, enabled, next_run_at, created_by, rpo_seconds) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(new.id)
    .bind(tx.tenant())
    .bind(new.name)
    .bind(new.profile_id)
    .bind(new.agent_id)
    .bind(new.repository_id)
    .bind(new.cron)
    .bind(new.time_zone)
    .bind(new.missed_run_policy.as_str())
    .bind(new.enabled)
    .bind(new.next_run_at)
    .bind(new.created_by)
    .bind(new.rpo_seconds)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(new.id)
}

/// Lists the plans of the tenant of the transaction, newest first.
pub async fn list(tx: &mut TenantTx<'_>, page: &PlanPage) -> Result<PlanPageResult, DbError> {
    let limit = page.limit.clamp(1, MAX_PAGE_LIMIT);
    let (cursor_time, cursor_id) = match page.after {
        Some(cursor) => (Some(cursor.created_at), Some(cursor.id.as_uuid())),
        None => (None, None),
    };
    let rows = sqlx::query_as::<_, PlanRow>(concat!(
        plan_select!(),
        " WHERE ($1::timestamptz IS NULL OR (p.created_at, p.id) < ($1::timestamptz, $2::uuid)) \
         ORDER BY p.created_at DESC, p.id DESC \
         LIMIT $3",
    ))
    .bind(cursor_time)
    .bind(cursor_id)
    .bind(limit + 1)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let mut plans: Vec<Plan> = rows.into_iter().map(Plan::from).collect();
    let has_more = plans.len() as i64 > limit;
    plans.truncate(limit as usize);
    Ok(PlanPageResult { plans, has_more })
}

/// Reads one plan of the tenant of the transaction. A plan of another tenant is absent.
pub async fn get(tx: &mut TenantTx<'_>, id: PlanId) -> Result<Option<Plan>, DbError> {
    let row = sqlx::query_as::<_, PlanRow>(concat!(plan_select!(), " WHERE p.id = $1",))
        .bind(id)
        .fetch_optional(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(row.map(Plan::from))
}

/// Lists the plans of the tenant that bind the profile.
///
/// A profile that a plan uses cannot be removed until the plan is removed or repointed. The check
/// runs before the delete, so the foreign key never blocks it with an internal error (FR-SC-11).
pub async fn referrers_of_profile(
    tx: &mut TenantTx<'_>,
    profile_id: ProfileId,
) -> Result<Referrers, DbError> {
    crate::referrers::fetch(
        tx,
        "SELECT id, name, count(*) OVER () FROM plans WHERE profile_id = $1 \
         ORDER BY lower(name), id LIMIT $2",
        profile_id.as_uuid(),
    )
    .await
}

/// Lists the plans of the tenant that back up into the repository.
///
/// A repository that a plan uses cannot be removed until the plan is removed or repointed.
pub async fn referrers_of_repository(
    tx: &mut TenantTx<'_>,
    repository_id: RepositoryId,
) -> Result<Referrers, DbError> {
    crate::referrers::fetch(
        tx,
        "SELECT id, name, count(*) OVER () FROM plans WHERE repository_id = $1 \
         ORDER BY lower(name), id LIMIT $2",
        repository_id.as_uuid(),
    )
    .await
}

/// The distinct profiles of the plans that back up into the repository, oldest profile first.
///
/// The maintenance job builds one `forget` step for each of them. A tie on `created_at` goes to the
/// older identifier, so two control planes agree on the order (ADR-0034).
pub async fn profiles_of_repository(
    tx: &mut TenantTx<'_>,
    repository_id: RepositoryId,
) -> Result<Vec<ProfileId>, DbError> {
    let rows = sqlx::query_as::<_, (uuid::Uuid,)>(
        "SELECT pr.id FROM profiles pr \
         WHERE EXISTS (SELECT 1 FROM plans p \
                       WHERE p.tenant_id = pr.tenant_id AND p.profile_id = pr.id \
                         AND p.repository_id = $1) \
         ORDER BY pr.created_at, pr.id",
    )
    .bind(repository_id)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(|(id,)| ProfileId::from_uuid(id))
        .collect())
}

/// Lists the plans of the profile that back up into the repository, oldest first.
///
/// The maintenance job reads the agents of these plans only, so a plan of the same profile in
/// another repository never supplies the facts for a `forget` step of this repository (ADR-0034).
pub async fn list_by_profile_and_repository(
    tx: &mut TenantTx<'_>,
    profile_id: ProfileId,
    repository_id: RepositoryId,
) -> Result<Vec<Plan>, DbError> {
    let rows = sqlx::query_as::<_, PlanRow>(concat!(
        plan_select!(),
        " WHERE p.profile_id = $1 AND p.repository_id = $2 \
         ORDER BY p.created_at, p.id",
    ))
    .bind(profile_id)
    .bind(repository_id)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(Plan::from).collect())
}

/// Lists the plans of the tenant that bind the profile, oldest first (FR-SC-11).
///
/// An update of a profile revalidates each plan against the facts of its agent, so the query names
/// the agent of the plan.
pub async fn list_by_profile(
    tx: &mut TenantTx<'_>,
    profile_id: ProfileId,
) -> Result<Vec<Plan>, DbError> {
    let rows = sqlx::query_as::<_, PlanRow>(concat!(
        plan_select!(),
        " WHERE p.profile_id = $1 \
         ORDER BY p.created_at, p.id",
    ))
    .bind(profile_id)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(Plan::from).collect())
}

/// True when a plan of the repository runs on another agent than `host`.
///
/// A local repository lives on the disk of one host, so each of its plans must run there.
pub async fn has_plan_off_host(
    tx: &mut TenantTx<'_>,
    repository_id: RepositoryId,
    host: AgentId,
) -> Result<bool, DbError> {
    let (found,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM plans p \
         WHERE p.repository_id = $1 AND p.agent_id <> $2)",
    )
    .bind(repository_id)
    .bind(host)
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(found)
}

/// A local repository that breaks the host rule: it has no host, or plans run elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalHostProblem {
    pub repository_id: RepositoryId,
    pub host: Option<AgentId>,
    pub plans_off_host: Vec<PlanId>,
}

/// Lists the local repositories of the tenant without a host or with plans on another agent.
pub async fn local_host_problems(tx: &mut TenantTx<'_>) -> Result<Vec<LocalHostProblem>, DbError> {
    let rows: Vec<(uuid::Uuid, Option<uuid::Uuid>, Vec<uuid::Uuid>)> = sqlx::query_as(
        "SELECT r.id, r.maintenance_agent_id, \
                COALESCE(array_agg(p.id ORDER BY p.created_at, p.id) \
                    FILTER (WHERE p.id IS NOT NULL \
                        AND p.agent_id IS DISTINCT FROM r.maintenance_agent_id), '{}'::uuid[]) \
         FROM repositories r \
         LEFT JOIN plans p ON p.tenant_id = r.tenant_id AND p.repository_id = r.id \
         WHERE r.backend = 'local' \
         GROUP BY r.id, r.maintenance_agent_id \
         HAVING r.maintenance_agent_id IS NULL \
             OR bool_or(p.id IS NOT NULL AND p.agent_id IS DISTINCT FROM r.maintenance_agent_id) \
         ORDER BY r.id",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(|(repository, host, plans)| LocalHostProblem {
            repository_id: RepositoryId::from_uuid(repository),
            host: host.map(AgentId::from_uuid),
            plans_off_host: plans.into_iter().map(PlanId::from_uuid).collect(),
        })
        .collect())
}

/// Reads one plan and locks its row for the transaction.
///
/// An update or a delete uses this function, so a concurrent change cannot remove the row between
/// the read and the write.
pub async fn get_for_update(tx: &mut TenantTx<'_>, id: PlanId) -> Result<Option<Plan>, DbError> {
    let row =
        sqlx::query_as::<_, PlanRow>(concat!(plan_select!(), " WHERE p.id = $1 FOR UPDATE OF p",))
            .bind(id)
            .fetch_optional(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(row.map(Plan::from))
}

/// Writes the mutable fields of a plan and sets `updated_at`. A missing plan returns `false`.
pub async fn update(
    tx: &mut TenantTx<'_>,
    id: PlanId,
    patch: &PlanUpdate<'_>,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE plans SET name = $2, profile_id = $3, agent_id = $4, repository_id = $5, \
                cron = $6, time_zone = $7, missed_run_policy = $8, enabled = $9, \
                rpo_seconds = $10, updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(patch.name)
    .bind(patch.profile_id)
    .bind(patch.agent_id)
    .bind(patch.repository_id)
    .bind(patch.cron)
    .bind(patch.time_zone)
    .bind(patch.missed_run_policy.as_str())
    .bind(patch.enabled)
    .bind(patch.rpo_seconds)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Sets the next fire time of a plan. `None` clears it, for a disabled plan.
pub async fn set_next_run_at(
    tx: &mut TenantTx<'_>,
    id: PlanId,
    next_run_at: Option<OffsetDateTime>,
) -> Result<bool, DbError> {
    let result = sqlx::query("UPDATE plans SET next_run_at = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(next_run_at)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Removes a plan of the tenant. `true` when it removed the row.
pub async fn delete(tx: &mut TenantTx<'_>, id: PlanId) -> Result<bool, DbError> {
    let result = sqlx::query("DELETE FROM plans WHERE id = $1")
        .bind(id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Claims the due plans of the tenant of the transaction (FR-SC-02).
///
/// The select locks each row with `SKIP LOCKED`, so a second control plane instance skips a plan
/// that this transaction holds and exactly one instance creates the jobs of one fire time
/// (AC-SC-02, ADR-0010). The caller advances `next_run_at` in the same transaction.
pub async fn claim_due(
    tx: &mut TenantTx<'_>,
    now: OffsetDateTime,
    limit: i64,
) -> Result<Vec<DuePlan>, DbError> {
    let rows = sqlx::query_as::<_, DuePlanRow>(
        "SELECT p.id, p.agent_id, p.repository_id, p.cron, p.time_zone, p.missed_run_policy, \
                p.next_run_at, a.status \
         FROM plans p \
         JOIN agents a ON a.tenant_id = p.tenant_id AND a.id = p.agent_id \
         WHERE p.enabled AND p.next_run_at IS NOT NULL AND p.next_run_at <= $1 \
         ORDER BY p.next_run_at \
         LIMIT $2 \
         FOR UPDATE OF p SKIP LOCKED",
    )
    .bind(now)
    .bind(limit)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(DuePlan {
                id: row.id,
                agent_id: row.agent_id,
                cron: row.cron,
                time_zone: row.time_zone,
                missed_run_policy: row.missed_run_policy,
                next_run_at: row.next_run_at?,
                agent_status: row.status,
                repository_id: row.repository_id,
            })
        })
        .collect())
}

/// Advances the fire time of a plan after the scheduler processed it (FR-SC-02).
///
/// `last_fire_time` is the last fire time that the control plane wrote a job for, or the last
/// missed fire time when the policy created no job.
pub async fn advance(
    tx: &mut TenantTx<'_>,
    id: PlanId,
    next_run_at: OffsetDateTime,
    last_fire_time: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE plans SET next_run_at = $2, last_fire_time = $3, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(next_run_at)
    .bind(last_fire_time)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// The projected columns of the plan and of its bound profile and agent.
#[derive(sqlx::FromRow)]
struct PlanRow {
    id: PlanId,
    name: String,
    profile_id: ProfileId,
    agent_id: AgentId,
    cron: String,
    time_zone: String,
    missed_run_policy: MissedRunPolicy,
    enabled: bool,
    next_run_at: Option<OffsetDateTime>,
    last_fire_time: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    created_by: Option<UserId>,
    profile_name: String,
    repository_id: RepositoryId,
    agent_hostname: String,
    agent_status: AgentStatus,
    rpo_seconds: Option<i32>,
    last_success_at: Option<OffsetDateTime>,
}

impl From<PlanRow> for Plan {
    fn from(row: PlanRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            profile_id: row.profile_id,
            agent_id: row.agent_id,
            cron: row.cron,
            time_zone: row.time_zone,
            missed_run_policy: row.missed_run_policy,
            enabled: row.enabled,
            next_run_at: row.next_run_at,
            last_fire_time: row.last_fire_time,
            created_at: row.created_at,
            updated_at: row.updated_at,
            created_by: row.created_by,
            profile_name: row.profile_name,
            agent_hostname: row.agent_hostname,
            agent_status: row.agent_status,
            repository_id: row.repository_id,
            rpo_seconds: row.rpo_seconds,
            last_success_at: row.last_success_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct DuePlanRow {
    id: PlanId,
    agent_id: AgentId,
    cron: String,
    time_zone: String,
    missed_run_policy: MissedRunPolicy,
    next_run_at: Option<OffsetDateTime>,
    status: AgentStatus,
    repository_id: RepositoryId,
}
