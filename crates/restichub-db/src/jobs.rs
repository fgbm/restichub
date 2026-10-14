//! The job rows of a tenant (FR-PR-01, FR-PR-03, FR-PR-05, FR-RP-02, FR-RP-08).
//!
//! One row holds the lifecycle of a job: its kind, the agent that runs it, the repository it
//! touches, the status and the typed result. The row never holds a secret: the repository password
//! and the backend credentials go with the signed job only (FR-PR-05, ADR-0003).
//!
//! The MVP sends an on-demand job at once, so the row moves `queued` to `sent` to `running` to a
//! terminal status. The scheduler of epic #7 adds the queue and the retries.
//!
//! The general tenant policy of the `jobs` table shows the rows of the tenant of the transaction
//! only (ADR-0023, section Tenant isolation).

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::enums::{JobKind, JobResultCode, JobStatus};
use crate::ids::{AgentId, JobId, PlanId, RepositoryId, UserId};
use crate::tx::TenantTx;

/// One stored row with its lifecycle, kind, agent and result.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: JobId,
    pub kind: JobKind,
    pub agent_id: AgentId,
    pub repository_id: Option<RepositoryId>,
    /// The plan of a scheduled job (FR-SC-01). Null for an on-demand job.
    pub plan_id: Option<PlanId>,
    /// Set for a job that a plan created, in UTC. Absent for an on-demand job.
    pub fire_time: Option<OffsetDateTime>,
    pub status: JobStatus,
    pub result_code: Option<JobResultCode>,
    /// A machine-readable error code from the agent. Never a secret and never prose (NFR-DOC-04).
    pub result_message: Option<String>,
    /// A hash of the repository location at create time. The control plane applies the
    /// `initialized` flag only while it still matches.
    pub location_fingerprint: Option<String>,
    pub expires_at: Option<OffsetDateTime>,
    pub sent_at: Option<OffsetDateTime>,
    pub started_at: Option<OffsetDateTime>,
    pub finished_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub created_by: Option<UserId>,
}

/// The values that the control plane writes for a new row in the `jobs` table.
#[derive(Debug, Clone)]
pub struct NewJob {
    pub id: JobId,
    pub kind: JobKind,
    pub agent_id: AgentId,
    pub repository_id: Option<RepositoryId>,
    /// The plan of a scheduled job. `None` for an on-demand job.
    pub plan_id: Option<PlanId>,
    /// Set for a job that a plan created, in UTC. `None` for an on-demand job.
    pub fire_time: Option<OffsetDateTime>,
    pub expires_at: Option<OffsetDateTime>,
    /// A hash of the repository location at create time. It holds no secret.
    pub location_fingerprint: Option<String>,
    /// Absent for an API token actor.
    pub created_by: Option<UserId>,
}

/// The terminal result that the control plane stores for a finished job.
#[derive(Debug, Clone)]
pub struct JobOutcome {
    pub status: JobStatus,
    pub result_code: JobResultCode,
    /// A machine-readable error code from the agent. Never a secret.
    pub result_message: Option<String>,
}

pub async fn create(tx: &mut TenantTx<'_>, new: &NewJob) -> Result<JobId, DbError> {
    sqlx::query(
        "INSERT INTO jobs \
         (id, tenant_id, kind, agent_id, repository_id, plan_id, fire_time, status, expires_at, \
          location_fingerprint, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'queued', $8, $9, $10)",
    )
    .bind(new.id)
    .bind(tx.tenant())
    .bind(new.kind.as_str())
    .bind(new.agent_id)
    .bind(new.repository_id)
    .bind(new.plan_id)
    .bind(new.fire_time)
    .bind(new.expires_at)
    .bind(new.location_fingerprint.as_deref())
    .bind(new.created_by)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(new.id)
}

/// Creates one scheduled job for a plan and a fire time (FR-SC-02).
///
/// The unique key `(plan_id, fire_time)` makes the create idempotent: a second control plane
/// instance, or a second run over the same fire time, inserts no row and gets `None` (AC-SC-02,
/// ADR-0010).
pub async fn create_scheduled(
    tx: &mut TenantTx<'_>,
    new: &NewJob,
) -> Result<Option<JobId>, DbError> {
    let row = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO jobs \
         (id, tenant_id, kind, agent_id, repository_id, plan_id, fire_time, status, expires_at, \
          location_fingerprint, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'queued', $8, $9, $10) \
         ON CONFLICT (plan_id, fire_time) DO NOTHING \
         RETURNING id",
    )
    .bind(new.id)
    .bind(tx.tenant())
    .bind(new.kind.as_str())
    .bind(new.agent_id)
    .bind(new.repository_id)
    .bind(new.plan_id)
    .bind(new.fire_time)
    .bind(new.expires_at)
    .bind(new.location_fingerprint.as_deref())
    .bind(new.created_by)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(JobId::from_uuid))
}

