//! The parameters and the results of the snapshot browsing and of a restore (FR-RS-01 to FR-RS-04,
//! ADR-0035).
//!
//! The `jobs` row holds no command (ADR-0010). The row of a `snapshots`, `ls` or `restore` job has
//! a [`JobRequest`] in `job_requests`, and the queue builds the typed steps from it at the first
//! send. The result of a browse job is a [`Listing`] in `job_listings`. The control plane bounds
//! and cleans each value of a listing before it stores the value, because an agent sent it.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::DbError;
use crate::ids::{AgentId, JobId, RepositoryId};
use crate::tx::TenantTx;

/// The policy for a file that exists in the restore target (FR-RS-04).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum OverwriteMode {
    /// Replace every file.
    Always,
    /// Replace a file whose content differs.
    IfChanged,
    /// Replace a file that is older than the snapshot copy.
    IfNewer,
    /// Keep every existing file.
    Never,
}

/// The filters of a snapshot listing. A date filter is applied when the control plane serves the
/// list, so it is not part of the request (FR-RS-01).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotsRequest {
    pub hosts: Vec<String>,
    pub tags: Vec<String>,
    pub paths: Vec<String>,
}

/// One page of the tree of a snapshot (FR-RS-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LsRequest {
    pub snapshot_id: String,
    /// The directory in the snapshot. Empty means the root.
    pub path: String,
    pub offset: u64,
    pub limit: u64,
}

/// A restore of one snapshot to a directory of the agent that runs the job (FR-RS-03, FR-RS-04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreRequest {
    pub snapshot_id: String,
    pub includes: Vec<String>,
    pub excludes: Vec<String>,
    pub target: String,
    pub overwrite: OverwriteMode,
    pub dry_run: bool,
    pub verify: bool,
}

/// The typed parameters of a job whose kind needs more than the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum JobRequest {
    Snapshots(SnapshotsRequest),
    Ls(LsRequest),
    Restore(RestoreRequest),
}

fn decode_error(error: impl std::error::Error + Send + Sync + 'static) -> DbError {
    DbError::Query(sqlx::Error::Decode(Box::new(error)))
}

/// Stores the parameters of a job. A second write keeps the first.
pub async fn insert_request(
    tx: &mut TenantTx<'_>,
    job: JobId,
    request: &JobRequest,
) -> Result<(), DbError> {
    let value = serde_json::to_value(request).map_err(decode_error)?;
    sqlx::query(
        "INSERT INTO job_requests (tenant_id, job_id, request) VALUES ($1, $2, $3) \
         ON CONFLICT DO NOTHING",
    )
    .bind(tx.tenant())
    .bind(job)
    .bind(sqlx::types::Json(value))
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

/// Reads the parameters of a job. `None` for a job that has none.
pub async fn get_request(tx: &mut TenantTx<'_>, job: JobId) -> Result<Option<JobRequest>, DbError> {
    let row: Option<(sqlx::types::Json<serde_json::Value>,)> =
        sqlx::query_as("SELECT request FROM job_requests WHERE job_id = $1")
            .bind(job)
            .fetch_optional(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    row.map(|(json,)| serde_json::from_value(json.0).map_err(decode_error))
        .transpose()
}

/// The result of a browse job, as the control plane stored it.
#[derive(Debug, Clone)]
pub struct Listing {
    pub job_id: JobId,
    pub payload: serde_json::Value,
    pub created_at: OffsetDateTime,
}

/// Stores the result of a browse job. A second write keeps the first.
pub async fn insert_listing(
    tx: &mut TenantTx<'_>,
    job: JobId,
    kind: &str,
    payload: &impl Serialize,
    created_at: OffsetDateTime,
) -> Result<(), DbError> {
    let payload = serde_json::to_value(payload).map_err(decode_error)?;
    sqlx::query(
        "INSERT INTO job_listings (tenant_id, job_id, kind, payload, created_at) \
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
    )
    .bind(tx.tenant())
    .bind(job)
    .bind(kind)
    .bind(sqlx::types::Json(payload))
    .bind(created_at)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

type ListingRow = (JobId, sqlx::types::Json<serde_json::Value>, OffsetDateTime);

fn listing(row: ListingRow) -> Listing {
    Listing {
        job_id: row.0,
        payload: row.1.0,
        created_at: row.2,
    }
}

/// Reads the listing of a job.
pub async fn get_listing(tx: &mut TenantTx<'_>, job: JobId) -> Result<Option<Listing>, DbError> {
    let row = sqlx::query_as::<_, ListingRow>(
        "SELECT job_id, payload, created_at FROM job_listings WHERE job_id = $1",
    )
    .bind(job)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(listing))
}

/// Reads the newest snapshot listing of a repository (FR-RS-01).
pub async fn latest_snapshots(
    tx: &mut TenantTx<'_>,
    repository: RepositoryId,
) -> Result<Option<Listing>, DbError> {
    let row = sqlx::query_as::<_, ListingRow>(
        "SELECT l.job_id, l.payload, l.created_at FROM job_listings l \
         JOIN jobs j ON j.tenant_id = l.tenant_id AND j.id = l.job_id \
         WHERE j.repository_id = $1 AND l.kind = 'snapshots' AND j.status = 'succeeded' \
         ORDER BY l.created_at DESC, l.job_id DESC LIMIT 1",
    )
    .bind(repository)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(listing))
}

/// Reads a page of a snapshot tree that a finished job listed not before `since` (AC-RS-02).
///
/// The same page of a snapshot never changes, so the control plane answers a second request from
/// this listing and starts no job.
pub async fn cached_page(
    tx: &mut TenantTx<'_>,
    repository: RepositoryId,
    request: &LsRequest,
    since: OffsetDateTime,
) -> Result<Option<Listing>, DbError> {
    let offset = i64::try_from(request.offset).unwrap_or(i64::MAX);
    let limit = i64::try_from(request.limit).unwrap_or(i64::MAX);
    let row = sqlx::query_as::<_, ListingRow>(
        "SELECT l.job_id, l.payload, l.created_at FROM job_listings l \
         JOIN jobs j ON j.tenant_id = l.tenant_id AND j.id = l.job_id \
         JOIN job_requests r ON r.tenant_id = l.tenant_id AND r.job_id = l.job_id \
         WHERE j.repository_id = $1 AND l.kind = 'ls' AND j.status = 'succeeded' \
           AND r.request ->> 'snapshot_id' = $2 AND r.request ->> 'path' = $3 \
           AND (r.request ->> 'offset')::bigint = $4 AND (r.request ->> 'limit')::bigint = $5 \
           AND l.created_at >= $6 \
         ORDER BY l.created_at DESC, l.job_id DESC LIMIT 1",
    )
    .bind(repository)
    .bind(&request.snapshot_id)
    .bind(&request.path)
    .bind(offset)
    .bind(limit)
    .bind(since)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(listing))
}

