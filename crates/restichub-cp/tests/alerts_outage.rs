//! No absence alert during the reconnect grace (AC-ID-06, AC-PR-07, ADR-0036).
//!
//! A database outage stores no contact of any agent, so the first pass after the return reads a
//! stale `last_seen_at` for a host that is on its way back.

#![allow(clippy::expect_used)]

mod alert_support;
mod common;
mod queue_support;

use alert_support::scoped;
use axum::http::StatusCode;
use queue_support::{Env, test_queue};
use restichub_cp::queue::RECONNECT_GRACE;
use restichub_db::{AgentId, AlertId, JobId, TenantId};
use serde_json::json;
use time::{Duration, OffsetDateTime};

async fn start() -> Option<Env> {
    let db = common::test_db().await?;
    Some(Env::start(db, test_queue()).await)
}

/// How long the stored contact of the agent is old.
async fn silent_for(env: &Env, agent: AgentId) -> Duration {
    let mut tx = env.scoped().await;
    let (seen,): (Option<OffsetDateTime>,) =
        sqlx::query_as("SELECT last_seen_at FROM agents WHERE id = $1")
            .bind(agent.as_uuid())
            .fetch_one(&mut *tx)
            .await
            .expect("cannot read the contact");
    tx.commit().await.expect("commit");
    env.now() - seen.expect("the agent contacted the control plane")
}

async fn open_offline(env: &Env, agent: AgentId) -> i64 {
    let mut tx = env.scoped().await;
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM alert_state \
         WHERE kind = 'agent_offline' AND agent_id = $1 AND status = 'open'",
    )
    .bind(agent.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("cannot count the alerts");
    tx.commit().await.expect("commit");
    count
}

async fn open_offline_id(env: &Env, agent: AgentId) -> AlertId {
    let mut tx = env.scoped().await;
    let (id,): (AlertId,) = sqlx::query_as(
        "SELECT id FROM alert_state \
         WHERE kind = 'agent_offline' AND agent_id = $1 AND status = 'open'",
    )
    .bind(agent.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("cannot read the open alert");
    tx.commit().await.expect("commit");
    id
}

async fn resolved_deliveries(env: &Env, alert: AlertId) -> i64 {
    let mut tx = env.scoped().await;
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM alert_deliveries WHERE alert_id = $1 AND event = 'resolved'",
    )
    .bind(alert)
    .fetch_one(&mut *tx)
    .await
    .expect("cannot count the deliveries");
    tx.commit().await.expect("commit");
    count
}

