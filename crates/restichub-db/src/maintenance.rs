//! The maintenance schedules of a repository (FR-RP-08, FR-SC-04, ADR-0034).
//!
//! A repository has one schedule row for each [`MaintenanceTask`]. The scheduler claims the due rows
//! with `FOR UPDATE SKIP LOCKED` and creates one job for each fire time. The unique key
//! `(maintenance_schedule_id, fire_time, attempt)` on the `jobs` table makes the create idempotent.
//!
//! The general tenant policy of the `maintenance_schedules` table shows the rows of the tenant of
//! the transaction only (ADR-0023, section Tenant isolation).

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::enums::{AgentStatus, MaintenanceTask};
use crate::ids::{AgentId, MaintenanceScheduleId, RepositoryId};
use crate::tx::TenantTx;

/// One schedule of a repository.
#[derive(Debug, Clone)]
pub struct MaintenanceSchedule {
    pub id: MaintenanceScheduleId,
    pub repository_id: RepositoryId,
    pub task: MaintenanceTask,
    pub cron: String,
    pub time_zone: String,
    pub enabled: bool,
    pub next_run_at: Option<OffsetDateTime>,
    pub last_fire_time: Option<OffsetDateTime>,
}

#[derive(Debug, Clone)]
pub struct NewMaintenanceSchedule<'a> {
    pub repository_id: RepositoryId,
    pub task: MaintenanceTask,
    pub cron: &'a str,
    pub time_zone: &'a str,
    pub enabled: bool,
    pub next_run_at: Option<OffsetDateTime>,
}

/// One due schedule that the scheduler claimed, with the state of its repository.
#[derive(Debug, Clone)]
pub struct DueMaintenance {
    pub id: MaintenanceScheduleId,
    pub repository_id: RepositoryId,
    pub task: MaintenanceTask,
    pub cron: String,
    pub time_zone: String,
    /// The next fire time in UTC. Not null for a claimed row.
    pub next_run_at: OffsetDateTime,
    pub maintenance_agent_id: Option<AgentId>,
    /// `None` when the repository names no maintenance agent.
    pub agent_status: Option<AgentStatus>,
    pub initialized: bool,
}

type ScheduleRow = (
    Uuid,
    Uuid,
    MaintenanceTask,
    String,
    String,
    bool,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
);

impl From<ScheduleRow> for MaintenanceSchedule {
    fn from(row: ScheduleRow) -> Self {
        let (id, repository_id, task, cron, time_zone, enabled, next_run_at, last_fire_time) = row;
        Self {
            id: MaintenanceScheduleId::from_uuid(id),
            repository_id: RepositoryId::from_uuid(repository_id),
            task,
            cron,
            time_zone,
            enabled,
            next_run_at,
            last_fire_time,
        }
    }
}

macro_rules! columns {
    () => {
        "id, repository_id, task, cron, time_zone, enabled, next_run_at, last_fire_time"
    };
}

pub async fn create(
    tx: &mut TenantTx<'_>,
    new: &NewMaintenanceSchedule<'_>,
) -> Result<MaintenanceScheduleId, DbError> {
    let id = MaintenanceScheduleId::new();
    sqlx::query(
        "INSERT INTO maintenance_schedules \
         (id, tenant_id, repository_id, task, cron, time_zone, enabled, next_run_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(tx.tenant())
    .bind(new.repository_id)
    .bind(new.task.as_str())
    .bind(new.cron)
    .bind(new.time_zone)
    .bind(new.enabled)
    .bind(new.next_run_at)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(id)
}

/// Lists the schedules of one repository.
pub async fn list_for_repository(
    tx: &mut TenantTx<'_>,
    repository_id: RepositoryId,
) -> Result<Vec<MaintenanceSchedule>, DbError> {
    let rows = sqlx::query_as::<_, ScheduleRow>(concat!(
        "SELECT ",
        columns!(),
        " FROM maintenance_schedules WHERE repository_id = $1 ORDER BY task"
    ))
    .bind(repository_id)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(MaintenanceSchedule::from).collect())
}

/// Reads one schedule of a repository and locks the row for the transaction.
pub async fn get_for_update(
    tx: &mut TenantTx<'_>,
    repository_id: RepositoryId,
    task: MaintenanceTask,
) -> Result<Option<MaintenanceSchedule>, DbError> {
    let row = sqlx::query_as::<_, ScheduleRow>(concat!(
        "SELECT ",
        columns!(),
        " FROM maintenance_schedules WHERE repository_id = $1 AND task = $2 FOR UPDATE"
    ))
    .bind(repository_id)
    .bind(task.as_str())
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(MaintenanceSchedule::from))
}