/// The browse job that already runs or waits with the same agent, repository and request.
///
/// The Restore screen asks for the same page again while a job loads it, so the control plane
/// hands out that job instead of a second one.
pub async fn find_pending(
    tx: &mut TenantTx<'_>,
    kind: &str,
    agent: AgentId,
    repository: RepositoryId,
    request: &JobRequest,
    now: OffsetDateTime,
) -> Result<Option<JobId>, DbError> {
    let value = serde_json::to_value(request).map_err(decode_error)?;
    let row: Option<(JobId,)> = sqlx::query_as(
        "SELECT j.id FROM jobs j \
         JOIN job_requests r ON r.tenant_id = j.tenant_id AND r.job_id = j.id \
         WHERE j.kind = $1 AND j.agent_id = $2 AND j.repository_id = $3 \
           AND j.status IN ('queued', 'sent', 'running') \
           AND (j.expires_at IS NULL OR j.expires_at > $5) \
           AND r.request = $4 \
         ORDER BY j.created_at DESC, j.id DESC LIMIT 1",
    )
    .bind(kind)
    .bind(agent)
    .bind(repository)
    .bind(sqlx::types::Json(value))
    .bind(now)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(|(id,)| id))
}

/// Locks the agent row until the end of the transaction. Two control plane instances then count
/// and insert the browse jobs of one agent one after the other, so the cap holds (ADR-0035).
pub async fn lock_agent(tx: &mut TenantTx<'_>, agent: AgentId) -> Result<(), DbError> {
    sqlx::query("SELECT 1 FROM agents WHERE id = $1 FOR UPDATE")
        .bind(agent)
        .fetch_optional(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(())
}

/// The number of browse jobs that wait or run on one agent.
pub async fn count_pending_browse(
    tx: &mut TenantTx<'_>,
    agent: AgentId,
    now: OffsetDateTime,
) -> Result<i64, DbError> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM jobs \
         WHERE kind IN ('snapshots', 'ls') AND agent_id = $1 \
           AND status IN ('queued', 'sent', 'running') \
           AND (expires_at IS NULL OR expires_at > $2)",
    )
    .bind(agent)
    .bind(now)
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(count)
}

/// Deletes the listings older than the given times and returns how many it deleted.
pub async fn delete_expired_listings(
    tx: &mut TenantTx<'_>,
    pages_before: OffsetDateTime,
    snapshots_before: OffsetDateTime,
) -> Result<u64, DbError> {
    let result = sqlx::query(
        "DELETE FROM job_listings WHERE (kind = 'ls' AND created_at < $1) \
                                     OR (kind = 'snapshots' AND created_at < $2)",
    )
    .bind(pages_before)
    .bind(snapshots_before)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}

/// Deletes the finished browse jobs that ended before `before`, with their steps, logs and
/// listings. The Restore screen creates one job for each page of a tree, so the rows do not stay
/// for good like the other jobs (FR-LG-03 keeps the history of backups and restores).
pub async fn delete_old_browse_jobs(
    tx: &mut TenantTx<'_>,
    before: OffsetDateTime,
) -> Result<u64, DbError> {
    let result = sqlx::query(
        "DELETE FROM jobs WHERE kind IN ('snapshots', 'ls') AND finished_at IS NOT NULL \
                            AND finished_at < $1",
    )
    .bind(before)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}
