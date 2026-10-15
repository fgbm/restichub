//! The agent-side repository jobs against the real restic and real network backends (AC-RP-01).
//!
//! The CI job `backends` starts a MinIO service container and a rest-server 0.14 and sets the
//! variables below. Without the variables the tests skip, unless `RESTICHUB_REQUIRE_BACKENDS` asks
//! them to fail.
//!
//! - `RESTICHUB_TEST_S3_ENDPOINT`, `RESTICHUB_TEST_S3_ACCESS_KEY`, `RESTICHUB_TEST_S3_SECRET_KEY`,
//!   `RESTICHUB_TEST_S3_BUCKET`: an S3 account with an existing bucket.
//! - `RESTICHUB_TEST_REST_ENDPOINT`: the base URL of a rest-server without authentication.

#![allow(clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use uuid::Uuid;

/// A backend under test: the settings and the extra write-only fields of the repository.
struct Backend {
    settings: Value,
    credentials: Value,
}

fn variable(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn skip_or_fail(what: &str) -> Option<Backend> {
    if std::env::var_os("RESTICHUB_REQUIRE_BACKENDS").is_some() {
        panic!("RESTICHUB_REQUIRE_BACKENDS is set but the {what} backend is not configured");
    }
    println!("skipping: the {what} backend is not configured");
    None
}

fn s3_backend() -> Option<Backend> {
    let (Some(endpoint), Some(access), Some(secret), Some(bucket)) = (
        variable("RESTICHUB_TEST_S3_ENDPOINT"),
        variable("RESTICHUB_TEST_S3_ACCESS_KEY"),
        variable("RESTICHUB_TEST_S3_SECRET_KEY"),
        variable("RESTICHUB_TEST_S3_BUCKET"),
    ) else {
        return skip_or_fail("S3");
    };
    Some(Backend {
        settings: json!({
            "kind": "s3",
            "endpoint": endpoint,
            "bucket": bucket,
            "prefix": format!("ci-{}", Uuid::now_v7()),
            "region": "us-east-1",
        }),
        credentials: json!({"s3_access_key": access, "s3_secret_key": secret}),
    })
}

fn rest_backend() -> Option<Backend> {
    let Some(endpoint) = variable("RESTICHUB_TEST_REST_ENDPOINT") else {
        return skip_or_fail("rest-server");
    };
    Some(Backend {
        settings: json!({
            "kind": "rest",
            "endpoint": format!("{}/ci-{}", endpoint.trim_end_matches('/'), Uuid::now_v7()),
        }),
        credentials: json!({}),
    })
}

fn result_code(job: &Value) -> Option<&str> {
    job.get("result_code").and_then(Value::as_str)
}

/// Runs init, test-access and a wrong-password test-access of one backend with the real restic.
async fn exercise(name: &str, backend: Backend) {
    let Some(restic) = common::real_restic_or_skip() else {
        return;
    };
    let Some(fixture) = common::stream_fixture(name).await else {
        return;
    };
    let dir = common::temp_config_dir(name);
    let identity = Arc::new(common::register(&fixture, &dir).await);
    let agent = Uuid::parse_str(&identity.agent_id).expect("the agent id");
    let app = common::api_router(fixture.server.state.clone());
    let admin =
        common::seed_session(fixture.db.database(), fixture.tenant, "jobs-admin", "admin").await;

    let password = format!("CANARY-{}", Uuid::now_v7());
    let repository = common::create_repository(
        &app,
        &admin,
        agent,
        backend.settings.clone(),
        &password,
        backend.credentials.clone(),
    )
    .await;

    let jobs = common::real_jobs(&dir, &restic);
    let (stop_tx, task) = common::run_agent(&dir, &identity, &jobs);
    assert!(
        common::wait_online(&fixture, agent).await,
        "the agent must be online"
    );

    let job_id = common::start_action(&app, &admin, &repository, "init").await;
    let job = common::wait_job(&app, &admin, &job_id).await;
    assert_eq!(result_code(&job), Some("succeeded"), "{job}");
    assert_eq!(
        common::repository_view(&app, &admin, &repository)
            .await
            .get("initialized")
            .and_then(Value::as_bool),
        Some(true),
        "a successful init must initialize the repository"
    );

    let job_id = common::start_action(&app, &admin, &repository, "test-access").await;
    let job = common::wait_job(&app, &admin, &job_id).await;
    assert_eq!(result_code(&job), Some("succeeded"), "{job}");

    // A second repository record on the same location with another password: restic exits with 12.
    let wrong = common::create_repository(
        &app,
        &admin,
        agent,
        backend.settings,
        &format!("WRONG-{}", Uuid::now_v7()),
        backend.credentials,
    )
    .await;
    let job_id = common::start_action(&app, &admin, &wrong, "test-access").await;
    let job = common::wait_job(&app, &admin, &job_id).await;
    assert_eq!(result_code(&job), Some("config_error"), "{job}");
    assert_eq!(
        job.get("result_message").and_then(Value::as_str),
        Some("wrong_password"),
        "{job}"
    );

    let journal = std::fs::read(dir.join(restichub_agent::journal::JOURNAL_FILE))
        .expect("the journal must exist");
    assert!(!String::from_utf8_lossy(&journal).contains(&password));

    stop_tx.send(true).expect("cannot stop the agent");
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
}

/// AC-RP-01: the real restic initializes and tests a repository in S3 (MinIO), and a wrong
/// password gives `config_error`.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rp_01_the_real_restic_works_against_s3() {
    if let Some(backend) = s3_backend() {
        exercise("agent-backend-s3", backend).await;
    }
}

/// AC-RP-01: the real restic initializes and tests a repository on a rest-server, and a wrong
/// password gives `config_error`.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rp_01_the_real_restic_works_against_rest_server() {
    if let Some(backend) = rest_backend() {
        exercise("agent-backend-rest", backend).await;
    }
}
