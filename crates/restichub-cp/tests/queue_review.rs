//! The running timeout, the cancel and the retry chain, the order of a retry, the window of the
//! select and the end of an unacknowledged job (FR-SC-04, FR-SC-05, FR-SC-06, FR-SC-09, FR-PR-06,
//! ADR-0020).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use axum::http::StatusCode;
use restichub_db::JobKind;
use serde_json::json;
use uuid::Uuid;

use queue_support::{Env, FakeAgent, decode_job_id, eventually, test_queue};

const QUIET: Duration = Duration::from_millis(400);

async fn number(env: &Env, sql: &'static str, job: Uuid) -> i64 {
    let mut tx = env.scoped().await;
    let value = sqlx::query_scalar::<_, i64>(sql)
        .bind(job)
        .fetch_one(&mut *tx)
        .await
        .expect("cannot read the row");
    tx.commit().await.expect("cannot end the read");
    value
}

async fn signed_stored(env: &Env, job: Uuid) -> bool {
    number(
        env,
        "SELECT (signed_job IS NOT NULL)::int::bigint FROM jobs WHERE id = $1",
        job,
    )
    .await
        == 1
}

async fn stale_marked(env: &Env, job: Uuid) -> bool {
    number(
        env,
        "SELECT (stale_lock_since IS NOT NULL)::int::bigint FROM jobs WHERE id = $1",
        job,
    )
    .await
        == 1
}

struct World {
    env: Env,
    agent: FakeAgent,
    plan: Uuid,
}

