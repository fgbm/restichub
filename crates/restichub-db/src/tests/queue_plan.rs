//! The plan of the dispatch statement (AC-NFR-01).
//!
//! The dispatcher runs one statement for each queue pass, so its cost must follow the queued and
//! active rows, never the job history of the tenant. A plan that reads the whole history holds the
//! queue of every organization.

use std::time::{Duration, Instant};

use sqlx::{AssertSqlSafe, Row as _};
use uuid::Uuid;

use super::*;
use crate::DbError;
use crate::ids::AgentId;
use crate::queue;
use crate::test_util::test_database;

/// The number of finished jobs in the history of the test tenant.
const HISTORY: i64 = 50_000;

/// The number of due jobs that the dispatcher looks for.
const DUE: i64 = 20;

/// The indexes that hold the queued and active rows of a tenant, so a scan of one of them costs
/// what the queue holds and never what the history holds.
const WAITING_INDEXES: [&str; 4] = [
    "jobs_waiting_idx",
    "jobs_tenant_kind_status_idx",
    "jobs_in_flight_idx",
    "jobs_pkey",
];

/// Writes the history and the due jobs of the tenant, and returns the agent that owns them.
async fn seed_queue(db: &Database, tenant: TenantId) -> Result<Uuid, Box<dyn std::error::Error>> {
    let mut tx = db.begin_tenant(tenant).await?;
    let agent: Uuid = sqlx::query_scalar("SELECT id FROM agents WHERE tenant_id = $1")
        .bind(tenant)
        .fetch_one(tx.as_conn())
        .await?;
    let repository: Uuid = sqlx::query_scalar("SELECT id FROM repositories WHERE tenant_id = $1")
        .bind(tenant)
        .fetch_one(tx.as_conn())
        .await?;
    sqlx::query(
        "INSERT INTO jobs (id, tenant_id, kind, agent_id, repository_id, status, result_code, \
                           created_at, updated_at, sent_at, started_at, finished_at) \
         SELECT gen_random_uuid(), $1, 'backup', $2, $3, 'succeeded', 'succeeded', \
                now() - interval '50 days' + (interval '50 days' * series / $4::double precision), \
                now() - interval '50 days' + (interval '50 days' * series / $4::double precision), \
                now() - interval '50 days' + (interval '50 days' * series / $4::double precision), \
                now() - interval '50 days' + (interval '50 days' * series / $4::double precision), \
                now() - interval '50 days' + (interval '50 days' * series / $4::double precision) \
         FROM generate_series(1, $4) AS series",
    )
    .bind(tenant)
    .bind(agent)
    .bind(repository)
    .bind(HISTORY)
    .execute(tx.as_conn())
    .await?;
    sqlx::query(
        "INSERT INTO jobs (id, tenant_id, kind, agent_id, repository_id, status, created_at, \
                           updated_at, run_at) \
         SELECT gen_random_uuid(), $1, 'backup', $2, $3, 'queued', now() - interval '1 minute', \
                now(), now() - interval '1 minute' \
         FROM generate_series(1, $4) AS series",
    )
    .bind(tenant)
    .bind(agent)
    .bind(repository)
    .bind(DUE)
    .execute(tx.as_conn())
    .await?;
    tx.commit().await?;
    Ok(agent)
}

/// Explains the dispatch statement as the runtime role would plan it, and returns the plan text.
async fn explain(
    db: &Database,
    tenant: TenantId,
    agent: Uuid,
    mode: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let sql = format!(
        "SET ROLE restichub_app; \
         SET restichub.tenant_id = '{tenant}'; \
         SET plan_cache_mode = {mode}; \
         PREPARE dispatch_pass(timestamptz, uuid[], bigint) AS {}; \
         EXPLAIN EXECUTE dispatch_pass(now(), '{{{agent}}}', 100)",
        queue::DISPATCHABLE_SQL
    );
    let rows = sqlx::raw_sql(AssertSqlSafe(sql))
        .fetch_all(db.pool())
        .await?;
    Ok(rows
        .iter()
        .map(|row| row.get::<String, _>(0))
        .collect::<Vec<_>>()
        .concat())
}

#[tokio::test]
async fn ac_nfr_01_the_dispatch_statement_reads_the_waiting_rows_and_not_the_history() -> TestResult
{
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (_, tenant) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant).await?;
    let agent = seed_queue(db.database(), tenant).await?;

    // Both plans of the same statement, so the choice of the planner cannot switch it to a walk
    // over the history of the tenant.
    for mode in ["force_generic_plan", "force_custom_plan"] {
        let plan = explain(db.database(), tenant, agent, mode).await?;
        assert!(
            WAITING_INDEXES.iter().any(|index| plan.contains(index)),
            "the {mode} plan must read the waiting rows through {WAITING_INDEXES:?}: {plan}"
        );
        assert!(
            !plan.contains("jobs_tenant_created_idx"),
            "the {mode} plan must not read the history: {plan}"
        );
        assert!(
            !plan.contains("Seq Scan"),
            "the {mode} plan must not scan the table: {plan}"
        );
    }

    // The sixth execution and the ones after it may use the generic plan, which the estimate of
    // the ranked set decides. The statement stays cheap after it.
    let mut last = Duration::ZERO;
    for _ in 0..8 {
        let mut tx = db.database().begin_tenant(tenant).await?;
        let started = Instant::now();
        let rows = queue::dispatchable(
            &mut tx,
            time::OffsetDateTime::now_utc(),
            &[AgentId::from_uuid(agent)],
            100,
        )
        .await?;
        last = started.elapsed();
        assert_eq!(rows.len(), DUE as usize, "the pass selects every due job");
        tx.commit().await?;
    }
    assert!(
        last < Duration::from_secs(1),
        "the dispatch statement takes {last:?} after the plan is generic"
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_nfr_01_a_dispatch_statement_that_passes_its_timeout_fails_as_a_timeout() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (_, tenant) = two_tenants(db.database()).await?;

    let mut tx = db.database().begin_tenant(tenant).await?;
    tx.statement_timeout(time::Duration::milliseconds(50))
        .await?;
    let error = match sqlx::query("SELECT pg_sleep(2)")
        .execute(tx.as_conn())
        .await
    {
        Ok(_) => panic!("the statement must pass the timeout"),
        Err(error) => DbError::Query(error),
    };
    assert!(
        error.is_statement_timeout(),
        "the error names the timeout: {error:?}"
    );
    drop(tx);

    db.cleanup().await;
    Ok(())
}
