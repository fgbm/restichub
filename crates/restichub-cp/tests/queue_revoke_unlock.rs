//! The stale lock recovery after a revoke (FR-SC-06, AC-SC-06, AC-ID-04).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use axum::http::StatusCode;
use restichub_db::AgentId;
use serde_json::{Value, json};
use uuid::Uuid;

use queue_support::{Env, decode_job_id, test_queue};

async fn s3_repository(env: &Env) -> Uuid {
    let body = json!({
        "name": format!("repo-{}", Uuid::now_v7()),
        "backend": {"kind": "s3", "endpoint": "https://s3.example.net", "bucket": "backups"},
        "password": "pw",
        "s3_access_key": "access-key",
        "s3_secret_key": "secret-key",
    });
    let (status, repository) = env
        .call(&env.admin, "POST", "/api/v1/repositories", Some(body))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{repository}");
    queue_support::id_of(&repository)
}

async fn repository_jobs(env: &Env, repository: Uuid) -> Vec<Value> {
    let (status, body) = env
        .call(
            &env.admin,
            "GET",
            &format!("/api/v1/jobs?repository_id={repository}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["items"].as_array().cloned().unwrap_or_default()
}

async fn revoke(env: &Env, agent: AgentId) {
    let (status, body) = env
        .call(
            &env.admin,
            "POST",
            &format!("/api/v1/agents/{agent}/revoke"),
            None,
        )
        .await;
    assert!(status.is_success(), "{body}");
}

async fn marks(env: &Env, repository: Uuid) -> i64 {
    let mut tx = env.scoped().await;
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM jobs WHERE repository_id = $1 AND stale_lock_since IS NOT NULL",
    )
    .bind(repository)
    .fetch_one(&mut *tx)
    .await
    .expect("cannot read the marks");
    tx.commit().await.expect("cannot end the read");
    count
}

/// A running backup on `agent` that a revoke ends, with the clock moved past the wait.
async fn revoke_during_backup(env: &Env, repository: Uuid, agent: &mut queue_support::FakeAgent) {
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    env.run_plan(plan).await;
    env.pass().await;
    let id = decode_job_id(&agent.job().await);
    agent.start(env, &id).await;
    revoke(env, agent.id).await;
    env.clock.advance(31 * 60);
    env.pass().await;
}

#[tokio::test]
async fn ac_sc_06_an_unlock_runs_on_another_agent_after_the_revoke_of_the_lock_owner() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut owner = env.connect_agent("lock-owner").await;
    let other = env.connect_agent("lock-successor").await;
    let repository = s3_repository(&env).await;

    revoke_during_backup(&env, repository, &mut owner).await;

    let jobs = repository_jobs(&env, repository).await;
    let unlock = jobs
        .iter()
        .find(|job| job["kind"] == "unlock")
        .unwrap_or_else(|| panic!("no unlock job: {jobs:?}"));
    assert_eq!(unlock["agent_id"], other.id.to_string());
    assert_eq!(
        marks(&env, repository).await,
        0,
        "the unlock clears the mark"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_06_a_lock_without_a_reachable_agent_keeps_its_mark() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut owner = env.connect_agent("disk-owner").await;
    let _other = env.connect_agent("no-disk-access").await;
    // The local repository belongs to the disk of its maintenance agent.
    let repository = env.create_repository(owner.id, "pw").await;

    revoke_during_backup(&env, repository, &mut owner).await;

    let jobs = repository_jobs(&env, repository).await;
    assert!(
        jobs.iter().all(|job| job["kind"] != "unlock"),
        "no agent can unlock a local disk: {jobs:?}"
    );
    assert_eq!(
        marks(&env, repository).await,
        1,
        "the mark stays, so the dashboard shows the suspect lock"
    );
    env.db.cleanup().await;
}
