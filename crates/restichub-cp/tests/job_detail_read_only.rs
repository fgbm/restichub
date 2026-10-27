//! The job detail on a database that serves reads only, and the state that an overdue job shows
//! before the expiry sweep runs (FR-SC-07, AC-SC-07).
//!
//! The read of a job never writes: a control plane that may read the `jobs` table but may not
//! update it answers with the job, and it shows the state that the expiry sweep persists.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use axum::http::StatusCode;
use restichub_db::JobKind;
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use queue_support::{Env, test_queue};

/// Takes the job write away from the runtime login, as a hot standby or a drift of the privileges
/// does.
async fn take_the_job_write(env: &Env) {
    sqlx::query("REVOKE UPDATE ON jobs FROM restichub_app")
        .execute(env.db.owner_pool())
        .await
        .expect("cannot revoke the write of the jobs");
}

async fn give_the_job_write_back(env: &Env) {
    sqlx::query("GRANT UPDATE ON jobs TO restichub_app")
        .execute(env.db.owner_pool())
        .await
        .expect("cannot grant the write of the jobs");
}

/// A queued job of a fresh agent and repository of the environment. Its queue lifetime is one hour.
async fn queued_job(env: &Env) -> Uuid {
    let agent = env.connect_agent("read-only-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    env.enqueue(JobKind::Backup, agent.id, repository, None)
        .await
}

async fn read_job(env: &Env, job: Uuid) -> (StatusCode, Value) {
    env.call(&env.admin, "GET", &format!("/api/v1/jobs/{job}"), None)
        .await
}

/// The lifecycle of the row itself, without the projection of the read.
async fn stored_state(
    env: &Env,
    job: Uuid,
) -> (String, Option<String>, Option<OffsetDateTime>, bool) {
    let mut tx = env.scoped().await;
    let row: (String, Option<String>, Option<OffsetDateTime>, bool) = sqlx::query_as(
        "SELECT status, result_message, finished_at, stale_lock_since IS NOT NULL \
         FROM jobs WHERE id = $1",
    )
    .bind(job)
    .fetch_one(&mut *tx)
    .await
    .expect("cannot read the job row");
    tx.commit().await.expect("cannot commit the read");
    row
}

#[tokio::test]
async fn ac_sc_07_the_job_detail_arrives_on_a_database_that_takes_no_write() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let job = queued_job(&env).await;
    take_the_job_write(&env).await;

    let (status, body) = read_job(&env, job).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], job.to_string());
    assert_eq!(body["status"], "queued");

    give_the_job_write_back(&env).await;
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_07_the_job_detail_shows_an_overdue_job_as_expired_before_the_sweep() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let job = queued_job(&env).await;
    env.clock.advance(2 * 60 * 60);

    let (status, body) = read_job(&env, job).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "expired", "{body}");
    assert!(body["finished_at"].is_string(), "{body}");

    let (stored, result_message, finished_at, stale) = stored_state(&env, job).await;
    assert_eq!(stored, "queued", "the read leaves the row to the sweep");
    assert!(result_message.is_none(), "{result_message:?}");
    assert!(finished_at.is_none(), "{finished_at:?}");
    assert!(!stale);
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_07_the_read_and_the_sweep_agree_on_the_end_of_a_job() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("expiry-rule-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let mut jobs = Vec::new();
    for _ in 0..3 {
        jobs.push(
            env.enqueue(JobKind::Backup, agent.id, repository, None)
                .await,
        );
    }
    env.clock.advance(2 * 60 * 60);
    let now = env.now();

    // A sent job past its lease, a sent job whose lease still runs, and a running job.
    let mut tx = env.scoped().await;
    for (job, status, lease) in [
        (jobs[0], "sent", Some(now - time::Duration::hours(1))),
        (jobs[1], "sent", Some(now + time::Duration::hours(1))),
        (jobs[2], "running", None),
    ] {
        sqlx::query("UPDATE jobs SET status = $2, lease_until = $3 WHERE id = $1")
            .bind(job)
            .bind(status)
            .bind(lease)
            .execute(&mut *tx)
            .await
            .expect("cannot set the lifecycle of the job");
    }
    tx.commit().await.expect("cannot commit the setup");

    let mut shown = Vec::new();
    for &job in &jobs {
        let (status, body) = read_job(&env, job).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        shown.push((
            body["status"].as_str().unwrap_or_default().to_owned(),
            body["result_message"].as_str().map(str::to_owned),
            body["finished_at"].is_string(),
        ));
    }

    restichub_cp::maintenance::expire_jobs(env.db.database(), now)
        .await
        .expect("the expiry sweep");
    for (&job, shown) in jobs.iter().zip(&shown) {
        let (stored, result_message, finished_at, stale) = stored_state(&env, job).await;
        assert_eq!(
            (&stored, &result_message, finished_at.is_some()),
            (&shown.0, &shown.1, shown.2),
            "the read of {job} shows the state of the sweep"
        );
        assert_eq!(
            stale,
            shown.1.as_deref() == Some(restichub_db::jobs::UNACKNOWLEDGED_EXPIRED),
            "the sweep marks the stale lock of {job} as the read shows it"
        );
    }
    env.db.cleanup().await;
}
