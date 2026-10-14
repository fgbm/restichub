//! The queue dispatcher: the job builder, the secrets, the lease and the tenant status (FR-PR-03,
//! FR-PR-05, FR-PR-06, FR-SC-03, FR-SC-05, AC-PR-03, AC-PR-04, AC-RP-03, AC-TN-03).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use restichub_proto::agent::v1::step;
use serde_json::json;

use queue_support::{Env, decode_job_id, test_queue};

/// Reads the audit records of one action. The owner login is scoped to the tenant.
async fn audit_rows(env: &Env, action: &str) -> Vec<String> {
    let mut tx = env.scoped().await;
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT coalesce(diff::text, '') || ' ' || coalesce(object_id::text, '') \
         FROM audit_log WHERE action = $1",
    )
    .bind(action)
    .fetch_all(&mut *tx)
    .await
    .expect("cannot read the audit log");
    tx.commit().await.expect("cannot end the read");
    rows
}

async fn delivered_count(env: &Env) -> i64 {
    audit_rows(env, "secret_delivered").await.len() as i64
}

#[tokio::test]
async fn ac_pr_03_the_job_carries_only_the_secrets_of_its_own_repository() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("pr03-host").await;
    let repository_a = env.create_repository(agent.id, "canary-secret-of-a").await;
    let repository_b = env.create_repository(agent.id, "canary-secret-of-b").await;
    let profile = env.create_profile(repository_a, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let other_profile = env.create_profile(repository_b, json!({})).await;
    let _other_plan = env.create_plan(other_profile, agent.id).await;

    let job_id = env.run_plan(plan).await;
    env.pass().await;

    let signed = agent.job().await;
    let job = env.verify(&signed, &agent).await;
    assert_eq!(job.job_id, job_id.to_string());
    assert_eq!(job.repositories.len(), 1, "one repository in the job");
    assert_eq!(job.repositories[0].repository_id, repository_a.to_string());
    assert_eq!(job.repositories[0].password, "canary-secret-of-a");
    assert!(
        !String::from_utf8_lossy(&signed.job).contains("canary-secret-of-b"),
        "the secret of another repository never reaches the job"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_05_every_restic_step_carries_retry_lock() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("lock-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;

    env.run_plan(plan).await;
    env.pass().await;

    let signed = agent.job().await;
    let job = env.verify(&signed, &agent).await;
    let backup = job
        .steps
        .iter()
        .find_map(|step| match &step.kind {
            Some(step::Kind::Backup(backup)) => Some(backup),
            _ => None,
        })
        .expect("the job holds a backup step");
    assert!(
        backup.retry_lock,
        "the control plane always sends --retry-lock"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_rp_03_the_delivery_record_commits_before_the_job_is_sent() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("rp03-host").await;
    let repository = env
        .create_repository(agent.id, "rp03-canary-password")
        .await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;

    env.run_plan(plan).await;
    assert_eq!(
        delivered_count(&env).await,
        0,
        "a queued job reads no secret"
    );
    env.pass().await;
    let signed = agent.job().await;
    let job_id = decode_job_id(&signed);

    // The agent holds the job, so the record is durable already.
    assert!(delivered_count(&env).await >= 1);
    let rows = audit_rows(&env, "secret_delivered").await;
    assert!(
        rows.iter().all(|row| !row.contains("rp03-canary-password")),
        "the record holds no secret value"
    );
    assert!(
        rows.iter().any(|row| row.contains(&job_id)),
        "the record names the job"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_pr_04_an_unacknowledged_job_is_delivered_again_after_the_lease() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("pr04-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let job = env.run_plan(plan).await;

    env.pass().await;
    let first = decode_job_id(&agent.job().await);
    assert_eq!(first, job.to_string());
    assert_eq!(env.job_status(job).await, "sent");

    // Inside the lease the job does not go out again.
    env.clock.advance(30);
    env.pass().await;
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());

    // After the lease it goes out once more, with the same identifier.
    env.clock.advance(31);
    env.pass().await;
    let second = decode_job_id(&agent.job().await);
    assert_eq!(second, first, "the agent journal dedupes by job_id");

    agent.ack(&first).await;
    queue_support::eventually(|| async { env.job_status(job).await == "running" }).await;
    env.clock.advance(120);
    env.pass().await;
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_pr_04_a_job_that_is_never_acknowledged_fails_after_the_delivery_limit() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut queue = test_queue();
    queue.max_deliveries = 2;
    let env = Env::start(db, queue).await;
    let mut agent = env.connect_agent("pr04-limit").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let job = env.run_plan(plan).await;

    for _ in 0..2 {
        env.pass().await;
        agent.job().await;
        env.clock.advance(61);
    }
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_message"], "not_acknowledged");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_03_a_suspended_tenant_dispatches_nothing_and_its_agents_stay_connected() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("tn03-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let job = env.run_plan(plan).await;

    common::set_tenant_status(env.db.database(), env.tenant, "suspended").await;
    env.pass().await;
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());
    assert!(
        env.state.agent_registry().is_online(agent.id),
        "the agent stays connected"
    );

    common::set_tenant_status(env.db.database(), env.tenant, "active").await;
    env.pass().await;
    assert_eq!(decode_job_id(&agent.job().await), job.to_string());
    env.db.cleanup().await;
}

#[tokio::test]
async fn an_offline_agent_gets_no_send_and_no_secret_is_read() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let first = env.connect_agent("later-host").await;
    let repository = env.create_repository(first.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, first.id).await;
    let job = env.run_plan(plan).await;
    let agent_id = first.id;
    drop(first);
    queue_support::eventually(|| async { !env.state.agent_registry().is_online(agent_id) }).await;

    env.pass().await;
    assert_eq!(
        env.job_status(job).await,
        "queued",
        "no online agent, no send"
    );
    assert_eq!(
        delivered_count(&env).await,
        0,
        "no secret is read for an offline agent"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn two_dispatcher_instances_never_send_the_same_job_twice() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("twice-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let mut expected = Vec::new();
    for _ in 0..8 {
        expected.push(env.run_fresh_plan(repository, agent.id).await.to_string());
    }

    let now = env.now();
    let (a, b, c) = tokio::join!(
        restichub_cp::queue::run_once(&env.state, now),
        restichub_cp::queue::run_once(&env.state, now),
        restichub_cp::queue::run_once(&env.state, now),
    );
    a.expect("the first instance");
    b.expect("the second instance");
    c.expect("the third instance");

    let mut delivered = agent.jobs_within(Duration::from_millis(1500)).await;
    delivered.sort();
    expected.sort();
    assert_eq!(delivered, expected, "each job goes out exactly once");
    env.db.cleanup().await;
}

#[tokio::test]
async fn a_profile_that_no_longer_compiles_fails_the_job_with_a_typed_reason() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("compile-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let job = env.run_plan(plan).await;

    // The agent downgrades restic below the supported minimum after the job was queued.
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE agents SET restic_version = '0.1.0' WHERE id = $1")
        .bind(agent.id)
        .execute(&mut *tx)
        .await
        .expect("cannot update the agent");
    tx.commit().await.expect("cannot commit the update");
    env.pass().await;

    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_code"], "config_error");
    assert_eq!(body["result_message"], "compile_failed");
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());
    assert_eq!(
        delivered_count(&env).await,
        0,
        "a refused job reads no secret"
    );
    env.db.cleanup().await;
}
