//! No absence alert during the reconnect grace (AC-ID-06, AC-PR-07, ADR-0036).
//!
//! A database outage stores no contact of any agent, so the first pass after the return reads a
//! stale `last_seen_at` for a host that is on its way back.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use queue_support::{Env, test_queue};
use restichub_cp::queue::RECONNECT_GRACE;
use restichub_db::AgentId;
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

async fn evaluate(env: &Env) -> restichub_cp::alerts::evaluate::Report {
    restichub_cp::alerts::evaluate::evaluate_tenant(
        env.db.database(),
        env.tenant,
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
        !env.state.agent_view().judges_absence(env.now()),
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
        !env.state.agent_view().judges_absence(env.now()),
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
