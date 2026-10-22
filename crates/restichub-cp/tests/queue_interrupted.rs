//! A job that a stop of the service, a restart of the agent or a reboot interrupts ends as
//! cancelled; a real failure stays failed (FR-MN-03, FR-SC-06, AC-MN-10, AC-SC-10).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use restichub_proto::agent::v1::{JobStatus, StepStatus};
use serde_json::{Value, json};
use uuid::Uuid;

use queue_support::{Env, FakeAgent, decode_job_id, eventually, test_queue};

struct World {
    cp: Env,
    agent: FakeAgent,
    plan: Uuid,
}

async fn world(db: restichub_db::test_util::TestDatabase) -> World {
    let cp = Env::start(db, test_queue()).await;
    let agent = cp.connect_agent("interrupted-host").await;
    let repository = cp.create_repository(agent.id, "pw").await;
    let profile = cp.create_profile(json!({})).await;
    let plan = cp.create_plan(profile, agent.id, repository).await;
    World { cp, agent, plan }
}

/// Starts one backup on the agent and returns the job and its identifier on the wire.
async fn running_job(w: &mut World) -> (Uuid, String) {
    let job = w.cp.run_plan(w.plan).await;
    w.cp.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    w.agent.start(&w.cp, &id).await;
    (job, id)
}

async fn ended(cp: &Env, job: Uuid) -> Value {
    eventually(|| async { cp.job_status(job).await != "running" }).await;
    cp.job(job).await
}

async fn stale_marked(cp: &Env, job: Uuid) -> bool {
    let mut tx = cp.scoped().await;
    let marked = sqlx::query_scalar::<_, bool>(
        "SELECT stale_lock_since IS NOT NULL FROM jobs WHERE id = $1",
    )
    .bind(job)
    .fetch_one(&mut *tx)
    .await
    .expect("cannot read the mark");
    tx.commit().await.expect("cannot end the read");
    marked
}

/// AC-MN-10: the agent stops the running job on a stop of its service, and the cancelled result
/// reaches the control plane after the agent connects again.
#[tokio::test]
async fn ac_mn_10_a_service_stop_ends_the_job_as_cancelled() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    let (job, id) = running_job(&mut w).await;
    w.agent
        .result_with_error(
            &id,
            JobStatus::Cancelled,
            StepStatus::Cancelled,
            Some("cancelled"),
        )
        .await;
    let body = ended(&w.cp, job).await;
    assert_eq!(body["result_code"], "cancelled", "{body}");
    assert_eq!(body["status"], "failed");
    w.cp.db.cleanup().await;
}

/// AC-MN-10: a job that a restart interrupted before it had a result ends as cancelled and keeps
/// the reason.
#[tokio::test]
async fn ac_mn_10_an_interrupted_job_ends_as_cancelled() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    let (job, id) = running_job(&mut w).await;
    w.agent
        .result_with_error(
            &id,
            JobStatus::Failed,
            StepStatus::Failed,
            Some("interrupted"),
        )
        .await;
    let body = ended(&w.cp, job).await;
    assert_eq!(body["result_code"], "cancelled", "{body}");
    assert_eq!(body["result_message"], "interrupted");
    w.cp.db.cleanup().await;
}

/// AC-MN-10: a restic error and a panic of the agent are failures, not interruptions.
#[tokio::test]
async fn ac_mn_10_a_real_failure_stays_failed() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    for code in ["step_failed", "agent_internal_error"] {
        let (job, id) = running_job(&mut w).await;
        w.agent
            .result_with_error(&id, JobStatus::Failed, StepStatus::Failed, Some(code))
            .await;
        let body = ended(&w.cp, job).await;
        assert_eq!(body["result_code"], "failed", "{code}: {body}");
        assert_eq!(body["result_message"], code);
    }
    w.cp.db.cleanup().await;
}

/// AC-SC-10: an agent that restarts and does not list the job has lost it. The job ends as
/// cancelled and its lock stays a suspect for the automatic unlock.
#[tokio::test]
async fn ac_sc_10_an_unlisted_job_ends_as_cancelled_with_a_stale_mark() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    let (job, _id) = running_job(&mut w).await;
    let _restarted = w.cp.reconnect_agent_holding(&w.agent, Vec::new()).await;
    let body = ended(&w.cp, job).await;
    assert_eq!(body["result_code"], "cancelled", "{body}");
    assert_eq!(body["result_message"], "agent_lost");
    assert!(stale_marked(&w.cp, job).await, "the lock may be stale");
    w.cp.db.cleanup().await;
}
