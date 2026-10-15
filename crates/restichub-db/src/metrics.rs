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

/// The upper bounds of the dispatch delay histogram, in seconds. AC-NFR-01 sets the target at 3 s.
pub const DELAY_BUCKETS: [f64; 10] = [0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 3.0, 5.0, 10.0, 30.0];

/// Cumulative histogram of the job durations of one kind.
#[derive(Debug, Clone, Default, PartialEq)]
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
    /// The time from the moment a job may run to the first send, over all kinds (NFR-PERF-01).
    pub dispatch_delay: DurationHistogram,
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

    // One pass over the jobs: the bounds come from `DURATION_BUCKETS`, one `FILTER` for each. The
    // bounds travel as a bind parameter. Only the array index is formatted into the text: it is a
    // `usize` counter, never input.
    let bounds: Vec<f64> = DURATION_BUCKETS.to_vec();
    let mut filters = String::new();
    for index in 1..=DURATION_BUCKETS.len() {
        if index > 1 {
            filters.push_str(", ");
        }
        filters.push_str(&format!(
            "count(*) FILTER (WHERE secs <= ($1::float8[])[{index}])"
        ));
    }
    let query = format!(
        "WITH finished AS ( \
             SELECT kind, extract(epoch FROM finished_at - started_at)::float8 AS secs FROM jobs \
             WHERE started_at IS NOT NULL AND finished_at IS NOT NULL \
               AND finished_at >= started_at) \
         SELECT kind, count(*), COALESCE(sum(secs), 0)::float8, ARRAY[{filters}]::bigint[] \
         FROM finished GROUP BY kind ORDER BY kind"
    );
    let totals = sqlx::query_as::<_, (String, i64, f64, Vec<i64>)>(sqlx::AssertSqlSafe(query))
        .bind(&bounds)
        .fetch_all(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    let mut durations = BTreeMap::new();
    for (kind, count, sum_seconds, buckets) in totals {
        let Some(kind) = JobKind::parse(&kind) else {
            continue;
        };
        durations.insert(
            kind.as_str(),
            DurationHistogram {
                buckets,
                count,
                sum_seconds,
            },
        );
    }

    let mut delay_filters = String::new();
    for index in 1..=DELAY_BUCKETS.len() {
        if index > 1 {
            delay_filters.push_str(", ");
        }
        delay_filters.push_str(&format!(
            "count(*) FILTER (WHERE secs <= ($1::float8[])[{index}])"
        ));
    }
    let delay_query = format!(
        "WITH sent AS ( \
             SELECT GREATEST(extract(epoch FROM sent_at - run_at), 0)::float8 AS secs FROM jobs \
             WHERE sent_at IS NOT NULL) \
         SELECT count(*), COALESCE(sum(secs), 0)::float8, ARRAY[{delay_filters}]::bigint[] \
         FROM sent"
    );
    let (delay_count, delay_sum, delay_buckets) =
        sqlx::query_as::<_, (i64, f64, Vec<i64>)>(sqlx::AssertSqlSafe(delay_query))
            .bind(DELAY_BUCKETS.to_vec())
            .fetch_one(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    let dispatch_delay = DurationHistogram {
        buckets: delay_buckets,
        count: delay_count,
        sum_seconds: delay_sum,
    };

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
        dispatch_delay,
        jobs_total,
        repo_lock_held,
        agents,
    })
}
