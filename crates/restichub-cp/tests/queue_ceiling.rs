//! The maximum duration of maintenance jobs and the hard ceiling of running jobs (ADR-0039).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use restichub_db::JobKind;
use serde_json::json;

use queue_support::{Env, decode_job_id, test_queue};

#[tokio::test]
async fn ac_ag_13_a_maintenance_job_carries_a_maximum_duration() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("ceiling-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env
        .create_profile(json!({"retention": {"keep_daily": 7}}))
        .await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let prune = env
        .enqueue(JobKind::Prune, agent.id, repository, Some(plan))
        .await;
    env.pass().await;
    let signed = agent.job().await;
    assert_eq!(decode_job_id(&signed), prune.to_string());
    assert_eq!(
        env.job(prune).await["limits"]["max_duration_seconds"],
        48 * 3600
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_10_a_job_without_a_maximum_duration_still_ends_at_the_hard_ceiling() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("ceiling-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env
        .create_profile(json!({"options": {"max_duration_minutes": 0}}))
        .await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job = env.run_plan(plan).await;
    env.pass().await;
    let id = decode_job_id(&agent.job().await);
    agent.start(&env, &id).await;

    env.clock.advance(30 * 24 * 3600);
    env.pass().await;
    assert_eq!(env.job_status(job).await, "running");

    env.clock.advance(2 * 24 * 3600);
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_message"], "timed_out");
    env.db.cleanup().await;
}
