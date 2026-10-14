//! The aggregates behind the Prometheus endpoint (FR-MN-05, AC-MN-03).
//!
//! Every query runs under the tenant policy, so a scrape sees the rows of its own tenant only.
//! The job counters count the jobs that stay in the table: the retention sweep never deletes a job.

use std::collections::BTreeMap;

use time::OffsetDateTime;

use crate::DbError;
use crate::enums::{JobKind, JobStatus};
use crate::ids::{AgentId, PlanId, RepositoryId};
use crate::tx::TenantTx;

/// The upper bounds of the job duration histogram, in seconds.
pub const DURATION_BUCKETS: [f64; 10] = [
    1.0, 5.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0,
];

/// Cumulative histogram of the job durations of one kind.
#[derive(Debug, Clone, PartialEq)]
pub struct DurationHistogram {
    /// For each bound of [`DURATION_BUCKETS`], the jobs that took at most that long.
    pub buckets: Vec<i64>,
    pub count: i64,
    pub sum_seconds: f64,
}

/// Everything one scrape reads. Each query runs under the tenant policy of the transaction.
#[derive(Debug, Clone, Default)]
pub struct MetricsSnapshot {
    /// The time of the last successful backup job of each plan.
    pub last_success: Vec<(PlanId, OffsetDateTime)>,
    pub durations: BTreeMap<&'static str, DurationHistogram>,
    /// Only the final statuses, so that the counter never decreases.
    pub jobs_total: Vec<(JobStatus, i64)>,
    /// For each repository, whether a job that waits or runs reported a held lock.
    pub repo_lock_held: Vec<(RepositoryId, bool)>,
    pub agents: Vec<AgentId>,
}

const TERMINAL: [JobStatus; 4] = [
    JobStatus::Succeeded,
    JobStatus::Failed,
    JobStatus::Expired,
    JobStatus::DeadLetter,
];

/// Reads the aggregates of the tenant of the transaction.
pub async fn snapshot(tx: &mut TenantTx<'_>) -> Result<MetricsSnapshot, DbError> {
    let last_success = sqlx::query_as::<_, (PlanId, OffsetDateTime)>(
        "SELECT plan_id, max(finished_at) FROM jobs \
         WHERE kind = 'backup' AND status = 'succeeded' AND plan_id IS NOT NULL \
           AND finished_at IS NOT NULL \
         GROUP BY plan_id ORDER BY plan_id",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;

    let totals = sqlx::query_as::<_, (String, i64, f64)>(
        "SELECT kind, count(*), COALESCE(sum(extract(epoch FROM finished_at - started_at)), 0)::float8 \
         FROM jobs WHERE started_at IS NOT NULL AND finished_at IS NOT NULL \
           AND finished_at >= started_at \
         GROUP BY kind ORDER BY kind",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let bounds: Vec<f64> = DURATION_BUCKETS.to_vec();
    let bucket_rows = sqlx::query_as::<_, (String, f64, i64)>(
        "SELECT j.kind, b.le, count(*) FROM jobs j CROSS JOIN unnest($1::float8[]) AS b(le) \
         WHERE j.started_at IS NOT NULL AND j.finished_at IS NOT NULL \
           AND extract(epoch FROM j.finished_at - j.started_at) <= b.le \
           AND j.finished_at >= j.started_at \
         GROUP BY j.kind, b.le",
    )
    .bind(&bounds)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let mut durations = BTreeMap::new();
    for (kind, count, sum_seconds) in totals {
        let Some(kind) = JobKind::parse(&kind) else {
            continue;
        };
        let buckets = DURATION_BUCKETS
            .iter()
            .map(|bound| {
                bucket_rows
                    .iter()
                    .find(|(row_kind, le, _)| row_kind == kind.as_str() && le == bound)
                    .map_or(0, |(_, _, n)| *n)
            })
            .collect();
        durations.insert(
            kind.as_str(),
            DurationHistogram {
                buckets,
                count,
                sum_seconds,
            },
        );
    }

    let status_rows =
        sqlx::query_as::<_, (String, i64)>("SELECT status, count(*) FROM jobs GROUP BY status")
            .fetch_all(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    let jobs_total = TERMINAL
        .iter()
        .map(|status| {
            let count = status_rows
                .iter()
                .find(|(name, _)| name == status.as_str())
                .map_or(0, |(_, n)| *n);
            (*status, count)
        })
        .collect();

    let repo_lock_held = sqlx::query_as::<_, (RepositoryId, bool)>(
        "SELECT r.id, EXISTS ( \
             SELECT 1 FROM jobs j WHERE j.repository_id = r.id \
               AND j.status IN ('queued', 'sent', 'running') \
               AND (j.lock_reported OR j.attempt > 0 OR j.stale_lock_since IS NOT NULL)) \
         FROM repositories r ORDER BY r.id",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;

    let agents = sqlx::query_scalar::<_, AgentId>(
        "SELECT id FROM agents WHERE status = 'active' ORDER BY id",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;

    Ok(MetricsSnapshot {
        last_success,
        durations,
        jobs_total,
        repo_lock_held,
        agents,
    })
}
