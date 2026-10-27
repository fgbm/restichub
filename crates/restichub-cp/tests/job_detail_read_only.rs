//! The end of an overdue job on the read endpoints (FR-SC-07).
//!
//! A read endpoint ends an overdue job before it answers, so every reader agrees on the state of
//! the job. That write is auxiliary: a database that the control plane may only read answers the
//! same read, with the end of the job shown from memory.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use axum::http::StatusCode;
use restichub_db::restore::{JobRequest, LsRequest};
use restichub_db::{JobId, JobKind};
use serde_json::Value;
use sqlx::AssertSqlSafe;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use queue_support::{Env, test_queue};

/// The role whose privileges the runtime login inherits: the application role of the installation.
async fn application_role(env: &Env) -> String {
    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT r.rolname \
         FROM pg_auth_members m \
         JOIN pg_roles r ON r.oid = m.roleid \
         JOIN pg_roles u ON u.oid = m.member \
         WHERE u.rolname = current_user",
    )
    .fetch_all(env.db.pool())
    .await
    .expect("cannot read the role of the runtime login");
    assert_eq!(roles.len(), 1, "one application role: {roles:?}");
    roles.into_iter().next().expect("the application role")
}

/// Takes the job write away, as a hot standby or a drift of the privileges does.
async fn take_the_job_write(env: &Env) {
    let role = application_role(env).await;
    sqlx::query(AssertSqlSafe(format!(
        "REVOKE UPDATE ON jobs FROM \"{role}\""
    )))
    .execute(env.db.owner_pool())
    .await
    .expect("cannot revoke the write of the jobs");
}

async fn give_the_job_write_back(env: &Env) {
    let role = application_role(env).await;
    sqlx::query(AssertSqlSafe(format!("GRANT UPDATE ON jobs TO \"{role}\"")))
        .execute(env.db.owner_pool())
        .await
        .expect("cannot grant the write of the jobs");
}

/// A queued job of a fresh agent and repository of the environment. Its queue lifetime is one hour.
async fn queued_job(env: &Env, kind: JobKind, hostname: &str) -> (Uuid, Uuid) {
    let agent = env.connect_agent(hostname).await;
    let repository = env.create_repository(agent.id, "pw").await;
    let job = env.enqueue(kind, agent.id, repository, None).await;
    (job, repository)
}

async fn add_the_ls_request(env: &Env, job: Uuid) {
    let mut tx = env
        .db
        .database()
        .begin_tenant(env.tenant)
        .await
        .expect("cannot begin the transaction");
    restichub_db::restore::insert_request(
        &mut tx,
        JobId::from_uuid(job),
        &JobRequest::Ls(LsRequest {
            snapshot_id: "a".repeat(64),
            path: String::new(),
            offset: 0,
            limit: 10,
        }),
    )
    .await
    .expect("cannot insert the ls request");
    tx.commit().await.expect("cannot commit the request");
}

/// The state of one job as a read shows it: status, result code, result message and the end.
#[derive(Debug, PartialEq, Eq)]
struct Shown {
    status: String,
    result_code: Option<String>,
    result_message: Option<String>,
    finished_at: Option<i64>,
}

async fn read_job(env: &Env, job: Uuid) -> (StatusCode, Value) {
    env.call(&env.admin, "GET", &format!("/api/v1/jobs/{job}"), None)
        .await
}

async fn detail_state(env: &Env, job: Uuid) -> Shown {
    let (status, body) = read_job(env, job).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    shown(&body)
}

fn shown(body: &Value) -> Shown {
    let finished_at = body["finished_at"]
        .as_str()
        .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())
        .map(|value| value.unix_timestamp());
    Shown {
        status: body["status"].as_str().unwrap_or_default().to_owned(),
        result_code: body["result_code"].as_str().map(str::to_owned),
        result_message: body["result_message"].as_str().map(str::to_owned),
        finished_at,
    }
}

/// The stored row, without the projection of a read.
async fn stored_state(env: &Env, job: Uuid) -> Shown {
    let mut tx = env.scoped().await;
    let row: (
        String,
        Option<String>,
        Option<String>,
        Option<OffsetDateTime>,
    ) = sqlx::query_as(
        "SELECT status, result_code, result_message, finished_at FROM jobs WHERE id = $1",
    )
    .bind(job)
    .fetch_one(&mut *tx)
    .await
    .expect("cannot read the job row");
    tx.commit().await.expect("cannot commit the read");
    Shown {
        status: row.0,
        result_code: row.1,
        result_message: row.2,
        finished_at: row.3.map(|value| value.unix_timestamp()),
    }
}

