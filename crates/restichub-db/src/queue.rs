//! The queue operations on the `jobs` table (FR-SC-03, FR-SC-04, FR-SC-05, FR-SC-06, FR-PR-06,
//! ADR-0010).
//!
//! The dispatcher of the control plane runs these statements inside a tenant transaction. The
//! select locks each candidate with `FOR UPDATE SKIP LOCKED`, so a second control plane instance
//! skips a row that the first one holds. A per-tenant advisory lock serializes the limit and
//! barrier accounting of one tenant between the instances.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::enums::{JobKind, JobStatus};
use crate::ids::{AgentId, JobId, PlanId, RepositoryId, SecretId};
use crate::jobs::{Job, JobRow, job_columns};
use crate::tx::TenantTx;

pub const DEFAULT_PAGE_LIMIT: i64 = 50;

pub const MAX_PAGE_LIMIT: i64 = 200;

/// A job that holds a slot of the limits: it is `sent` or `running` (FR-SC-04, FR-SC-08).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InFlight {
    pub agent_id: AgentId,
    pub repository_id: Option<RepositoryId>,
    pub kind: JobKind,
}

/// A `running` job whose agent has not been seen for a long time (FR-SC-06).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LostJob {
    pub id: JobId,
    /// The latest sign of life of the agent or of the job.
    pub lost_at: OffsetDateTime,
}

/// A repository with a lock that may be stale (FR-SC-06).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StaleRepository {
    pub repository_id: RepositoryId,
    /// The agent of the marked job that left the lock last; the unlock falls back to it.
    pub agent_id: AgentId,
    /// The latest time that a marked job left a possible stale lock.
    pub stale_since: OffsetDateTime,
}

/// A job with the display names of its plan, agent and repository.
#[derive(Debug, Clone)]
pub struct JobDetail {
    pub job: Job,
    pub plan_name: Option<String>,
    pub agent_hostname: Option<String>,
    pub repository_name: Option<String>,
}

/// The filters of the job list. An absent filter matches every job.
#[derive(Debug, Clone, Copy, Default)]
pub struct JobFilter {
    pub plan_id: Option<PlanId>,
    pub repository_id: Option<RepositoryId>,
    pub agent_id: Option<AgentId>,
    pub status: Option<JobStatus>,
}

/// The keyset cursor of the job list. It orders by `(created_at, id)`, newest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobCursor {
    pub created_at: OffsetDateTime,
    pub id: JobId,
}

#[derive(Debug, Clone, Copy)]
pub struct JobPage {
    /// Clamped to [`MAX_PAGE_LIMIT`].
    pub limit: i64,
    pub after: Option<JobCursor>,
}

