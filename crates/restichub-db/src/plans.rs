//! The plan rows of a tenant (FR-SC-01, FR-SC-07, FR-SC-11, ADR-0010).
//!
//! One row binds a profile to one agent with a cron schedule and a time zone (FR-SC-01). The
//! scheduler reads the due rows and creates one job for each fire time; the unique key
//! `(plan_id, fire_time)` on the `jobs` table makes the create idempotent (FR-SC-02).
//!
//! The row holds no secret and no compiled command: a scheduled job names the plan and the fire
//! time only, and the control plane recompiles the stored profile when it dispatches.
//!
//! The general tenant policy of the `plans` table shows the rows of the tenant of the transaction
//! only (ADR-0023, section Tenant isolation).

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::enums::{AgentStatus, MissedRunPolicy};
use crate::ids::{AgentId, PlanId, ProfileId, RepositoryId, UserId};
use crate::tx::TenantTx;

pub const DEFAULT_PAGE_LIMIT: i64 = 50;

pub const MAX_PAGE_LIMIT: i64 = 200;

/// A schedule that binds a profile to one agent, with a cron expression and a time zone
/// (FR-SC-01, ADR-0010).
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
    /// The repository of the bound profile (display only).
    pub repository_id: RepositoryId,
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
    pub cron: &'a str,
    pub time_zone: &'a str,
    pub missed_run_policy: MissedRunPolicy,
    pub enabled: bool,
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
    pub cron: &'a str,
    pub time_zone: &'a str,
    pub missed_run_policy: MissedRunPolicy,
    pub enabled: bool,
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
    /// The repository of the bound profile. A scheduled job names it (FR-SC-04).
    pub repository_id: RepositoryId,
}

pub async fn create(tx: &mut TenantTx<'_>, new: &NewPlan<'_>) -> Result<PlanId, DbError> {
    sqlx::query(
        "INSERT INTO plans \
         (id, tenant_id, name, profile_id, agent_id, cron, time_zone, missed_run_policy, enabled, \
          next_run_at, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(new.id)
    .bind(tx.tenant())
    .bind(new.name)
    .bind(new.profile_id)
    .bind(new.agent_id)
    .bind(new.cron)
    .bind(new.time_zone)
    .bind(new.missed_run_policy.as_str())
    .bind(new.enabled)
    .bind(new.next_run_at)
    .bind(new.created_by)
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
    let rows = sqlx::query_as::<_, PlanRow>(
        "SELECT p.id, p.name, p.profile_id, p.agent_id, p.cron, p.time_zone, \
                p.missed_run_policy, p.enabled, p.next_run_at, p.last_fire_time, p.created_at, \
                p.updated_at, p.created_by, pr.name, pr.repository_id, a.hostname \
         FROM plans p \
         JOIN profiles pr ON pr.tenant_id = p.tenant_id AND pr.id = p.profile_id \
         JOIN agents a ON a.tenant_id = p.tenant_id AND a.id = p.agent_id \
         WHERE ($1::timestamptz IS NULL OR (p.created_at, p.id) < ($1::timestamptz, $2::uuid)) \
         ORDER BY p.created_at DESC, p.id DESC \
         LIMIT $3",
    )
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
    let row = sqlx::query_as::<_, PlanRow>(
        "SELECT p.id, p.name, p.profile_id, p.agent_id, p.cron, p.time_zone, \
                p.missed_run_policy, p.enabled, p.next_run_at, p.last_fire_time, p.created_at, \
                p.updated_at, p.created_by, pr.name, pr.repository_id, a.hostname \
         FROM plans p \
         JOIN profiles pr ON pr.tenant_id = p.tenant_id AND pr.id = p.profile_id \
         JOIN agents a ON a.tenant_id = p.tenant_id AND a.id = p.agent_id \
         WHERE p.id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(Plan::from))
}

/// Reads one plan and locks its row for the transaction.
///
/// An update or a delete uses this function, so a concurrent change cannot remove the row between
/// the read and the write.
pub async fn get_for_update(tx: &mut TenantTx<'_>, id: PlanId) -> Result<Option<Plan>, DbError> {
    let row = sqlx::query_as::<_, PlanRow>(
        "SELECT p.id, p.name, p.profile_id, p.agent_id, p.cron, p.time_zone, \
                p.missed_run_policy, p.enabled, p.next_run_at, p.last_fire_time, p.created_at, \
                p.updated_at, p.created_by, pr.name, pr.repository_id, a.hostname \
         FROM plans p \
         JOIN profiles pr ON pr.tenant_id = p.tenant_id AND pr.id = p.profile_id \
         JOIN agents a ON a.tenant_id = p.tenant_id AND a.id = p.agent_id \
         WHERE p.id = $1 FOR UPDATE OF p",
    )
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
        "UPDATE plans SET name = $2, profile_id = $3, agent_id = $4, cron = $5, time_zone = $6, \
                missed_run_policy = $7, enabled = $8, updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(patch.name)
    .bind(patch.profile_id)
    .bind(patch.agent_id)
    .bind(patch.cron)
    .bind(patch.time_zone)
    .bind(patch.missed_run_policy.as_str())
    .bind(patch.enabled)
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
        "SELECT p.id, p.agent_id, p.cron, p.time_zone, p.missed_run_policy, p.next_run_at, \
                a.status, pr.repository_id \
         FROM plans p \
         JOIN agents a ON a.tenant_id = p.tenant_id AND a.id = p.agent_id \
         JOIN profiles pr ON pr.tenant_id = p.tenant_id AND pr.id = p.profile_id \
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
type PlanRow = (
    Uuid,
    String,
    Uuid,
    Uuid,
    String,
    String,
    MissedRunPolicy,
    bool,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
    OffsetDateTime,
    OffsetDateTime,
    Option<Uuid>,
    String,
    Uuid,
    String,
);

impl From<PlanRow> for Plan {
    fn from(row: PlanRow) -> Self {
        let (
            id,
            name,
            profile_id,
            agent_id,
            cron,
            time_zone,
            missed_run_policy,
            enabled,
            next_run_at,
            last_fire_time,
            created_at,
            updated_at,
            created_by,
            profile_name,
            repository_id,
            agent_hostname,
        ) = row;
        Self {
            id: PlanId::from_uuid(id),
            name,
            profile_id: ProfileId::from_uuid(profile_id),
            agent_id: AgentId::from_uuid(agent_id),
            cron,
            time_zone,
            missed_run_policy,
            enabled,
            next_run_at,
            last_fire_time,
            created_at,
            updated_at,
            created_by: created_by.map(UserId::from_uuid),
            profile_name,
            agent_hostname,
            repository_id: RepositoryId::from_uuid(repository_id),
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