/// The status of the job in the list of the tenant.
async fn list_status(env: &Env, job: Uuid) -> String {
    let (status, body) = env.call(&env.admin, "GET", "/api/v1/jobs", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().cloned().unwrap_or_default();
    let item = items
        .iter()
        .find(|item| item["id"] == job.to_string())
        .unwrap_or_else(|| panic!("the job is in the list: {body}"));
    item["status"].as_str().unwrap_or_default().to_owned()
}

/// One job of the environment with the lifecycle that the test needs.
async fn a_job(
    env: &Env,
    kind: JobKind,
    status: &str,
    lease: Option<OffsetDateTime>,
    expires_at: Option<OffsetDateTime>,
    hostname: &str,
) -> (Uuid, Uuid) {
    let (job, repository) = queued_job(env, kind, hostname).await;
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE jobs SET status = $2, lease_until = $3, expires_at = $4 WHERE id = $1")
        .bind(job)
        .bind(status)
        .bind(lease)
        .bind(expires_at)
        .execute(&mut *tx)
        .await
        .expect("cannot set the lifecycle of the job");
    tx.commit().await.expect("cannot commit the setup");
    (job, repository)
}

#[tokio::test]
async fn the_read_ends_an_overdue_job_on_a_writable_database() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let (job, _) = queued_job(&env, JobKind::Backup, "read-only-host").await;
    env.clock.advance(2 * 60 * 60);

    let shown = detail_state(&env, job).await;
    assert_eq!(shown.status, "expired");

    let stored = stored_state(&env, job).await;
    assert_eq!(
        stored.status, "expired",
        "the read stored the end of the job"
    );
    assert!(stored.finished_at.is_some(), "{stored:?}");
    assert_eq!(
        list_status(&env, job).await,
        "expired",
        "the list and the detail agree"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn the_job_detail_arrives_on_a_database_that_takes_no_write() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let (job, _) = queued_job(&env, JobKind::Backup, "read-only-host").await;
    env.clock.advance(2 * 60 * 60);
    take_the_job_write(&env).await;

    let shown = detail_state(&env, job).await;
    assert_eq!(
        shown.status, "expired",
        "the answer holds the end of the job"
    );
    assert!(shown.finished_at.is_some(), "{shown:?}");

    let stored = stored_state(&env, job).await;
    assert_eq!(
        stored.status, "queued",
        "the write reached no row: {stored:?}"
    );
    assert!(stored.finished_at.is_none(), "{stored:?}");

    give_the_job_write_back(&env).await;
    env.db.cleanup().await;
}

#[tokio::test]
async fn the_browse_reads_arrive_on_a_database_that_takes_no_write() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("browse-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let snapshots = env
        .enqueue(JobKind::Snapshots, agent.id, repository, None)
        .await;
    let ls = env.enqueue(JobKind::Ls, agent.id, repository, None).await;
    add_the_ls_request(&env, ls).await;
    take_the_job_write(&env).await;

    let (status, body) = env
        .call(
            &env.admin,
            "GET",
            &format!("/api/v1/repositories/{repository}/snapshots?job={snapshots}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = env
        .call(
            &env.admin,
            "GET",
            &format!("/api/v1/repositories/{repository}/ls?job={ls}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    give_the_job_write_back(&env).await;
    env.db.cleanup().await;
}

#[tokio::test]
async fn the_answer_of_a_read_only_database_equals_the_end_that_the_sweep_writes() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let now = env.now();
    // The end of the queue lifetime is exactly now for the first job, so the boundary is covered.
    let (queued, _) = a_job(
        &env,
        JobKind::Backup,
        "queued",
        None,
        Some(now),
        "queued-host",
    )
    .await;
    let (sent, _) = a_job(
        &env,
        JobKind::Backup,
        "sent",
        Some(now - time::Duration::minutes(1)),
        Some(now - time::Duration::minutes(1)),
        "sent-host",
    )
    .await;
    let (leased, _) = a_job(
        &env,
        JobKind::Backup,
        "sent",
        Some(now + time::Duration::minutes(1)),
        Some(now - time::Duration::minutes(1)),
        "leased-host",
    )
    .await;
    let (running, _) = a_job(
        &env,
        JobKind::Backup,
        "running",
        None,
        Some(now - time::Duration::minutes(1)),
        "running-host",
    )
    .await;
    let jobs = [queued, sent, leased, running];
    take_the_job_write(&env).await;

    let mut answers = Vec::new();
    for &job in &jobs {
        answers.push(detail_state(&env, job).await);
    }

    give_the_job_write_back(&env).await;
    restichub_cp::maintenance::expire_jobs(env.db.database(), now)
        .await
        .expect("the expiry sweep");
    for (&job, answer) in jobs.iter().zip(&answers) {
        assert_eq!(
            &stored_state(&env, job).await,
            answer,
            "the sweep writes the state that {job} already showed"
        );
    }
    env.db.cleanup().await;
}
