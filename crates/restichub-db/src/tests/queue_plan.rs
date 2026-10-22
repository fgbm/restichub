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

const DUE: i64 = 20;

/// The longest time that one statement of a pass may run. The control plane sets the same limit.
const PASS_TIMEOUT: time::Duration = time::Duration::seconds(10);

/// A prepared statement outlives the transaction that created it on a pooled connection, so the
/// test removes it when it is done.
const PREPARED: &str = "dispatch_pass";

/// Both partial indexes hold the queued and active rows of a tenant only, so a scan of one of them
/// costs what the queue holds and never what the history holds. A scan of `jobs_pkey` would say
/// nothing here: the whole history is in it.
const WAITING_INDEXES: [&str; 2] = ["jobs_waiting_idx", "jobs_in_flight_idx"];

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

async fn agent_of(db: &Database, tenant: TenantId) -> Result<Uuid, Box<dyn std::error::Error>> {
    let mut tx = db.begin_tenant(tenant).await?;
    let agent = sqlx::query_scalar("SELECT id FROM agents WHERE tenant_id = $1")
        .bind(tenant)
        .fetch_one(tx.as_conn())
        .await?;
    tx.commit().await?;
    Ok(agent)
}

/// Opens the transaction of a queue pass: the tenant scope, the statement timeout and the plan mode
/// that the dispatcher and the stale lock recovery set.
async fn pass_tx(
    db: &Database,
    tenant: TenantId,
) -> Result<TenantTx<'_>, Box<dyn std::error::Error>> {
    let mut tx = db.begin_tenant(tenant).await?;
    tx.statement_timeout(PASS_TIMEOUT).await?;
    tx.force_custom_plan().await?;
    Ok(tx)
}

async fn prepare(
    tx: &mut TenantTx<'_>,
    name: &'static str,
) -> Result<(), Box<dyn std::error::Error>> {
    let sql = format!(
        "PREPARE {name}(timestamptz, uuid[], bigint) AS {}",
        queue::DISPATCHABLE_SQL
    );
    sqlx::raw_sql(AssertSqlSafe(sql))
        .execute(tx.as_conn())
        .await?;
    Ok(())
}

async fn execute(
    tx: &mut TenantTx<'_>,
    name: &'static str,
    agent: Uuid,
) -> Result<(), Box<dyn std::error::Error>> {
    let sql = format!("EXECUTE {name}(now(), '{{{agent}}}', 100)");
    sqlx::raw_sql(AssertSqlSafe(sql))
        .execute(tx.as_conn())
        .await?;
    Ok(())
}

async fn explain_execute(
    tx: &mut TenantTx<'_>,
    name: &'static str,
    agent: Uuid,
) -> Result<String, Box<dyn std::error::Error>> {
    let sql = format!("EXPLAIN (FORMAT TEXT) EXECUTE {name}(now(), '{{{agent}}}', 100)");
    let rows = sqlx::raw_sql(AssertSqlSafe(sql))
        .fetch_all(tx.as_conn())
        .await?;
    Ok(rows
        .iter()
        .map(|row| row.get::<String, _>(0))
        .collect::<Vec<_>>()
        .join("\n"))
}

/// The plan of the statement must follow the tenant that runs it, not the tenant that happened to
/// build the plan of the connection first.
#[tokio::test]
async fn ac_nfr_01_the_dispatch_statement_plans_for_the_tenant_that_runs_it() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (small, big) = two_tenants(db.database()).await?;
    // The small tenant holds one agent, one repository and one job of a kind that the queue never
    // dispatches: it looks like a new organization to the planner.
    seed_tenant(db.database(), small).await?;
    seed_tenant(db.database(), big).await?;
    let small_agent = agent_of(db.database(), small).await?;
    let big_agent = seed_queue(db.database(), big).await?;
    // The estimates of the planner decide the plan, so the statistics of the two tenants must be
    // there. Autovacuum does not run within one test.
    sqlx::query("ANALYZE jobs").execute(db.owner_pool()).await?;

    // One connection: PostgreSQL caches the plan of a prepared statement for each connection, so
    // the plan that the small tenant builds is the one that the big tenant gets.
    let single = db.database_with_max(1).await?;
    let mut tx = pass_tx(&single, small).await?;
    prepare(&mut tx, PREPARED).await?;
    tx.commit().await?;

    // The first executions plan for their own values. The sixth may build a generic plan from the
    // estimates of the small tenant, and the plan cache of the connection then keeps that plan for
    // the next tenant. `force_custom_plan` in the pass transaction is what stops this.
    for _ in 0..6 {
        let mut tx = pass_tx(&single, small).await?;
        execute(&mut tx, PREPARED, small_agent).await?;
        tx.commit().await?;
    }

    let mut tx = pass_tx(&single, big).await?;
    let plan = explain_execute(&mut tx, PREPARED, big_agent).await?;
    sqlx::raw_sql("DEALLOCATE ALL")
        .execute(tx.as_conn())
        .await?;
    tx.commit().await?;

    assert!(
        !plan.contains("jobs_tenant_created_idx"),
        "the plan of the big tenant reads its history: {plan}"
    );
    assert!(
        !plan.contains("SubPlan"),
        "the plan of the big tenant ranks the history again for each row: {plan}"
    );
    assert!(
        WAITING_INDEXES.iter().any(|index| plan.contains(index)),
        "the plan of the big tenant reads the waiting rows through {WAITING_INDEXES:?}: {plan}"
    );
    assert!(
        !plan.contains("Seq Scan"),
        "the plan of the big tenant scans the table: {plan}"
    );

    // The statement stays cheap for each tenant, also when the connection ran it for another one.
    let mut last = Duration::ZERO;
    for _ in 0..8 {
        let mut tx = pass_tx(db.database(), big).await?;
        let started = Instant::now();
        let rows = queue::dispatchable(
            &mut tx,
            time::OffsetDateTime::now_utc(),
            &[AgentId::from_uuid(big_agent)],
            100,
        )
        .await?;
        last = started.elapsed();
        assert_eq!(rows.len(), DUE as usize, "the pass selects every due job");
        tx.commit().await?;
    }
    assert!(
        last < Duration::from_secs(1),
        "the dispatch statement takes {last:?} for the big tenant"
    );

    single.pool().close().await;
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
