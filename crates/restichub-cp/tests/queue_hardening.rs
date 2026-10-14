//! The expiry of the queue lifetime, the redelivery of the signed bytes, the cancel race, the
//! fairness between agents and the small guards of the queue (FR-SC-04, FR-SC-06, FR-SC-09,
//! FR-PR-06, AC-PR-04, ADR-0020).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use axum::http::StatusCode;
use restichub_db::JobKind;
use restichub_proto::agent::v1::{AgentMessage, JobRejected, RejectReason, agent_message};
use serde_json::{Value, json};
use uuid::Uuid;

use queue_support::{Env, decode_job_id, eventually, test_queue};

const QUIET: Duration = Duration::from_millis(400);

async fn count(env: &Env, sql: &'static str, job: Uuid) -> i64 {
    let mut tx = env.scoped().await;
    let value = sqlx::query_scalar::<_, i64>(sql)
        .bind(job)
        .fetch_one(&mut *tx)
        .await
        .expect("cannot read the row");
    tx.commit().await.expect("cannot end the read");
    value
}

async fn deliveries(env: &Env, job: Uuid) -> i64 {
    count(
        env,
        "SELECT deliveries::bigint FROM jobs WHERE id = $1",
        job,
    )
    .await
}

async fn signed_stored(env: &Env, job: Uuid) -> bool {
    count(
        env,
        "SELECT (signed_job IS NOT NULL)::int::bigint FROM jobs WHERE id = $1",
        job,
    )
    .await
        == 1
}

async fn delivered(env: &Env) -> i64 {
    let mut tx = env.scoped().await;
    let value = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM audit_log WHERE action = 'secret_delivered'",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("cannot read the audit log");
    tx.commit().await.expect("cannot end the read");
    value
}

async fn repository_name(env: &Env, repository: Uuid) -> String {
    let (_, body) = env
        .call(
            &env.admin,
            "GET",
            &format!("/api/v1/repositories/{repository}"),
            None,
        )
        .await;
    body["name"].as_str().expect("name").to_owned()
}

async fn expire(env: &Env) {
    restichub_cp::maintenance::expire_jobs(env.db.database(), env.now())
        .await
        .expect("the expiry sweep");
}

struct World {
    env: Env,
    agent: queue_support::FakeAgent,
    repository: Uuid,
    profile: Uuid,
    plan: Uuid,
}

async fn world(
    db: restichub_db::test_util::TestDatabase,
    queue: restichub_cp::queue::QueueConfig,
) -> World {
    let env = Env::start(db, queue).await;
    let agent = env.connect_agent("hardening-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    World {
        env,
        agent,
        repository,
        profile,
        plan,
    }
}

