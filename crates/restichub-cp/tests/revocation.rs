//! The revocation of an agent closes its stream (FR-ID-04, AC-ID-04, T17, ADR-0026).
//!
//! The first test revokes an agent during a job on the instance that holds the stream. The second
//! revokes it on another instance of the control plane: the revocation sweep of the instance that
//! holds the stream sees the registry row and closes the stream within five seconds.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use queue_support::{Env, FakeAgent, decode_job_id, test_queue};
use restichub_cp::agent_stream::{REVOCATION_SWEEP_INTERVAL, spawn_revocation_sweep};
use restichub_db::AgentId;
use serde_json::json;

/// AC-ID-04: the stream closes in 5 seconds or less after the revoke.
const CLOSE_LIMIT: Duration = Duration::from_secs(5);

async fn revoke(env: &Env, agent: AgentId) {
    let (status, _, body) = common::send(
        &env.app,
        common::bearer_request(
            "POST",
            &format!("/api/v1/agents/{agent}/revoke"),
            &env.admin,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

/// A queued backup that the agent acknowledged, so it runs. Returns the job identifier.
async fn running_job(env: &Env, agent: &mut FakeAgent) -> String {
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    env.run_plan(plan).await;
    env.pass().await;
    let signed = agent.job().await;
    let job = decode_job_id(&signed);
    agent.start(env, &job).await;
    job
}

/// The stream ends: the next read gives the end or an error.
async fn stream_ends(agent: &mut FakeAgent, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, agent.inbound.message()).await {
            Ok(Ok(Some(_))) => {}
            Ok(Ok(None) | Err(_)) => return true,
            Err(_) => return false,
        }
    }
}

async fn revoke_audit_count(env: &Env) -> i64 {
    let mut tx = env.scoped().await;
    sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'agent_revoke'")
        .fetch_one(&mut *tx)
        .await
        .expect("cannot count the audit records")
}

/// AC-ID-04: an Admin revokes an agent during a job. The control plane sends the cancellation,
/// closes the stream in 5 seconds or less, refuses the next connection and writes the audit record.
#[tokio::test]
async fn ac_id_04_a_revoke_during_a_job_cancels_it_and_closes_the_stream() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("revoke-host").await;
    let job = running_job(&env, &mut agent).await;

    let started = Instant::now();
    revoke(&env, agent.id).await;
    assert_eq!(
        agent.try_cancel(CLOSE_LIMIT).await.as_deref(),
        Some(job.as_str()),
        "the cancellation names the running job"
    );
    assert!(
        stream_ends(&mut agent, CLOSE_LIMIT).await,
        "the stream must close"
    );
    assert!(
        started.elapsed() <= CLOSE_LIMIT,
        "closed in {:?}",
        started.elapsed()
    );

    let refused = env
        .open_raw(&agent.identity)
        .await
        .expect_err("the next connection is refused");
    assert_eq!(refused.code(), tonic::Code::PermissionDenied, "{refused}");
    assert_eq!(revoke_audit_count(&env).await, 1);
    env.db.cleanup().await;
}

/// AC-ID-04: a revoke on another control plane instance closes the stream of this instance in 5
/// seconds or less, with the production sweep interval.
#[tokio::test]
async fn ac_id_04_a_revoke_on_another_instance_closes_the_stream_within_five_seconds() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let other = env.start_instance().await;
    let sweep = spawn_revocation_sweep(other.state.clone(), REVOCATION_SWEEP_INTERVAL);

    let enrolled = env.enroll_agent("remote-revoke-host").await;
    let mut agent = env.connect_to(&other, enrolled).await;
    // The second instance dispatches the job; its registry holds the stream.
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    env.run_plan(plan).await;
    restichub_cp::queue::run_once(&other.state, env.now())
        .await
        .expect("the dispatcher pass");
    let signed = agent.job().await;
    let job = decode_job_id(&signed);
    agent.ack(&job).await;

    let started = Instant::now();
    revoke(&env, agent.id).await;
    assert_eq!(
        agent.try_cancel(CLOSE_LIMIT).await.as_deref(),
        Some(job.as_str()),
        "the instance that holds the stream sends the cancellation"
    );
    assert!(
        stream_ends(&mut agent, CLOSE_LIMIT).await,
        "the stream must close"
    );
    assert!(
        started.elapsed() <= CLOSE_LIMIT,
        "closed in {:?}, over the limit",
        started.elapsed()
    );
    let refused = other.state.agent_registry().is_online(agent.id);
    assert!(!refused, "the instance drops the stream");

    sweep.abort();
    env.db.cleanup().await;
}
