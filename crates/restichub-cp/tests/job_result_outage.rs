//! A job result that the control plane cannot store closes the agent stream, so the agent sends
//! its stored result again after the reconnect (FR-PR-07, ADR-0020).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use restichub_cp::queue::RECONNECT_GRACE;
use restichub_proto::agent::v1::{JobStatus, StepStatus};
use serde_json::Value;
use uuid::Uuid;

use queue_support::{Env, FakeAgent, decode_job_id, eventually, test_queue};

/// Runs one backup of a fresh plan on the agent until the job is `running`.
async fn running_job(env: &Env, agent: &mut FakeAgent) -> (Uuid, String) {
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(serde_json::json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job = env.run_plan(plan).await;
    env.pass().await;
    let signed = agent.job().await;
    let id = decode_job_id(&signed);
    agent.start(env, &id).await;
    (job, id)
}

/// Runs one more backup of the agent to its end and returns its identifier.
async fn another_finished_job(env: &Env, agent: &mut FakeAgent) -> String {
    let repository = env.create_repository(agent.id, "pw").await;
    let job = env.run_fresh_plan(repository, agent.id).await;
    env.pass().await;
    let id = decode_job_id(&agent.job().await);
    assert_eq!(id, job.to_string(), "the next job reaches the agent");
    agent.start(env, &id).await;
    agent.finish(env, &id).await;
    id
}

/// Waits until the control plane ends the stream. A heartbeat before the end is skipped.
async fn stream_closed(agent: &mut FakeAgent) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, agent.inbound.message()).await {
            Ok(Ok(None)) | Ok(Err(_)) => return true,
            Ok(Ok(Some(_))) => {}
            Err(_) => return false,
        }
    }
}

async fn revoke_the_job_write(env: &Env) {
    sqlx::query("REVOKE UPDATE ON jobs FROM restichub_app")
        .execute(env.db.owner_pool())
        .await
        .expect("cannot revoke the write of the jobs");
}

async fn grant_the_job_write(env: &Env) {
    sqlx::query("GRANT UPDATE ON jobs TO restichub_app")
        .execute(env.db.owner_pool())
        .await
        .expect("cannot grant the write of the jobs");
}

fn finished_at(job: &Value) -> Value {
    job.get("finished_at").cloned().unwrap_or(Value::Null)
}

#[tokio::test]
async fn fr_pr_07_a_job_result_during_a_database_outage_arrives_after_the_return() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("outage-host").await;
    let (job, id) = running_job(&env, &mut agent).await;

    // The database takes no write of a job while the agent reports the result.
    revoke_the_job_write(&env).await;
    agent
        .result(&id, JobStatus::Succeeded, StepStatus::Success)
        .await;
    assert!(
        stream_closed(&mut agent).await,
        "the control plane must close the stream when the result is not stored"
    );

    // The database is back. The agent connects again, lists the job as held and sends the stored
    // result (ADR-0039).
    grant_the_job_write(&env).await;
    assert_eq!(
        env.job_status(job).await,
        "running",
        "the control plane kept no result, and the hello of the agent keeps a job that it lists"
    );
    let mut agent = env.reconnect_agent_holding(&agent, vec![id.clone()]).await;
    agent
        .result(&id, JobStatus::Succeeded, StepStatus::Success)
        .await;
    eventually(|| async { env.job_status(job).await == "succeeded" }).await;

    // A second copy of the same result keeps the end of the job.
    let ended = finished_at(&env.job(job).await);
    agent
        .result(&id, JobStatus::Succeeded, StepStatus::Success)
        .await;
    another_finished_job(&env, &mut agent).await;
    assert_eq!(finished_at(&env.job(job).await), ended);
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_pr_07_a_result_for_a_finished_job_keeps_the_stream_open() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("repeated-host").await;
    let (job, id) = running_job(&env, &mut agent).await;
    agent.finish(&env, &id).await;
    let ended = finished_at(&env.job(job).await);

    agent
        .result(&id, JobStatus::Succeeded, StepStatus::Success)
        .await;

    // A job of the same agent still reaches the stream, so the ignored result closed nothing. The
    // stream handles one message at a time, so the end of that job proves that the stream took the
    // ignored result before.
    another_finished_job(&env, &mut agent).await;
    assert_eq!(finished_at(&env.job(job).await), ended);
    env.db.cleanup().await;
}

/// True when the job carries the mark of a possibly stale repository lock.
async fn stale_marked(env: &Env, job: Uuid) -> bool {
    let mut tx = env.scoped().await;
    let marked: bool =
        sqlx::query_scalar("SELECT stale_lock_since IS NOT NULL FROM jobs WHERE id = $1")
            .bind(job)
            .fetch_one(&mut *tx)
            .await
            .expect("cannot read the stale mark");
    marked
}

/// Takes the database away for longer than the wait that fails a lost agent, and lets one
/// dispatcher pass fail during the outage.
async fn outage(env: &Env, agent: &mut FakeAgent, id: &str) {
    revoke_the_job_write(env).await;
    agent
        .result(id, JobStatus::Succeeded, StepStatus::Success)
        .await;
    assert!(
        stream_closed(agent).await,
        "the control plane must close the stream when the result is not stored"
    );
    env.clock.advance(31 * 60);
    env.pass().await;
}

#[tokio::test]
async fn ac_pr_07_a_job_whose_agent_reconnects_after_the_outage_keeps_its_result() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("return-host").await;
    let (job, id) = running_job(&env, &mut agent).await;
    env.pass().await;
    outage(&env, &mut agent, &id).await;

    // The database is back. The first pass after it must not fail a job whose agent is still on its
    // way back: the agent waits up to a minute before it connects again.
    grant_the_job_write(&env).await;
    env.pass().await;
    assert_eq!(
        env.job_status(job).await,
        "running",
        "the agent had no time to connect after the database returned"
    );
    assert!(!stale_marked(&env, job).await, "the lock is not a suspect");

    // The agent connects within the grace, lists the job and sends the result of the journal.
    let agent = env.reconnect_agent_holding(&agent, vec![id.clone()]).await;
    agent
        .result(&id, JobStatus::Succeeded, StepStatus::Success)
        .await;
    eventually(|| async { env.job_status(job).await == "succeeded" }).await;
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_pr_07_a_lost_agent_fails_the_job_after_the_grace() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("lost-after-outage-host").await;
    let (job, id) = running_job(&env, &mut agent).await;
    env.pass().await;
    outage(&env, &mut agent, &id).await;
    drop(agent);

    grant_the_job_write(&env).await;
    env.pass().await;
    assert_eq!(env.job_status(job).await, "running", "the grace runs");

    // No agent connected within the grace, so the job is lost.
    env.clock.advance(RECONNECT_GRACE.whole_seconds() + 1);
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_message"], "agent_lost");
    env.db.cleanup().await;
}
