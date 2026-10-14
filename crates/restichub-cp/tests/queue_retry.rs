//! The lock retries, the dead-letter status and the stale lock recovery (FR-SC-05, FR-SC-06,
//! AC-SC-05, AC-SC-06).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use restichub_proto::agent::v1::{JobStatus, StepStatus, step};
use serde_json::json;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use queue_support::{Env, eventually, test_queue};

const QUIET: Duration = Duration::from_millis(300);

async fn audit_actions(env: &Env, action: &str) -> Vec<String> {
    let mut tx = env.scoped().await;
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT coalesce(diff::text, '') FROM audit_log WHERE action = $1",
    )
    .bind(action)
    .fetch_all(&mut *tx)
    .await
    .expect("cannot read the audit log");
    tx.commit().await.expect("cannot end the read");
    rows
}

async fn repository_jobs(env: &Env, repository: Uuid) -> Vec<serde_json::Value> {
    let (status, _, body) = common::send(
        &env.app,
        common::bearer_request(
            "GET",
            &format!("/api/v1/jobs?repository_id={repository}"),
            &env.admin,
            None,
        ),
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    body["items"].as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn ac_sc_05_a_lock_result_retries_as_a_new_job_with_a_growing_delay_and_then_dead_letters() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("lock-retry").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let mut current = env.run_plan(plan).await;

    let mut delays = Vec::new();
    let mut ids = vec![current];
    for attempt in 1..=5 {
        env.pass().await;
        let Some(signed) = agent.try_job(Duration::from_secs(3)).await else {
            panic!("attempt {attempt}: {}", env.job(current).await);
        };
        let id = queue_support::decode_job_id(&signed);
        assert_eq!(id, current.to_string());
        agent.start(&env, &id).await;
        // Fake restic exits with code 11: the step reports the retry status and the job result has
        // the status `failed`, as the real agent sends it.
        agent.lock_held(&id).await;
        if attempt < 5 {
            eventually(|| async { env.plan_jobs(plan).await.len() == attempt + 1 }).await;
            let body = env.plan_jobs(plan).await.remove(0);
            let next = Uuid::parse_str(body["id"].as_str().expect("id")).expect("job id");
            assert_ne!(next, current, "the retry is a new job");
            assert_eq!(body["status"], "queued");
            assert_eq!(body["attempt"], attempt);
            let ended = env.job(current).await;
            assert_eq!(ended["status"], "failed");
            assert_eq!(ended["result_message"], "lock_retry");
            let run_at =
                time::OffsetDateTime::parse(body["run_at"].as_str().expect("run_at"), &Rfc3339)
                    .expect("run_at is RFC 3339");
            // The stored time has microsecond precision, so the difference rounds to whole seconds.
            let delay = (run_at - env.now()).as_seconds_f64().round() as i64;
            delays.push(delay);
            env.clock.advance(delay - 1);
            env.pass().await;
            assert!(
                agent.try_job(QUIET).await.is_none(),
                "the job waits for run_at"
            );
            env.clock.advance(1);
            current = next;
            ids.push(current);
        } else {
            eventually(|| async { env.job_status(current).await == "dead_letter" }).await;
        }
    }

    assert_eq!(delays, vec![30, 60, 120, 240], "the delay doubles");
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), 5, "each attempt has its own job identifier");
    let body = env.job(current).await;
    assert_eq!(body["status"], "dead_letter");
    assert_eq!(body["result_message"], "lock_retries_exhausted");
    let alerts = audit_actions(&env, "job_dead_letter").await;
    assert_eq!(alerts.len(), 1, "the dead letter raises one alert record");
    assert!(alerts[0].contains(&current.to_string()) || alerts[0].contains("attempts"));
    // The dead letter is final: no later pass sends a job.
    env.clock.advance(7200);
    env.pass().await;
    assert!(agent.try_job(QUIET).await.is_none());
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_06_an_idle_repository_gets_an_unlock_after_thirty_minutes() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("stale-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let job = env.run_plan(plan).await;

    // restic dies with SIGKILL in the middle of the backup.
    env.pass().await;
    let id = queue_support::decode_job_id(&agent.job().await);
    agent.start(&env, &id).await;
    agent.stale_lock_step(&id).await;
    agent
        .result(&id, JobStatus::Failed, StepStatus::Failed)
        .await;
    eventually(|| async { env.job_status(job).await == "failed" }).await;

    // Before thirty minutes the control plane waits.
    env.clock.advance(29 * 60);
    env.pass().await;
    assert!(agent.try_job(QUIET).await.is_none());
    assert_eq!(repository_jobs(&env, repository).await.len(), 1);

    // After thirty minutes the idle repository gets an unlock, never `--remove-all`.
    env.clock.advance(2 * 60);
    env.pass().await;
    let signed = agent.job().await;
    let unlock = env.verify(&signed, &agent).await;
    assert_eq!(unlock.steps.len(), 1);
    assert!(matches!(unlock.steps[0].kind, Some(step::Kind::Unlock(_))));
    let rows = audit_actions(&env, "job_run").await;
    assert!(
        rows.iter().any(|row| row.contains("stale_lock")),
        "the audit log holds the event"
    );
    env.pass().await;
    assert!(agent.try_job(QUIET).await.is_none());
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_06_a_busy_repository_gets_no_unlock_and_a_success_cancels_it() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("busy-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;

    let first = env.run_plan(plan).await;
    env.pass().await;
    let id = queue_support::decode_job_id(&agent.job().await);
    agent.start(&env, &id).await;
    agent.stale_lock_step(&id).await;
    agent
        .result(&id, JobStatus::Failed, StepStatus::Failed)
        .await;
    eventually(|| async { env.job_status(first).await == "failed" }).await;

    // Another backup runs on the repository when the wait ends.
    env.clock.advance(31 * 60);
    let second = env.run_plan(plan).await;
    env.pass().await;
    let next = queue_support::decode_job_id(&agent.job().await);
    assert_eq!(next, second.to_string(), "the backup runs, not an unlock");
    agent.start(&env, &next).await;
    env.pass().await;
    assert!(
        agent.try_job(QUIET).await.is_none(),
        "no unlock while the repository is busy"
    );

    // The backup succeeds, so no lock blocks the repository and no unlock is needed.
    agent.finish(&env, &next).await;
    env.clock.advance(60);
    env.pass().await;
    assert!(agent.try_job(QUIET).await.is_none());
    assert_eq!(repository_jobs(&env, repository).await.len(), 2);
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_06_a_lost_agent_leaves_an_unlock_after_thirty_minutes() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("lost-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let job = env.run_plan(plan).await;
    env.pass().await;
    let id = queue_support::decode_job_id(&agent.job().await);
    agent.start(&env, &id).await;

    let agent_id = agent.id;
    drop(agent);
    eventually(|| async { !env.state.agent_registry().is_online(agent_id) }).await;
    env.clock.advance(29 * 60);
    env.pass().await;
    assert_eq!(
        env.job_status(job).await,
        "running",
        "the wait has not ended"
    );

    env.clock.advance(2 * 60);
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_message"], "agent_lost");
    let jobs = repository_jobs(&env, repository).await;
    assert!(
        jobs.iter()
            .any(|row| row["kind"] == "unlock" && row["status"] == "queued"),
        "an unlock job waits for the agent: {jobs:?}"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_05_a_typed_repository_locked_result_retries_without_a_step_result() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("typed-lock").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let backup = env.run_plan(plan).await;
    env.pass().await;
    let id = queue_support::decode_job_id(&agent.job().await);
    agent.start(&env, &id).await;

    agent
        .result(&id, JobStatus::RepositoryLocked, StepStatus::Failed)
        .await;

    eventually(|| async { env.plan_jobs(plan).await.len() == 2 }).await;
    let newest = env.plan_jobs(plan).await.remove(0);
    assert_eq!(newest["status"], "queued", "{newest}");
    assert_eq!(newest["attempt"], 1);
    let ended = env.job(backup).await;
    assert_eq!(ended["status"], "failed");
    assert_eq!(ended["result_message"], "lock_retry");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_05_a_result_with_warnings_is_a_typed_success() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("typed-warnings").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let backup = env.run_plan(plan).await;
    env.pass().await;
    let id = queue_support::decode_job_id(&agent.job().await);
    agent.start(&env, &id).await;

    agent
        .result(&id, JobStatus::SucceededWithWarnings, StepStatus::Warning)
        .await;

    eventually(|| async { env.job_status(backup).await == "succeeded" }).await;
    let ended = env.job(backup).await;
    assert_eq!(ended["result_code"], "succeeded_with_warnings", "{ended}");
    assert!(ended["result_message"].is_null(), "{ended}");
    env.db.cleanup().await;
}