async fn world(
    db: restichub_db::test_util::TestDatabase,
    queue: restichub_cp::queue::QueueConfig,
) -> World {
    let env = Env::start(db, queue).await;
    let agent = env.connect_agent("review-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    World { env, agent, plan }
}

async fn cancel(env: &Env, job: Uuid) {
    let (status, body) = env
        .call(
            &env.operator,
            "POST",
            &format!("/api/v1/jobs/{job}/cancel"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
}

/// ADR-0039: a connected agent keeps a job for longer than 12 hours; the default is 48 hours.
#[tokio::test]
async fn ac_sc_10_a_running_job_of_a_connected_agent_is_not_failed_by_the_clock() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    w.agent.start(env, &id).await;

    env.clock.advance(13 * 3600);
    env.pass().await;
    assert_eq!(env.job_status(job).await, "running");
    let limits = &env.job(job).await["limits"];
    assert_eq!(limits["max_duration_seconds"], 48 * 3600);
    assert_eq!(limits["stall_timeout_seconds"], 3600);

    // Only the loss of the agent ends the job.
    let agent_id = w.agent.id;
    drop(w.agent);
    eventually(|| async { !env.state.agent_registry().is_online(agent_id) }).await;
    env.clock.advance(31 * 60);
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_message"], "agent_lost");
    env.db.cleanup().await;
}

/// ADR-0039: an agent that connects and does not list a running job has lost it.
#[tokio::test]
async fn ac_sc_10_a_hello_that_does_not_list_a_running_job_fails_it() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    w.agent.start(env, &id).await;
    env.clock.advance(3 * 60);

    // A hello that lists the job keeps it running.
    let listed = env
        .reconnect_agent_holding(&w.agent, vec![id.clone()])
        .await;
    env.pass().await;
    assert_eq!(env.job_status(job).await, "running");

    let _restarted = env.reconnect_agent_holding(&listed, Vec::new()).await;
    eventually(|| async { env.job_status(job).await == "failed" }).await;
    assert_eq!(env.job(job).await["result_message"], "agent_lost");
    assert!(stale_marked(env, job).await, "the lock may be stale");
    env.db.cleanup().await;
}

/// ADR-0039: a job that carries a maximum duration still fails past it and a grace period.
#[tokio::test]
async fn ac_sc_06_a_running_job_past_its_maximum_duration_fails_while_the_agent_is_online() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("review-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env
        .create_profile(
            repository,
            json!({"options": {"max_duration_minutes": 120}}),
        )
        .await;
    let plan = env.create_plan(profile, agent.id).await;
    let job = env.run_plan(plan).await;
    env.pass().await;
    let id = decode_job_id(&agent.job().await);
    agent.start(&env, &id).await;

    assert_eq!(env.job(job).await["limits"]["max_duration_seconds"], 7200);

    // The agent restarted and lost the job, but it is online again.
    env.clock.advance(2 * 3600 + 5 * 60);
    env.pass().await;
    assert_eq!(env.job_status(job).await, "running", "the grace still runs");
    env.clock.advance(6 * 60);
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_message"], "timed_out");
    assert!(stale_marked(&env, job).await, "the lock may be stale");

    let next = env.run_plan(plan).await;
    env.pass().await;
    assert_eq!(decode_job_id(&agent.job().await), next.to_string());
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_09_a_cancelled_job_with_a_lock_result_does_not_retry() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    w.agent.start(env, &id).await;
    cancel(env, job).await;

    w.agent.lock_held(&id).await;
    eventually(|| async { env.job_status(job).await == "failed" }).await;
    assert_eq!(env.job(job).await["result_code"], "cancelled");
    assert_eq!(env.plan_jobs(w.plan).await.len(), 1, "no retry row");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_09_a_late_acknowledgement_of_an_ended_cancelled_job_gets_the_cancel() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    cancel(env, job).await;
    assert_eq!(
        w.agent.try_cancel(Duration::from_secs(3)).await.as_deref(),
        Some(id.as_str())
    );
    env.clock.advance(61);
    env.pass().await;
    assert_eq!(env.job(job).await["result_code"], "cancelled");

    w.agent.ack(&id).await;
    assert_eq!(
        w.agent.try_cancel(Duration::from_secs(3)).await.as_deref(),
        Some(id.as_str()),
        "the agent that runs the ended job gets the cancel"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_pr_04_the_stored_signed_job_is_gone_once_the_job_runs() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    assert!(signed_stored(env, job).await);
    w.agent.start(env, &id).await;
    assert!(!signed_stored(env, job).await, "the running row holds none");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_05_a_retry_keeps_the_place_of_the_job_in_the_queue() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    w.agent.start(env, &id).await;
    env.clock.advance(5);
    w.agent.lock_held(&id).await;
    eventually(|| async { env.plan_jobs(w.plan).await.len() == 2 }).await;

    let retry = env.plan_jobs(w.plan).await.remove(0);
    let retry = Uuid::parse_str(retry["id"].as_str().expect("id")).expect("job id");
    assert_ne!(retry, job);
    let same = number(
        env,
        "SELECT (j.created_at = o.created_at)::int::bigint FROM jobs j \
         JOIN jobs o ON o.id = j.retry_of WHERE j.id = $1",
        retry,
    )
    .await;
    assert_eq!(same, 1, "the retry copies the creation time of the job");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_04_a_backlog_of_one_repository_never_hides_another_repository_of_the_agent() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut queue = test_queue();
    queue.agent_limit = 500;
    queue.repository_limit = 1;
    let env = Env::start(db, queue).await;
    let mut agent = env.connect_agent("window-host").await;
    let busy = env.create_repository(agent.id, "pw").await;
    for _ in 0..120 {
        env.enqueue(JobKind::Backup, agent.id, busy, None).await;
    }
    let other = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(other, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let wanted = env.run_plan(plan).await;

    env.pass().await;
    let sent = agent.jobs_within(Duration::from_secs(3)).await;
    assert!(
        sent.contains(&wanted.to_string()),
        "the job of the second repository starts in the first pass"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_pr_04_an_unacknowledged_job_past_its_lifetime_fails_and_blocks_the_repository() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    w.agent.job().await;

    // The agent may have run the job and lost the connection before it acknowledged.
    env.clock.advance(3600 + 61);
    env.pass().await;
    assert!(
        w.agent.try_job(QUIET).await.is_none(),
        "no redelivery past the signed expiry"
    );
    restichub_cp::maintenance::expire_jobs(env.db.database(), env.now())
        .await
        .expect("the expiry sweep");
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_message"], "unacknowledged_expired");
    assert!(stale_marked(env, job).await, "the lock may be stale");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_09_a_cancelled_sent_job_ends_while_no_agent_is_online() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    w.agent.job().await;
    cancel(env, job).await;

    let agent_id = w.agent.id;
    drop(w.agent);
    eventually(|| async { !env.state.agent_registry().is_online(agent_id) }).await;
    env.clock.advance(61);
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_code"], "cancelled");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_05_a_dead_letter_job_keeps_its_attempt() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut queue = test_queue();
    queue.lock_retry_limit = 1;
    let mut w = world(db, queue).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    w.agent.start(env, &id).await;
    w.agent.lock_held(&id).await;
    eventually(|| async { env.job_status(job).await == "dead_letter" }).await;
    assert_eq!(env.job(job).await["attempt"], 0);
    assert_eq!(env.plan_jobs(w.plan).await.len(), 1);

    // The key `(plan, fire_time, attempt)` still has room for a later run of the plan.
    let next = env.run_plan(w.plan).await;
    assert_ne!(next, job);
    env.db.cleanup().await;
}
