//! The state of each repository for the repository list (FR-RP-01, FR-MN-01, ADR-0036).
//!
//! The lock marks are the ones of the dashboard. The last check and the snapshot count come from the
//! jobs that the control plane already stores; no query starts agent work.

use time::OffsetDateTime;

use crate::DbError;
use crate::enums::JobStatus;
use crate::ids::RepositoryId;
use crate::tx::TenantTx;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RepositoryHealth {
    pub id: RepositoryId,
    /// True when a waiting or running job reported that it holds the lock.
    pub lock_held: bool,
    /// True when a waiting job found a lock that outlived its holder (FR-SC-06).
    pub lock_stale: bool,
    pub check_finished_at: Option<OffsetDateTime>,
    pub check_status: Option<JobStatus>,
    /// The count of the newest snapshot listing; null when it was cut to fit a result.
    pub snapshot_count: Option<i64>,
}

/// Reads the state of the given repositories of the tenant of the transaction.
///
/// A cancelled check does not count: it says nothing about the data. The snapshot count lives as
/// long as the browse job of the Restore screen that listed it.
pub async fn health(
    tx: &mut TenantTx<'_>,
    ids: &[RepositoryId],
) -> Result<Vec<RepositoryHealth>, DbError> {
    let ids: Vec<uuid::Uuid> = ids.iter().map(|id| id.as_uuid()).collect();
    sqlx::query_as::<_, RepositoryHealth>(
        "SELECT r.id, \
                EXISTS (SELECT 1 FROM jobs j WHERE j.tenant_id = r.tenant_id \
                          AND j.repository_id = r.id AND j.status IN ('queued', 'sent', 'running') \
                          AND (j.lock_reported OR j.attempt > 0 OR j.stale_lock_since IS NOT NULL)) \
                    AS lock_held, \
                EXISTS (SELECT 1 FROM jobs j WHERE j.tenant_id = r.tenant_id \
                          AND j.repository_id = r.id AND j.status IN ('queued', 'sent', 'running') \
                          AND j.stale_lock_since IS NOT NULL) AS lock_stale, \
                c.finished_at AS check_finished_at, c.status AS check_status, \
                CASE WHEN NOT s.truncated THEN s.count END AS snapshot_count \
         FROM repositories r \
         LEFT JOIN LATERAL ( \
             SELECT j.finished_at, j.status FROM jobs j \
             WHERE j.tenant_id = r.tenant_id AND j.repository_id = r.id AND j.kind = 'check' \
               AND j.status IN ('succeeded', 'failed', 'dead_letter') \
               AND j.result_code IS DISTINCT FROM 'cancelled' AND j.finished_at IS NOT NULL \
             ORDER BY j.finished_at DESC LIMIT 1) c ON true \
         LEFT JOIN LATERAL ( \
             SELECT (st.summary ->> 'count')::bigint AS count, \
                    (st.summary ->> 'truncated')::boolean AS truncated \
             FROM jobs j \
             JOIN job_steps st ON st.tenant_id = j.tenant_id AND st.job_id = j.id \
                              AND st.output_kind = 'snapshots' \
             WHERE j.tenant_id = r.tenant_id AND j.repository_id = r.id AND j.kind = 'snapshots' \
               AND j.status = 'succeeded' AND j.finished_at IS NOT NULL \
             ORDER BY j.finished_at DESC LIMIT 1) s ON true \
         WHERE r.id = ANY($1)",
    )
    .bind(&ids)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}