#[tokio::test]
async fn ac_sc_06_a_running_job_past_its_queue_lifetime_stays_in_flight() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    w.agent.start(env, &id).await;

    env.clock.advance(2 * 3600);
    expire(env).await;
    env.pass().await;
    assert_eq!(env.job_status(job).await, "running", "the sweep skips it");

    // The barrier holds: unlock_all is refused and a prune does not start.
    let name = repository_name(env, w.repository).await;
    let (status, body) = env
        .call(
            &env.admin,
            "POST",
            &format!("/api/v1/repositories/{}/unlock-all", w.repository),
            Some(json!({"confirm": name})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let prune = env
        .enqueue(JobKind::Prune, w.agent.id, w.repository, Some(w.plan))
        .await;
    env.pass().await;
    assert!(w.agent.try_job(QUIET).await.is_none(), "the prune waits");
    assert_eq!(env.job_status(prune).await, "queued");

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

#[tokio::test]
async fn ac_pr_04_a_sent_job_expires_only_after_its_lease_and_its_lifetime() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    w.agent.job().await;

    // The lease runs again at the end of the lifetime, so the lifetime passes inside the lease.
    env.clock.advance(3600 - 1);
    env.pass().await;
    w.agent.job().await;
    env.clock.advance(2);
    expire(env).await;
    assert_eq!(env.job_status(job).await, "sent", "the lease still runs");
    env.clock.advance(60);
    expire(env).await;
    assert_eq!(env.job_status(job).await, "expired");
    assert!(
        !signed_stored(env, job).await,
        "the terminal row drops the blob"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_pr_04_a_redelivery_sends_the_same_signed_bytes_after_a_profile_edit() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    let first = w.agent.job().await;
    assert!(signed_stored(env, job).await);
    let after_first = delivered(env).await;
    assert!(after_first >= 1);

    let (status, body) = env
        .call(
            &env.admin,
            "PATCH",
            &format!("/api/v1/profiles/{}", w.profile),
            Some(json!({
                "name": "edited",
                "repository_id": w.repository.to_string(),
                "paths": {"all": ["/changed"]},
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    env.clock.advance(61);
    env.pass().await;
    let second = w.agent.job().await;
    assert_eq!(first, second, "the redelivery sends the same signed bytes");
    assert_eq!(env.job_status(job).await, "sent");
    assert_eq!(
        delivered(env).await,
        after_first * 2,
        "each delivery writes its secret records"
    );

    w.agent.start(env, &decode_job_id(&first)).await;
    w.agent.finish(env, &decode_job_id(&first)).await;
    assert!(
        !signed_stored(env, job).await,
        "the terminal row drops the blob"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_09_a_cancel_after_the_dispatch_reaches_the_agent_at_the_acknowledgement() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    let id = decode_job_id(&w.agent.job().await);

    let (status, body) = env
        .call(
            &env.operator,
            "POST",
            &format!("/api/v1/jobs/{job}/cancel"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(
        w.agent.try_cancel(Duration::from_secs(3)).await.as_deref(),
        Some(id.as_str()),
        "the first cancel overtakes the job on a real network"
    );
    // The agent started the job after it ignored the early cancel.
    w.agent.ack(&id).await;
    assert_eq!(
        w.agent.try_cancel(Duration::from_secs(3)).await.as_deref(),
        Some(id.as_str()),
        "the acknowledgement of a flagged job gets the cancel again"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_09_a_cancelled_sent_job_is_never_sent_again_and_fails_with_its_lease() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    w.agent.job().await;
    let (status, _) = env
        .call(
            &env.operator,
            "POST",
            &format!("/api/v1/jobs/{job}/cancel"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    env.clock.advance(30);
    env.pass().await;
    assert_eq!(env.job_status(job).await, "sent", "the lease still runs");
    env.clock.advance(31);
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_code"], "cancelled");
    assert!(
        w.agent.try_job(QUIET).await.is_none(),
        "the flagged job never goes out again"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_04_a_backlog_of_one_agent_never_blocks_another_agent() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let busy = env.connect_agent("backlog-a").await;
    let mut other = env.connect_agent("backlog-b").await;
    let busy_repository = env.create_repository(busy.id, "pw").await;
    for _ in 0..150 {
        env.enqueue(JobKind::Backup, busy.id, busy_repository, None)
            .await;
    }
    let repository = env.create_repository(other.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, other.id).await;
    let job = env.run_plan(plan).await;

    env.pass().await;
    assert_eq!(
        decode_job_id(&other.job().await),
        job.to_string(),
        "the job of the other agent starts in the first pass"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_09_a_plan_holds_one_queued_manual_run() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let w = world(db, test_queue()).await;
    let env = &w.env;
    let run = format!("/api/v1/plans/{}/run", w.plan);
    let first = env.run_plan(w.plan).await;
    let (status, body) = env.call(&env.operator, "POST", &run, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "already_queued");

    env.call(
        &env.operator,
        "POST",
        &format!("/api/v1/jobs/{first}/cancel"),
        None,
    )
    .await;
    let (status, body) = env.call(&env.operator, "POST", &run, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_pr_04_a_busy_agent_does_not_use_up_the_deliveries() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut queue = test_queue();
    queue.max_deliveries = 2;
    let mut w = world(db, queue).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    for _ in 0..4 {
        env.pass().await;
        let id = decode_job_id(&w.agent.job().await);
        assert_eq!(deliveries(env, job).await, 1);
        w.agent
            .tx
            .send(AgentMessage {
                payload: Some(agent_message::Payload::JobRejected(JobRejected {
                    job_id: id,
                    reason: RejectReason::Busy as i32,
                })),
            })
            .await
            .expect("cannot send the rejection");
        let mut zero = false;
        for _ in 0..100 {
            if deliveries(env, job).await == 0 {
                zero = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(zero, "a busy refusal is not a delivery");
        env.clock.advance(11);
    }
    assert_eq!(env.job_status(job).await, "sent", "the job never fails");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_06_only_an_admin_cancels_an_unlock_all_job() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let w = world(db, test_queue()).await;
    let env = &w.env;
    let name = repository_name(env, w.repository).await;
    let (status, job) = env
        .call(
            &env.admin,
            "POST",
            &format!("/api/v1/repositories/{}/unlock-all", w.repository),
            Some(json!({"confirm": name})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    let uri = format!("/api/v1/jobs/{}/cancel", job["id"].as_str().expect("id"));
    let (status, _) = env.call(&env.operator, "POST", &uri, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = env.call(&env.admin, "POST", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_06_a_stale_lock_mark_needs_a_sent_or_running_job() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db, test_queue()).await;
    let env = &w.env;
    let marked = |job: Uuid| async move {
        count(
            env,
            "SELECT (stale_lock_since IS NOT NULL)::int::bigint FROM jobs WHERE id = $1",
            job,
        )
        .await
            == 1
    };

    let queued = env.run_plan(w.plan).await;
    w.agent.stale_lock_step(&queued.to_string()).await;
    tokio::time::sleep(QUIET).await;
    assert!(!marked(queued).await, "a queued job takes no mark");

    env.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    w.agent.start(env, &id).await;
    w.agent.finish(env, &id).await;
    w.agent.stale_lock_step(&id).await;
    tokio::time::sleep(QUIET).await;
    assert!(
        !marked(Uuid::parse_str(&id).expect("job id")).await,
        "a finished job takes no mark"
    );
    let _: Value = env.job(queued).await;
    env.db.cleanup().await;
}