/// Reads one job of the tenant of the transaction. A job of another tenant is absent.
pub async fn get(tx: &mut TenantTx<'_>, id: JobId) -> Result<Option<Job>, DbError> {
    let row = sqlx::query_as::<_, JobRow>(
        "SELECT id, kind, agent_id, repository_id, plan_id, fire_time, status, result_code, \
                result_message, location_fingerprint, expires_at, sent_at, started_at, finished_at, \
                created_at, updated_at, created_by \
         FROM jobs WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(Job::from))
}

/// Reads one job and locks its row for the transaction.
///
/// A result that arrives for a job of another tenant is absent, so the caller cannot change it
/// (FR-PR-08).
pub async fn get_for_update(tx: &mut TenantTx<'_>, id: JobId) -> Result<Option<Job>, DbError> {
    let row = sqlx::query_as::<_, JobRow>(
        "SELECT id, kind, agent_id, repository_id, plan_id, fire_time, status, result_code, \
                result_message, location_fingerprint, expires_at, sent_at, started_at, finished_at, \
                created_at, updated_at, created_by \
         FROM jobs WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(Job::from))
}

/// Moves a queued job to `sent` and sets `sent_at`. `false` when the row is not queued.
pub async fn mark_sent(
    tx: &mut TenantTx<'_>,
    id: JobId,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET status = 'sent', sent_at = $2, updated_at = now() \
         WHERE id = $1 AND status = 'queued'",
    )
    .bind(id)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Moves a sent job to `running` and sets `started_at`. `false` when the row is not sent.
pub async fn mark_running(
    tx: &mut TenantTx<'_>,
    id: JobId,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET status = 'running', started_at = $2, updated_at = now() \
         WHERE id = $1 AND status = 'sent'",
    )
    .bind(id)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Writes the terminal status and the typed result of a job and sets `finished_at`.
///
/// The update runs only while the job is not terminal, so a second result for the same job changes
/// nothing. `false` when the row is already terminal or absent.
pub async fn finish(
    tx: &mut TenantTx<'_>,
    id: JobId,
    outcome: &JobOutcome,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET status = $2, result_code = $3, result_message = $4, finished_at = $5, \
                updated_at = now() \
         WHERE id = $1 AND status NOT IN ('succeeded', 'failed', 'expired')",
    )
    .bind(id)
    .bind(outcome.status.as_str())
    .bind(outcome.result_code.as_str())
    .bind(outcome.result_message.as_deref())
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Marks a queued job `failed` because the control plane refused to send it (offline agent).
///
/// The row stays as a record of the refusal. The caller returns an error to the client and writes
/// no secret.
pub async fn mark_refused(
    tx: &mut TenantTx<'_>,
    id: JobId,
    message: &str,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET status = 'failed', result_code = 'failed', result_message = $2, \
                finished_at = $3, updated_at = now() \
         WHERE id = $1 AND status = 'queued'",
    )
    .bind(id)
    .bind(message)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Lists the scheduled jobs of one plan, oldest fire time first (FR-SC-02).
pub async fn list_for_plan(tx: &mut TenantTx<'_>, plan_id: PlanId) -> Result<Vec<Job>, DbError> {
    let rows = sqlx::query_as::<_, JobRow>(
        "SELECT id, kind, agent_id, repository_id, plan_id, fire_time, status, result_code, \
                result_message, location_fingerprint, expires_at, sent_at, started_at, finished_at, \
                created_at, updated_at, created_by \
         FROM jobs WHERE plan_id = $1 ORDER BY fire_time",
    )
    .bind(plan_id)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(Job::from).collect())
}

/// True when the repository holds a job that is queued, sent or running.
pub async fn has_active_for_repository(
    tx: &mut TenantTx<'_>,
    repository_id: RepositoryId,
) -> Result<bool, DbError> {
    let (active,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM jobs WHERE repository_id = $1 \
         AND status IN ('queued', 'sent', 'running'))",
    )
    .bind(repository_id)
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(active)
}

/// Moves one overdue job to the terminal `expired` status. `false` when it is terminal
/// or not overdue.
pub async fn mark_expired(
    tx: &mut TenantTx<'_>,
    id: JobId,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET status = 'expired', finished_at = $2, updated_at = now() \
         WHERE id = $1 AND status NOT IN ('succeeded', 'failed', 'expired') \
           AND expires_at IS NOT NULL AND expires_at <= $2",
    )
    .bind(id)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Marks every overdue non-terminal job of the tenant `expired`.
///
/// The periodic sweep calls it. The update is idempotent, so two control plane instances both run
/// it without a conflict.
pub async fn expire_overdue(tx: &mut TenantTx<'_>, now: OffsetDateTime) -> Result<u64, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET status = 'expired', finished_at = $1, updated_at = now() \
         WHERE status NOT IN ('succeeded', 'failed', 'expired') \
           AND expires_at IS NOT NULL AND expires_at <= $1",
    )
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}

#[derive(sqlx::FromRow)]
struct JobRow {
    id: Uuid,
    kind: JobKind,
    agent_id: Uuid,
    repository_id: Option<Uuid>,
    plan_id: Option<Uuid>,
    fire_time: Option<OffsetDateTime>,
    status: JobStatus,
    result_code: Option<JobResultCode>,
    result_message: Option<String>,
    location_fingerprint: Option<String>,
    expires_at: Option<OffsetDateTime>,
    sent_at: Option<OffsetDateTime>,
    started_at: Option<OffsetDateTime>,
    finished_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    created_by: Option<Uuid>,
}

impl From<JobRow> for Job {
    fn from(row: JobRow) -> Self {
        Self {
            id: JobId::from_uuid(row.id),
            kind: row.kind,
            agent_id: AgentId::from_uuid(row.agent_id),
            repository_id: row.repository_id.map(RepositoryId::from_uuid),
            plan_id: row.plan_id.map(PlanId::from_uuid),
            fire_time: row.fire_time,
            status: row.status,
            result_code: row.result_code,
            result_message: row.result_message,
            location_fingerprint: row.location_fingerprint,
            expires_at: row.expires_at,
            sent_at: row.sent_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
            created_by: row.created_by.map(UserId::from_uuid),
        }
    }
}