/// A channel that takes the messages of the tenant, so the outbox holds one row for each event.
async fn notification_channel(env: &Env) {
    let (status, body) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/notification-channels",
            Some(json!({"name": "ops", "kind": "email", "recipients": ["ops@example.com"]})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

async fn evaluate(env: &Env) -> restichub_cp::alerts::evaluate::Report {
    evaluate_at(env, env.tenant).await
}

async fn evaluate_at(env: &Env, tenant: TenantId) -> restichub_cp::alerts::evaluate::Report {
    restichub_cp::alerts::evaluate::evaluate_tenant(
        env.db.database(),
        tenant,
        env.now(),
        env.state.agent_view(),
    )
    .await
    .expect("the evaluation")
}

/// Takes the database away for longer than the threshold, so one pass reads no agent.
async fn outage(env: &Env) {
    sqlx::query("REVOKE SELECT ON tenants FROM restichub_app")
        .execute(env.db.owner_pool())
        .await
        .expect("cannot revoke the read of the tenants");
    env.clock.advance(31 * 60);
    let pass = restichub_cp::queue::run_once(&env.state, env.now()).await;
    assert!(pass.is_err(), "the database is away");
    assert!(
        !env.state.agent_view().judges_absence(env.tenant, env.now()),
        "the outage starts the grace again"
    );
}

/// Gives the database back, so one pass reads the agents again.
async fn database_back(env: &Env) {
    sqlx::query("GRANT SELECT ON tenants TO restichub_app")
        .execute(env.db.owner_pool())
        .await
        .expect("cannot grant the read of the tenants");
    env.pass().await;
    assert!(
        !env.state.agent_view().judges_absence(env.tenant, env.now()),
        "the grace runs after the return"
    );
}

#[tokio::test]
async fn ac_id_06_a_database_outage_opens_no_offline_alert_for_a_host_that_connects_again() {
    let Some(env) = start().await else {
        return;
    };
    let agent = env.connect_agent("outage-host").await;
    let id = agent.id;
    env.pass().await;

    outage(&env).await;
    database_back(&env).await;
    assert!(
        silent_for(&env, id).await > Duration::minutes(30),
        "the database stored no contact during the outage, so the host looks silent"
    );

    let report = evaluate(&env).await;
    assert_eq!(report.opened, 0, "no alert in the grace");
    assert_eq!(open_offline(&env, id).await, 0);

    let _agent = env.reconnect_agent(&agent, Vec::new()).await;
    assert!(
        silent_for(&env, id).await <= Duration::minutes(30),
        "the contact is fresh again"
    );
    env.clock.advance(RECONNECT_GRACE.whole_seconds());
    let report = evaluate(&env).await;
    assert_eq!(
        report.opened, 0,
        "the host is online, so no alert opens after the grace either"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_id_06_a_host_that_stays_away_gets_the_offline_alert_after_the_grace() {
    let Some(env) = start().await else {
        return;
    };
    let agent = env.connect_agent("gone-host").await;
    let id = agent.id;
    env.pass().await;
    drop(agent);

    outage(&env).await;
    database_back(&env).await;
    assert_eq!(evaluate(&env).await.opened, 0, "the grace runs");
    assert_eq!(open_offline(&env, id).await, 0);

    env.clock.advance(RECONNECT_GRACE.whole_seconds() + 1);
    assert_eq!(
        evaluate(&env).await.opened,
        1,
        "the threshold of the rule holds after the grace"
    );
    assert_eq!(open_offline(&env, id).await, 1);
    env.db.cleanup().await;
}

/// AC-ID-06: an alert that is open when the database goes away neither closes nor repeats in the
/// grace, and it resolves with one recovery message after the host returns.
#[tokio::test]
async fn ac_id_06_an_open_offline_alert_waits_for_the_grace_before_it_resolves() {
    let Some(env) = start().await else {
        return;
    };
    notification_channel(&env).await;
    let agent = env.connect_agent("returning-host").await;
    let id = agent.id;
    env.pass().await;

    env.clock.advance(31 * 60);
    assert_eq!(
        evaluate(&env).await.opened,
        1,
        "the threshold of the rule holds"
    );
    let alert = open_offline_id(&env, id).await;

    outage(&env).await;
    database_back(&env).await;
    let report = evaluate(&env).await;
    assert_eq!(report.resolved, 0, "an alert does not close in the grace");
    assert_eq!(report.opened, 0);
    assert_eq!(
        open_offline(&env, id).await,
        1,
        "the alert stays open in the grace"
    );
    assert_eq!(
        resolved_deliveries(&env, alert).await,
        0,
        "no recovery message in the grace"
    );

    let _agent = env.reconnect_agent(&agent, Vec::new()).await;
    env.clock.advance(RECONNECT_GRACE.whole_seconds() + 1);
    let report = evaluate(&env).await;
    assert_eq!(
        report.resolved, 1,
        "the host is on its way, the alert closes"
    );
    assert_eq!(open_offline(&env, id).await, 0);
    assert_eq!(
        resolved_deliveries(&env, alert).await,
        1,
        "one recovery message"
    );
    env.db.cleanup().await;
}

/// Two tenants, and only the pass of the second one fails on every attempt. The first tenant holds
/// a host that never connects and a running job of that host.
struct Poisoned {
    env: Env,
    second: TenantId,
    gone: AgentId,
    lost: JobId,
}

async fn poisoned() -> Option<Poisoned> {
    let db = common::test_db().await?;
    let env = Env::start(db, test_queue()).await;
    let gone = env.connect_agent("gone-host").await;
    let gone = gone.id;
    let lost_host = common::seed_agent(env.db.database(), env.tenant, "lost-host").await;
    let lost = JobId::new();
    let mut tx = scoped(&env.db, env.tenant).await;
    sqlx::query(
        "INSERT INTO jobs (id, tenant_id, kind, agent_id, status, started_at, sent_at) \
         VALUES ($1, $2, 'test_access', $3, 'running', $4, $4)",
    )
    .bind(lost)
    .bind(env.tenant)
    .bind(lost_host)
    .bind(env.now())
    .execute(&mut *tx)
    .await
    .expect("cannot insert the running job");
    tx.commit().await.expect("commit");

    let second = common::create_tenant_with_key(env.db.database(), "failing-tenant").await;
    // A host of the second tenant that never connects: without the grace it raises an alert.
    common::seed_agent(env.db.database(), second, "silent-host").await;
    let revoked = common::seed_agent(env.db.database(), second, "revoked-host").await;
    let queued = JobId::new();
    let mut tx = scoped(&env.db, second).await;
    sqlx::query(
        "INSERT INTO jobs (id, tenant_id, kind, agent_id, status) \
         VALUES ($1, $2, 'test_access', $3, 'queued')",
    )
    .bind(queued)
    .bind(second)
    .bind(revoked)
    .execute(&mut *tx)
    .await
    .expect("cannot insert the queued job");
    sqlx::query("UPDATE agents SET status = 'revoked', revoked_at = $2 WHERE id = $1")
        .bind(revoked)
        .bind(env.now())
        .execute(&mut *tx)
        .await
        .expect("cannot revoke the agent");
    tx.commit().await.expect("commit");

    // The pass of the second tenant fails on the queued job of its revoked agent.
    sqlx::raw_sql(
        "CREATE TABLE failing_tenant (tenant_id uuid NOT NULL); \
         GRANT SELECT ON failing_tenant TO PUBLIC; \
         CREATE FUNCTION fail_second_tenant() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN \
           IF EXISTS (SELECT 1 FROM failing_tenant WHERE tenant_id = NEW.tenant_id) THEN \
             RAISE EXCEPTION 'injected failure'; \
           END IF; \
           RETURN NEW; \
         END $$; \
         CREATE TRIGGER fail_second_tenant BEFORE UPDATE ON jobs \
         FOR EACH ROW EXECUTE FUNCTION fail_second_tenant();",
    )
    .execute(env.db.owner_pool())
    .await
    .expect("cannot install the failing trigger");
    sqlx::query("INSERT INTO failing_tenant (tenant_id) VALUES ($1)")
        .bind(second)
        .execute(env.db.owner_pool())
        .await
        .expect("cannot poison the second tenant");

    Some(Poisoned {
        env,
        second,
        gone,
        lost,
    })
}

/// AC-ID-06: a pass that keeps failing in one tenant hides no host of another tenant.
#[tokio::test]
async fn ac_id_06_a_failing_tenant_keeps_the_offline_alert_of_another_tenant() {
    let Some(poisoned) = poisoned().await else {
        return;
    };
    let Poisoned {
        env, second, gone, ..
    } = poisoned;
    env.pass().await;
    env.clock.advance(31 * 60);
    env.pass().await;

    evaluate(&env).await;
    assert_eq!(
        open_offline(&env, gone).await,
        1,
        "the first tenant judges its absent host after the grace"
    );
    assert_eq!(
        evaluate_at(&env, second).await.opened,
        0,
        "the second tenant stays in the grace while its pass fails"
    );

    // The failures continue for an hour, so the second tenant never leaves the grace.
    env.clock.advance(60 * 60);
    env.pass().await;
    assert!(
        !env.state.agent_view().judges_absence(second, env.now()),
        "a failing pass restarts the grace of its tenant"
    );
    assert!(
        env.state.agent_view().judges_absence(env.tenant, env.now()),
        "the grace of the first tenant is not touched"
    );
    assert_eq!(evaluate_at(&env, second).await.opened, 0);
    env.db.cleanup().await;
}

/// AC-PR-07: the stale lock recovery of one tenant runs while the pass of another tenant fails.
#[tokio::test]
async fn ac_pr_07_the_lost_sweep_runs_while_another_tenant_fails() {
    let Some(poisoned) = poisoned().await else {
        return;
    };
    let Poisoned { env, lost, .. } = poisoned;
    env.pass().await;
    env.clock.advance(31 * 60);
    env.pass().await;

    let mut tx = env.scoped().await;
    let (status, message): (String, Option<String>) =
        sqlx::query_as("SELECT status, result_message FROM jobs WHERE id = $1")
            .bind(lost)
            .fetch_one(&mut *tx)
            .await
            .expect("cannot read the lost job");
    tx.commit().await.expect("commit");
    assert_eq!(status, "failed");
    assert_eq!(message.as_deref(), Some("agent_lost"));
    env.db.cleanup().await;
}
