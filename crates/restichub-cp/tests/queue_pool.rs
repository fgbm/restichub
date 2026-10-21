//! A small database pool under the session authentication and the dispatcher (NFR-PERF-01,
//! FR-SC-03, FR-PR-03).
//!
//! A path that holds a transaction never waits for a second pool connection. Otherwise a pool of
//! one connection times out at once, and a busy pool locks up.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use axum::http::StatusCode;
use tokio::task::JoinSet;

use queue_support::{Env, decode_job_id, test_queue};

const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);

async fn me(app: &axum::Router, cookie: &str) -> StatusCode {
    let (status, _, _) = common::send(
        app,
        common::request("GET", "/api/v1/auth/me", Some(cookie), None),
    )
    .await;
    status
}

#[tokio::test]
async fn nfr_perf_01_a_pool_of_one_authenticates_a_session_and_dispatches() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start_with_pool(db, test_queue(), 1, ACQUIRE_TIMEOUT).await;
    let cookie = common::session_cookie(env.db.database(), env.tenant, env.creator_id()).await;
    assert_eq!(me(&env.app, &cookie).await, StatusCode::OK);

    let mut agent = env.connect_agent("pool-one-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let job = env.run_fresh_plan(repository, agent.id).await;
    env.pass().await;
    let signed = agent.job().await;
    assert_eq!(decode_job_id(&signed), job.to_string());
    assert_eq!(env.job_status(job).await, "sent");
    env.db.cleanup().await;
}

#[tokio::test]
async fn nfr_perf_01_concurrent_sessions_and_a_dispatch_fit_a_pool_of_two() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start_with_pool(db, test_queue(), 2, ACQUIRE_TIMEOUT).await;
    let mut agents = Vec::new();
    let mut jobs = Vec::new();
    for index in 0..3 {
        let agent = env.connect_agent(&format!("pool-two-host-{index}")).await;
        let repository = env.create_repository(agent.id, "pw").await;
        for _ in 0..4 {
            jobs.push(env.run_fresh_plan(repository, agent.id).await);
        }
        agents.push(agent);
    }
    let mut cookies = Vec::new();
    for _ in 0..4 {
        cookies.push(common::session_cookie(env.db.database(), env.tenant, env.creator_id()).await);
    }

    let mut requests = JoinSet::new();
    for index in 0..16 {
        let app = env.app.clone();
        let cookie = cookies[index % cookies.len()].clone();
        requests.spawn(async move { me(&app, &cookie).await });
    }
    let work = async {
        env.pass().await;
        let mut statuses = Vec::new();
        while let Some(status) = requests.join_next().await {
            statuses.push(status.expect("the request task"));
        }
        statuses
    };
    let statuses = tokio::time::timeout(Duration::from_secs(10), work)
        .await
        .expect("the requests and the pass end in time");
    assert!(
        statuses.iter().all(|status| *status == StatusCode::OK),
        "{statuses:?}"
    );

    let mut delivered = Vec::new();
    for agent in &mut agents {
        delivered.extend(agent.jobs_within(Duration::from_secs(2)).await);
    }
    assert_eq!(delivered.len(), jobs.len(), "{delivered:?}");
    for job in jobs {
        assert_eq!(env.job_status(job).await, "sent");
    }
    env.db.cleanup().await;
}

/// A database error while the dispatcher reads the key set is transient: the job stays queued and
/// the next pass sends it. It never fails as `signing_failed`.
#[tokio::test]
async fn fr_sc_03_a_transient_signing_error_leaves_the_job_queued() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("signing-retry-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let job = env.run_fresh_plan(repository, agent.id).await;

    sqlx::query("REVOKE SELECT ON signing_keys FROM restichub_app")
        .execute(env.db.owner_pool())
        .await
        .expect("cannot revoke the read of the signing keys");
    env.pass().await;
    assert_eq!(env.job_status(job).await, "queued");
    assert!(agent.try_job(Duration::from_millis(500)).await.is_none());

    sqlx::query("GRANT SELECT ON signing_keys TO restichub_app")
        .execute(env.db.owner_pool())
        .await
        .expect("cannot grant the read of the signing keys");
    env.pass().await;
    let signed = agent.job().await;
    assert_eq!(decode_job_id(&signed), job.to_string());
    assert_eq!(env.job_status(job).await, "sent");
    env.db.cleanup().await;
}