/// Writes the schedule of a row and its next fire time. A new schedule starts from `next_run_at`.
pub async fn update(
    tx: &mut TenantTx<'_>,
    id: MaintenanceScheduleId,
    cron: &str,
    time_zone: &str,
    enabled: bool,
    next_run_at: Option<OffsetDateTime>,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE maintenance_schedules SET cron = $2, time_zone = $3, enabled = $4, \
                next_run_at = $5, schedule_error = NULL, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(cron)
    .bind(time_zone)
    .bind(enabled)
    .bind(next_run_at)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// The repositories of the tenant that miss a schedule row of a task, with the task.
///
/// The scheduler creates the default rows for them. The repositories that exist before the
/// migration 0037 have none.
pub async fn missing_defaults(
    tx: &mut TenantTx<'_>,
    limit: i64,
) -> Result<Vec<(RepositoryId, MaintenanceTask)>, DbError> {
    let rows: Vec<(Uuid, MaintenanceTask)> = sqlx::query_as(
        "SELECT r.id, t.task FROM repositories r \
         CROSS JOIN (VALUES ('retention'), ('check')) AS t (task) \
         WHERE NOT EXISTS (SELECT 1 FROM maintenance_schedules s \
                           WHERE s.repository_id = r.id AND s.task = t.task) \
         ORDER BY r.created_at, r.id, t.task \
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(|(id, task)| (RepositoryId::from_uuid(id), task))
        .collect())
}

/// Claims the enabled schedules whose first fire time is not set, so the caller computes it.
pub async fn claim_unarmed(
    tx: &mut TenantTx<'_>,
    limit: i64,
) -> Result<Vec<MaintenanceSchedule>, DbError> {
    let rows = sqlx::query_as::<_, ScheduleRow>(concat!(
        "SELECT ",
        columns!(),
        " FROM maintenance_schedules WHERE enabled AND next_run_at IS NULL \
          ORDER BY created_at, id LIMIT $1 FOR UPDATE SKIP LOCKED"
    ))
    .bind(limit)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(MaintenanceSchedule::from).collect())
}

/// Claims the enabled schedules that are due at `now`, oldest first.
///
/// A row that another scheduler instance holds is skipped, so two instances never process the same
/// row at the same time (ADR-0010).
pub async fn claim_due(
    tx: &mut TenantTx<'_>,
    now: OffsetDateTime,
    limit: i64,
) -> Result<Vec<DueMaintenance>, DbError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: Uuid,
        repository_id: Uuid,
        task: MaintenanceTask,
        cron: String,
        time_zone: String,
        next_run_at: Option<OffsetDateTime>,
        maintenance_agent_id: Option<Uuid>,
        status: Option<AgentStatus>,
        initialized: bool,
    }
    let rows = sqlx::query_as::<_, Row>(
        "SELECT s.id, s.repository_id, s.task, s.cron, s.time_zone, s.next_run_at, \
                r.maintenance_agent_id, a.status, r.initialized \
         FROM maintenance_schedules s \
         JOIN repositories r ON r.tenant_id = s.tenant_id AND r.id = s.repository_id \
         LEFT JOIN agents a ON a.tenant_id = r.tenant_id AND a.id = r.maintenance_agent_id \
         WHERE s.enabled AND s.next_run_at IS NOT NULL AND s.next_run_at <= $1 \
         ORDER BY s.next_run_at \
         LIMIT $2 \
         FOR UPDATE OF s SKIP LOCKED",
    )
    .bind(now)
    .bind(limit)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(DueMaintenance {
                id: MaintenanceScheduleId::from_uuid(row.id),
                repository_id: RepositoryId::from_uuid(row.repository_id),
                task: row.task,
                cron: row.cron,
                time_zone: row.time_zone,
                next_run_at: row.next_run_at?,
                maintenance_agent_id: row.maintenance_agent_id.map(AgentId::from_uuid),
                agent_status: row.status,
                initialized: row.initialized,
            })
        })
        .collect())
}

/// Moves a schedule to its next fire time after the scheduler processed it.
pub async fn advance(
    tx: &mut TenantTx<'_>,
    id: MaintenanceScheduleId,
    next_run_at: OffsetDateTime,
    last_fire_time: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE maintenance_schedules SET next_run_at = $2, last_fire_time = $3, \
                schedule_error = NULL, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(next_run_at)
    .bind(last_fire_time)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Moves a schedule whose pass failed to its next fire time and records the cause (FR-SC-02).
pub async fn defer_failed(
    tx: &mut TenantTx<'_>,
    id: MaintenanceScheduleId,
    next_run_at: OffsetDateTime,
    cause: &str,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE maintenance_schedules SET next_run_at = $2, schedule_error = $3, \
                updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(next_run_at)
    .bind(cause)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Disables a schedule that the control plane cannot compute and records the cause.
pub async fn disable_failed(
    tx: &mut TenantTx<'_>,
    id: MaintenanceScheduleId,
    cause: &str,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE maintenance_schedules SET enabled = false, next_run_at = NULL, \
                schedule_error = $2, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(cause)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Sets the first fire time of an unarmed schedule.
pub async fn arm(
    tx: &mut TenantTx<'_>,
    id: MaintenanceScheduleId,
    next_run_at: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE maintenance_schedules SET next_run_at = $2, schedule_error = NULL, \
                updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(next_run_at)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// True when a plan of the repository uses a profile that sets a retention and an active agent
/// runs the plan.
///
/// A repository without such a plan has nothing to forget, so the scheduler and the manual run
/// create no retention job for it.
pub async fn has_retention_in_use(
    tx: &mut TenantTx<'_>,
    repository_id: RepositoryId,
) -> Result<bool, DbError> {
    let (found,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM plans pl \
                        JOIN profiles p ON p.tenant_id = pl.tenant_id AND p.id = pl.profile_id \
                        JOIN agents a ON a.tenant_id = pl.tenant_id AND a.id = pl.agent_id \
                        WHERE pl.repository_id = $1 AND p.retention IS NOT NULL \
                          AND a.status = 'active')",
    )
    .bind(repository_id)
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(found)
}