impl Default for JobPage {
    fn default() -> Self {
        Self {
            limit: DEFAULT_PAGE_LIMIT,
            after: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct JobPageResult {
    pub jobs: Vec<JobDetail>,
    pub has_more: bool,
}

/// Takes the queue lock of the tenant for the rest of the transaction.
///
/// `false` when another transaction holds it. The holder is another dispatcher instance that works
/// on the same tenant, so the caller skips the tenant until the next pass.
pub async fn try_lock_tenant(tx: &mut TenantTx<'_>) -> Result<bool, DbError> {
    let key = format!("restichub.queue:{}", tx.tenant());
    let (locked,): (bool,) =
        sqlx::query_as("SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(key)
            .fetch_one(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(locked)
}

/// Selects the jobs that the dispatcher may send now, oldest first, and locks them.
///
/// Only the kinds that the queue builds qualify: an on-demand `init` or `test_access` job is never
/// a queue job (FR-RP-02).
///
/// A `queued` job is dispatchable at once, or after `run_at` when a lock retry delayed it. A `sent`
/// job is dispatchable again once its lease passed without an acknowledgement (FR-PR-06). A job
/// past its queue lifetime and a job with a cancel request never qualify. Only the jobs of the
/// given online agents qualify, and each agent contributes its `per_agent` oldest rows, so the
/// backlog of one agent never hides the jobs of another. A locked row of another instance is
/// skipped.
pub async fn dispatchable(
    tx: &mut TenantTx<'_>,
    now: OffsetDateTime,
    online: &[AgentId],
    per_agent: i64,
) -> Result<Vec<Job>, DbError> {
    let agents: Vec<Uuid> = online.iter().map(|agent| agent.as_uuid()).collect();
    // The window function cannot share a query level with `FOR UPDATE`, so the ranking selects the
    // identifiers and the outer query locks the rows.
    let rows = sqlx::query_as::<_, JobRow>(concat!(
        "SELECT ",
        job_columns!(),
        " FROM jobs WHERE id IN ( \
              SELECT id FROM ( \
                SELECT id, row_number() OVER (PARTITION BY agent_id ORDER BY created_at, id) AS rank \
                FROM jobs \
                WHERE ((status = 'queued' AND (attempt = 0 OR run_at <= $1)) \
                       OR (status = 'sent' AND lease_until IS NOT NULL AND lease_until <= $1)) \
                  AND NOT cancel_requested \
                  AND (expires_at IS NULL OR expires_at > $1) \
                  AND kind IN ('backup', 'forget', 'prune', 'check', 'unlock', 'unlock_all') \
                  AND agent_id = ANY($2)) ranked \
              WHERE rank <= $3) \
          ORDER BY created_at, id \
          FOR UPDATE SKIP LOCKED"
    ))
    .bind(now)
    .bind(&agents)
    .bind(per_agent)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(Job::from).collect())
}

/// Lists the jobs that hold a slot of the limits.
pub async fn in_flight(tx: &mut TenantTx<'_>) -> Result<Vec<InFlight>, DbError> {
    sqlx::query_as::<_, InFlight>(
        "SELECT agent_id, repository_id, kind FROM jobs WHERE status IN ('sent', 'running')",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// The signed job of a `sent` row, encrypted with the data encryption key of the tenant.
#[derive(Debug, Clone)]
pub struct StoredSignedJob {
    pub key_version: i32,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    /// The secrets that the job carries; each delivery writes their audit records.
    pub secrets: Vec<SecretId>,
}

/// Marks a queued job as sent, stores its signed bytes and starts its lease. `false` when the job
/// is not queued.
pub async fn mark_dispatched(
    tx: &mut TenantTx<'_>,
    id: JobId,
    now: OffsetDateTime,
    lease_until: OffsetDateTime,
    signed: &StoredSignedJob,
) -> Result<bool, DbError> {
    let secrets: Vec<Uuid> = signed.secrets.iter().map(|id| id.as_uuid()).collect();
    let result = sqlx::query(
        "UPDATE jobs SET status = 'sent', sent_at = $2, lease_until = $3, \
                deliveries = deliveries + 1, signed_job = $4, signed_job_nonce = $5, \
                signed_job_key_version = $6, signed_job_secrets = $7, updated_at = now() \
         WHERE id = $1 AND status = 'queued'",
    )
    .bind(id)
    .bind(now)
    .bind(lease_until)
    .bind(&signed.ciphertext)
    .bind(&signed.nonce)
    .bind(signed.key_version)
    .bind(&secrets)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Starts a new lease for a `sent` job that goes out again. `false` when the job is not sent.
pub async fn mark_redelivered(
    tx: &mut TenantTx<'_>,
    id: JobId,
    now: OffsetDateTime,
    lease_until: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET sent_at = $2, lease_until = $3, deliveries = deliveries + 1, \
                updated_at = now() \
         WHERE id = $1 AND status = 'sent'",
    )
    .bind(id)
    .bind(now)
    .bind(lease_until)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// The ciphertext of a `sent` row, or `None` when the row holds none.
pub async fn signed_job(
    tx: &mut TenantTx<'_>,
    id: JobId,
) -> Result<Option<StoredSignedJob>, DbError> {
    let row = sqlx::query_as::<_, (Option<Vec<u8>>, Option<Vec<u8>>, Option<i32>, Vec<Uuid>)>(
        "SELECT signed_job, signed_job_nonce, signed_job_key_version, signed_job_secrets \
         FROM jobs WHERE id = $1 AND status = 'sent'",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(match row {
        Some((Some(ciphertext), Some(nonce), Some(key_version), secrets)) => {
            Some(StoredSignedJob {
                key_version,
                nonce,
                ciphertext,
                secrets: secrets.into_iter().map(SecretId::from_uuid).collect(),
            })
        }
        _ => None,
    })
}

/// Flags a sent or running job for cancel. The dispatcher never sends a flagged job.
pub async fn request_cancel(tx: &mut TenantTx<'_>, id: JobId) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET cancel_requested = true, updated_at = now() \
         WHERE id = $1 AND status IN ('sent', 'running')",
    )
    .bind(id)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Fails the flagged `sent` jobs whose lease passed without an acknowledgement, with the
/// `cancelled` result.
pub async fn cancel_unacknowledged(
    tx: &mut TenantTx<'_>,
    now: OffsetDateTime,
) -> Result<u64, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET status = 'failed', result_code = 'cancelled', finished_at = $1, \
                lease_until = NULL, updated_at = now() \
         WHERE status = 'sent' AND cancel_requested \
           AND (lease_until IS NULL OR lease_until <= $1)",
    )
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}

/// A busy refusal is not a delivery: it frees the delivery count, and the job goes out again at
/// `until`.
pub async fn defer_busy(
    tx: &mut TenantTx<'_>,
    id: JobId,
    until: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET lease_until = $2, deliveries = GREATEST(deliveries - 1, 0), \
                updated_at = now() \
         WHERE id = $1 AND status = 'sent'",
    )
    .bind(id)
    .bind(until)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Records that a step of a sent or running job reported a held repository lock.
pub async fn note_lock_reported(tx: &mut TenantTx<'_>, id: JobId) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET lock_reported = true, updated_at = now() \
         WHERE id = $1 AND status IN ('sent', 'running')",
    )
    .bind(id)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Returns a sent job to the queue because the control plane could not send it.
pub async fn revert_dispatch(tx: &mut TenantTx<'_>, id: JobId) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET status = 'queued', sent_at = NULL, lease_until = NULL, \
                deliveries = GREATEST(deliveries - 1, 0), updated_at = now() \
         WHERE id = $1 AND status = 'sent'",
    )
    .bind(id)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Ends a job that reported a held lock and queues its retry as a new row (FR-SC-05, ADR-0020).
///
/// The agent journal returns the stored result for a known `job_id`, so the retry needs its own
/// identifier. The old row becomes `failed` first, so the active-job index of the repository allows
/// the new row. `None` when the old row is already terminal.
pub async fn retry_as_new_job(
    tx: &mut TenantTx<'_>,
    old: &Job,
    attempt: i32,
    run_at: OffsetDateTime,
    expires_at: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> Result<Option<JobId>, DbError> {
    let ended = sqlx::query(
        "UPDATE jobs SET status = 'failed', result_code = 'failed', \
                result_message = 'lock_retry', finished_at = $2, lease_until = NULL, \
                updated_at = now() \
         WHERE id = $1 AND status IN ('sent', 'running')",
    )
    .bind(old.id)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    if ended.rows_affected() != 1 {
        return Ok(None);
    }
    let id = JobId::new();
    sqlx::query(
        "INSERT INTO jobs \
         (id, tenant_id, kind, agent_id, repository_id, plan_id, fire_time, status, expires_at, \
          location_fingerprint, created_by, attempt, run_at, retry_of) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'queued', $8, $9, $10, $11, $12, $13)",
    )
    .bind(id)
    .bind(tx.tenant())
    .bind(old.kind.as_str())
    .bind(old.agent_id)
    .bind(old.repository_id)
    .bind(old.plan_id)
    .bind(old.fire_time)
    .bind(expires_at)
    .bind(old.location_fingerprint.as_deref())
    .bind(old.created_by)
    .bind(attempt)
    .bind(run_at)
    .bind(old.id)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(Some(id))
}

/// Moves a job to the terminal `dead_letter` status after the last retry (FR-SC-05).
pub async fn dead_letter(
    tx: &mut TenantTx<'_>,
    id: JobId,
    message: &str,
    attempt: i32,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET status = 'dead_letter', result_code = 'failed', result_message = $2, \
                attempt = $3, finished_at = $4, lease_until = NULL, updated_at = now() \
         WHERE id = $1 AND status NOT IN ('succeeded', 'failed', 'expired', 'dead_letter')",
    )
    .bind(id)
    .bind(message)
    .bind(attempt)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Fails a job that never left the queue with the `cancelled` result. `false` when the job is not queued.
pub async fn cancel_queued(
    tx: &mut TenantTx<'_>,
    id: JobId,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET status = 'failed', result_code = 'cancelled', finished_at = $2, \
                updated_at = now() \
         WHERE id = $1 AND status = 'queued'",
    )
    .bind(id)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Fails the queued jobs of a revoked agent, so they do not wait for a client that never returns.
pub async fn fail_for_revoked_agents(
    tx: &mut TenantTx<'_>,
    now: OffsetDateTime,
) -> Result<u64, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET status = 'failed', result_code = 'failed', \
                result_message = 'agent_revoked', finished_at = $1, updated_at = now() \
         WHERE status = 'queued' \
           AND agent_id IN (SELECT id FROM agents WHERE status = 'revoked')",
    )
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}

/// Marks the lock of a job as possibly stale from `since`, unless it is marked already.
pub async fn note_stale_lock(
    tx: &mut TenantTx<'_>,
    id: JobId,
    since: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET stale_lock_since = $2, updated_at = now() \
         WHERE id = $1 AND stale_lock_since IS NULL",
    )
    .bind(id)
    .bind(since)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Lists and locks the running jobs whose agent is offline and was not seen since `cutoff`.
pub async fn lost_running(
    tx: &mut TenantTx<'_>,
    cutoff: OffsetDateTime,
    online: &[AgentId],
) -> Result<Vec<LostJob>, DbError> {
    let agents: Vec<Uuid> = online.iter().map(|agent| agent.as_uuid()).collect();
    sqlx::query_as::<_, LostJob>(
        "SELECT j.id, GREATEST(a.last_seen_at, j.started_at, j.sent_at) AS lost_at \
         FROM jobs j JOIN agents a ON a.tenant_id = j.tenant_id AND a.id = j.agent_id \
         WHERE j.status = 'running' \
           AND GREATEST(a.last_seen_at, j.started_at, j.sent_at) <= $1 \
           AND NOT (j.agent_id = ANY($2)) \
         FOR UPDATE OF j SKIP LOCKED",
    )
    .bind(cutoff)
    .bind(&agents)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// Fails a running job because its agent is lost, and marks the repository lock as possibly stale.
pub async fn mark_agent_lost(
    tx: &mut TenantTx<'_>,
    id: JobId,
    lost_at: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET status = 'failed', result_code = 'failed', result_message = 'agent_lost', \
                stale_lock_since = $2, finished_at = $3, lease_until = NULL, updated_at = now() \
         WHERE id = $1 AND status = 'running'",
    )
    .bind(id)
    .bind(lost_at)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Lists the repositories whose latest possible stale lock is older than `cutoff`.
pub async fn stale_repositories(
    tx: &mut TenantTx<'_>,
    cutoff: OffsetDateTime,
) -> Result<Vec<StaleRepository>, DbError> {
    sqlx::query_as::<_, StaleRepository>(
        "SELECT repository_id, \
                (array_agg(agent_id ORDER BY stale_lock_since DESC))[1] AS agent_id, \
                max(stale_lock_since) AS stale_since \
         FROM jobs \
         WHERE stale_lock_since IS NOT NULL AND repository_id IS NOT NULL \
         GROUP BY repository_id \
         HAVING max(stale_lock_since) <= $1",
    )
    .bind(cutoff)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// Drops the marks that make a repository a candidate for an unlock.
pub async fn clear_stale_locks(
    tx: &mut TenantTx<'_>,
    repository_id: RepositoryId,
) -> Result<u64, DbError> {
    let result = sqlx::query(
        "UPDATE jobs SET stale_lock_since = NULL WHERE repository_id = $1 \
                AND stale_lock_since IS NOT NULL",
    )
    .bind(repository_id)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}

/// True when a job of the repository succeeded after `since`. Such a job proves that no stale lock
/// blocks the repository.
pub async fn has_success_since(
    tx: &mut TenantTx<'_>,
    repository_id: RepositoryId,
    since: OffsetDateTime,
) -> Result<bool, DbError> {
    let (found,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM jobs WHERE repository_id = $1 AND status = 'succeeded' \
                        AND finished_at > $2)",
    )
    .bind(repository_id)
    .bind(since)
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(found)
}

/// True when the plan holds a queued manual run: a backup without a fire time, a retry included.
pub async fn has_queued_manual_run(
    tx: &mut TenantTx<'_>,
    plan_id: PlanId,
) -> Result<bool, DbError> {
    let (queued,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM jobs WHERE plan_id = $1 AND fire_time IS NULL \
                        AND kind = 'backup' AND status = 'queued')",
    )
    .bind(plan_id)
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(queued)
}

/// True when the repository holds a queued, sent or running job of any kind.
pub async fn has_any_active_for_repository(
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

/// Reads one job with the display names of its plan, agent and repository.
pub async fn get_detail(tx: &mut TenantTx<'_>, id: JobId) -> Result<Option<JobDetail>, DbError> {
    let row = sqlx::query_as::<_, DetailRow>(concat!(
        "SELECT ",
        job_columns!("j."),
        ", p.name AS plan_name, a.hostname AS agent_hostname, r.name AS repository_name \
          FROM jobs j \
          LEFT JOIN plans p ON p.tenant_id = j.tenant_id AND p.id = j.plan_id \
          LEFT JOIN agents a ON a.tenant_id = j.tenant_id AND a.id = j.agent_id \
          LEFT JOIN repositories r ON r.tenant_id = j.tenant_id AND r.id = j.repository_id \
          WHERE j.id = $1"
    ))
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(JobDetail::from))
}

/// Lists the jobs of the tenant, newest first, with keyset pagination.
pub async fn list(
    tx: &mut TenantTx<'_>,
    filter: &JobFilter,
    page: &JobPage,
) -> Result<JobPageResult, DbError> {
    let limit = page.limit.clamp(1, MAX_PAGE_LIMIT);
    let (after_time, after_id) = match page.after {
        Some(cursor) => (Some(cursor.created_at), Some(cursor.id)),
        None => (None, None),
    };
    let rows = sqlx::query_as::<_, DetailRow>(concat!(
        "SELECT ",
        job_columns!("j."),
        ", p.name AS plan_name, a.hostname AS agent_hostname, r.name AS repository_name \
          FROM jobs j \
          LEFT JOIN plans p ON p.tenant_id = j.tenant_id AND p.id = j.plan_id \
          LEFT JOIN agents a ON a.tenant_id = j.tenant_id AND a.id = j.agent_id \
          LEFT JOIN repositories r ON r.tenant_id = j.tenant_id AND r.id = j.repository_id \
          WHERE ($1::uuid IS NULL OR j.plan_id = $1) \
            AND ($2::uuid IS NULL OR j.repository_id = $2) \
            AND ($3::uuid IS NULL OR j.agent_id = $3) \
            AND ($4::text IS NULL OR j.status = $4) \
            AND ($5::timestamptz IS NULL OR (j.created_at, j.id) < ($5, $6)) \
          ORDER BY j.created_at DESC, j.id DESC \
          LIMIT $7"
    ))
    .bind(filter.plan_id)
    .bind(filter.repository_id)
    .bind(filter.agent_id)
    .bind(filter.status.map(JobStatus::as_str))
    .bind(after_time)
    .bind(after_id)
    .bind(limit + 1)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let has_more = rows.len() as i64 > limit;
    let jobs = rows
        .into_iter()
        .take(limit as usize)
        .map(JobDetail::from)
        .collect();
    Ok(JobPageResult { jobs, has_more })
}

#[derive(sqlx::FromRow)]
struct DetailRow {
    #[sqlx(flatten)]
    job: JobRow,
    plan_name: Option<String>,
    agent_hostname: Option<String>,
    repository_name: Option<String>,
}

impl From<DetailRow> for JobDetail {
    fn from(row: DetailRow) -> Self {
        Self {
            job: Job::from(row.job),
            plan_name: row.plan_name,
            agent_hostname: row.agent_hostname,
            repository_name: row.repository_name,
        }
    }
}
