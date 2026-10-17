//! The displayed host state follows the live stream, as the dispatcher does (FR-ID-07, FR-ID-08,
//! AC-MN-09, AC-ID-06).
//!
//! The manual clock stays still in these tests, so `last_seen_at` stays within the online window:
//! only the stream decides the state.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use queue_support::{Env, FakeAgent, eventually, test_queue};
use restichub_db::AgentId;
use restichub_db::agents as agent_repo;
use serde_json::Value;

async fn start() -> Option<Env> {
    let db = common::test_db().await?;
    Some(Env::start(db, test_queue()).await)
}

async fn get(env: &Env, uri: &str) -> Value {
    let (status, body) = env.call(&env.viewer, "GET", uri, None).await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    body
}

async fn online(env: &Env, agent: AgentId) -> bool {
    get(env, &format!("/api/v1/agents/{agent}")).await["online"]
        .as_bool()
        .expect("the online flag")
}

async fn listed(env: &Env, state: &str, agent: AgentId) -> bool {
    let page = get(env, &format!("/api/v1/agents?state={state}&facets=true")).await;
    page["items"]
        .as_array()
        .expect("the items")
        .iter()
        .any(|item| item["id"] == agent.to_string())
}

async fn dashboard_online(env: &Env) -> i64 {
    get(env, "/api/v1/dashboard").await["hosts"]["online"]
        .as_i64()
        .expect("the online count")
}

async fn wait_online(env: &Env, agent: AgentId, expected: bool) {
    eventually(|| async { online(env, agent).await == expected }).await;
}

/// Closes the agent side of the stream, as a stopped agent does.
fn stop(agent: FakeAgent) -> restichub_emulator::enroll::EnrolledIdentity {
    let FakeAgent {
        identity,
        tx,
        inbound,
        ..
    } = agent;
    drop(tx);
    drop(inbound);
    identity
}

#[tokio::test]
async fn ac_mn_09_a_closed_stream_shows_the_host_offline_at_once_in_the_view_the_list_and_the_dashboard()
 {
    let Some(env) = start().await else {
        return;
    };
    let agent = env.connect_agent("online-host").await;
    let id = agent.id;
    wait_online(&env, id, true).await;
    assert!(listed(&env, "online", id).await);
    assert_eq!(dashboard_online(&env).await, 1);

    let identity = stop(agent);
    wait_online(&env, id, false).await;
    assert!(!env.state.agent_registry().is_online(id));
    assert!(listed(&env, "offline", id).await);
    assert!(!listed(&env, "online", id).await);
    let facets = get(&env, "/api/v1/agents?facets=true").await;
    assert_eq!(facets["facets"]["states"]["online"], 0);
    assert_eq!(facets["facets"]["states"]["offline"], 1);
    assert_eq!(dashboard_online(&env).await, 0);

    let again = env.connect_enrolled(identity).await;
    wait_online(&env, again.id, true).await;
    assert!(listed(&env, "online", id).await);
}

#[tokio::test]
async fn fr_id_08_the_close_of_a_replaced_stream_keeps_the_host_online() {
    let Some(env) = start().await else {
        return;
    };
    let first = env.connect_agent("replaced-host").await;
    let second = env.connect_enrolled(first.identity.clone()).await;
    let id = second.id;
    // The server closes the first stream when the second opens.
    let mut inbound = first.inbound;
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(10), inbound.message()).await {
            Ok(Ok(Some(_))) => continue,
            Ok(Ok(None) | Err(_)) => break,
            Err(_) => panic!("the server must close the replaced stream"),
        }
    }
    drop(first.tx);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(online(&env, id).await);
    assert!(listed(&env, "online", id).await);
    drop(second);
}

#[tokio::test]
async fn ac_mn_09_another_instance_shows_the_stream_state_of_the_instance_that_holds_it() {
    let Some(env) = start().await else {
        return;
    };
    let instance = env.start_instance().await;
    let identity = env.enroll_agent("instance-host").await;
    let agent = env.connect_to(&instance, identity).await;
    let id = agent.id;
    assert!(!env.state.agent_registry().is_online(id));
    wait_online(&env, id, true).await;

    stop(agent);
    wait_online(&env, id, false).await;
}

