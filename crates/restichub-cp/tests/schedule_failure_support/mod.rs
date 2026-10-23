//! Helpers of the tests for a scheduler pass that fails (FR-SC-02).
//!
//! A database trigger makes every job insert for one agent fail, so a plan or a maintenance schedule
//! of that agent fails in the pass and the others do not.

#![allow(clippy::expect_used, dead_code)]

use restichub_db::test_util::TestDatabase;
use restichub_db::{AgentId, RepositoryId, TenantId};
use sqlx::Row;
use time::OffsetDateTime;
use uuid::Uuid;

const TRIGGER: &str = "test_fail_jobs";

/// The state of a plan or of a maintenance schedule row that the scheduler writes.
#[derive(Debug, Clone, PartialEq)]
pub struct RowState {
    pub enabled: bool,
    pub next_run_at: Option<OffsetDateTime>,
    pub schedule_error: Option<String>,
    pub updated_at: OffsetDateTime,
}

/// Installs a trigger that raises on each job insert for the agent.
pub async fn fail_jobs_of_agent(db: &TestDatabase, agent: AgentId) {
    let function = format!(
        "CREATE FUNCTION {TRIGGER}() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
             IF NEW.agent_id = '{}' THEN RAISE EXCEPTION 'injected failure'; END IF; \
             RETURN NEW; \
         END $$",
        agent.as_uuid()
    );
    sqlx::query(sqlx::AssertSqlSafe(function))
        .execute(db.owner_pool())
        .await
        .expect("cannot create the failure function");
    let trigger = format!(
        "CREATE TRIGGER {TRIGGER} BEFORE INSERT ON jobs FOR EACH ROW EXECUTE FUNCTION {TRIGGER}()"
    );
    sqlx::query(sqlx::AssertSqlSafe(trigger))
        .execute(db.owner_pool())
        .await
        .expect("cannot create the failure trigger");
}

/// Removes the failure of [`fail_jobs_of_agent`].
pub async fn heal(db: &TestDatabase) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER {TRIGGER} ON jobs"
    )))
    .execute(db.owner_pool())
    .await
    .expect("cannot drop the failure trigger");
}

/// Runs a statement through the owner connection under the scope of the tenant.
pub async fn owner_exec(db: &TestDatabase, tenant: TenantId, sql: &str, binds: &[Uuid]) {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    scope(&mut tx, tenant).await;
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()));
    for bind in binds {
        query = query.bind(*bind);
    }
    query
        .execute(&mut *tx)
        .await
        .expect("cannot run the statement");
    tx.commit().await.expect("cannot commit the statement");
}

/// Makes the agent the initialized maintenance agent of the repository.
pub async fn arm_repository(
    db: &TestDatabase,
    tenant: TenantId,
    repository: RepositoryId,
    agent: AgentId,
) {
    owner_exec(
        db,
        tenant,
        "UPDATE repositories SET maintenance_agent_id = $2, initialized = true WHERE id = $1",
        &[repository.as_uuid(), agent.as_uuid()],
    )
    .await;
}

pub async fn plan_state(db: &TestDatabase, tenant: TenantId, plan: Uuid) -> RowState {
    state(
        db,
        tenant,
        "SELECT enabled, next_run_at, schedule_error, updated_at FROM plans WHERE id = $1",
        plan,
    )
    .await
}

pub async fn check_schedule_state(
    db: &TestDatabase,
    tenant: TenantId,
    repository: RepositoryId,
) -> RowState {
    state(
        db,
        tenant,
        "SELECT enabled, next_run_at, schedule_error, updated_at FROM maintenance_schedules \
         WHERE repository_id = $1 AND task = 'check'",
        repository.as_uuid(),
    )
    .await
}

/// The number of enabled plans and schedules of the tenant that are due at `now`.
pub async fn due_rows(
    db: &TestDatabase,
    tenant: TenantId,
    table: &'static str,
    now: OffsetDateTime,
) -> i64 {
    let sql = format!(
        "SELECT count(*) FROM {table} WHERE enabled AND next_run_at IS NOT NULL AND next_run_at <= $1"
    );
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    scope(&mut tx, tenant).await;
    let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(now)
        .fetch_one(&mut *tx)
        .await
        .expect("cannot count the rows");
    tx.commit().await.expect("cannot commit the count");
    count
}

pub async fn repository_jobs(db: &TestDatabase, tenant: TenantId, repository: RepositoryId) -> i64 {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    scope(&mut tx, tenant).await;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE repository_id = $1")
        .bind(repository.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .expect("cannot count the jobs");
    tx.commit().await.expect("cannot commit the count");
    count
}

async fn state(db: &TestDatabase, tenant: TenantId, sql: &str, id: Uuid) -> RowState {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    scope(&mut tx, tenant).await;
    let row = sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .expect("cannot read the row");
    tx.commit().await.expect("cannot commit the read");
    RowState {
        enabled: row.get("enabled"),
        next_run_at: row.get("next_run_at"),
        schedule_error: row.get("schedule_error"),
        updated_at: row.get("updated_at"),
    }
}

async fn scope(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, tenant: TenantId) {
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut **tx)
        .await
        .expect("cannot set the tenant scope");
}