#[tokio::test]
async fn ac_id_06_the_offline_alert_still_waits_for_its_threshold() {
    let Some(env) = start().await else {
        return;
    };
    let agent = env.connect_agent("alert-host").await;
    let id = agent.id;
    wait_online(&env, id, true).await;
    stop(agent);
    wait_online(&env, id, false).await;

    let evaluate =
        |at| restichub_cp::alerts::evaluate::evaluate_tenant(env.db.database(), env.tenant, at);
    let report = evaluate(env.now()).await.expect("the evaluation");
    assert_eq!(report.opened, 0, "the display is offline, the alert waits");
    let report = evaluate(env.now() + time::Duration::minutes(31))
        .await
        .expect("the evaluation");
    assert_eq!(report.opened, 1, "the default threshold is 30 minutes");
}

async fn stored_stream(env: &Env, agent: AgentId) -> Option<uuid::Uuid> {
    let mut tx = env.scoped().await;
    sqlx::query_scalar("SELECT stream_id FROM agents WHERE id = $1")
        .bind(agent.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .expect("cannot read the stream id")
}

#[tokio::test]
async fn fr_id_08_an_older_stream_write_does_not_replace_a_newer_stream() {
    let Some(env) = start().await else {
        return;
    };
    let identity = env.enroll_agent("ordered-host").await;
    let agent =
        AgentId::from_uuid(uuid::Uuid::parse_str(&identity.agent_id).expect("the agent id"));
    let older = uuid::Uuid::now_v7();
    let newer = uuid::Uuid::now_v7();
    let write = async |open: bool, stream: uuid::Uuid| {
        let mut tx = env
            .state
            .db()
            .begin_tenant(env.tenant)
            .await
            .expect("cannot begin");
        let changed = if open {
            agent_repo::set_stream_open(&mut tx, agent, stream).await
        } else {
            agent_repo::set_stream_closed(&mut tx, agent, stream).await
        }
        .expect("the stream write");
        tx.commit().await.expect("cannot commit");
        changed
    };

    assert!(write(true, newer).await);
    assert!(!write(true, older).await, "a late older open must not win");
    assert!(
        !write(false, older).await,
        "the older close must clear nothing"
    );
    assert_eq!(stored_stream(&env, agent).await, Some(newer));
    assert!(
        write(true, newer).await,
        "a retry of the same stream succeeds"
    );
    assert!(write(false, newer).await);
    assert_eq!(stored_stream(&env, agent).await, None);
}

#[tokio::test]
async fn fr_id_08_a_failed_open_write_is_retried_on_the_next_tick() {
    let Some(env) = start().await else {
        return;
    };
    // The sequence counts the failed writes: it does not roll back with the failing statement.
    sqlx::raw_sql(
        "CREATE SEQUENCE stream_write_failures; \
         GRANT USAGE ON SEQUENCE stream_write_failures TO PUBLIC; \
         CREATE FUNCTION fail_stream_write() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN PERFORM nextval('stream_write_failures'); RAISE EXCEPTION 'injected failure'; END $$; \
         CREATE TRIGGER fail_stream_write BEFORE UPDATE OF stream_id ON agents \
         FOR EACH ROW WHEN (NEW.stream_id IS NOT NULL) EXECUTE FUNCTION fail_stream_write();",
    )
    .execute(env.db.owner_pool())
    .await
    .expect("cannot install the failing trigger");

    let agent = env.connect_agent("retried-host").await;
    let id = agent.id;
    eventually(|| async {
        sqlx::query_scalar::<_, bool>("SELECT is_called FROM stream_write_failures")
            .fetch_one(env.db.owner_pool())
            .await
            .expect("cannot read the failures")
    })
    .await;
    assert!(
        !online(&env, id).await,
        "the failed write shows no open stream"
    );

    sqlx::raw_sql("DROP TRIGGER fail_stream_write ON agents")
        .execute(env.db.owner_pool())
        .await
        .expect("cannot drop the failing trigger");
    tokio::time::pause();
    tokio::time::advance(
        restichub_cp::agent_stream::LAST_SEEN_INTERVAL + std::time::Duration::from_secs(1),
    )
    .await;
    tokio::time::resume();
    wait_online(&env, id, true).await;
    drop(agent);
}
