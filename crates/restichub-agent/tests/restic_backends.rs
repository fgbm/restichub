//! The agent-side repository jobs against the real restic and real network backends (AC-RP-01,
//! AC-AG-10).
//!
//! The CI job `backends` starts an S3 server and two rest-server 0.14 instances and sets the
//! variables below. Without the variables the tests skip, unless `RESTICHUB_REQUIRE_BACKENDS` asks
//! them to fail.
//!
//! - `RESTICHUB_TEST_S3_ENDPOINT`, `RESTICHUB_TEST_S3_ACCESS_KEY`, `RESTICHUB_TEST_S3_SECRET_KEY`,
//!   `RESTICHUB_TEST_S3_BUCKET`: an S3 account with an existing bucket.
//! - `RESTICHUB_TEST_REST_ENDPOINT`: the base URL of a rest-server without authentication.
//! - `RESTICHUB_TEST_REST_AUTH_ENDPOINT`, `RESTICHUB_TEST_REST_AUTH_USERNAME`,
//!   `RESTICHUB_TEST_REST_AUTH_PASSWORD`: a rest-server with an `--htpasswd-file` and one account.

#![allow(clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use restichub_proto::agent::v1::{
    BackupStep, JobStatus, RepositoryAccess, RestCredentials, StepStatus,
    repository_access::BackendCredentials, step, step_result::Output,
};
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

/// The base URL and the account of the rest-server with authentication.
fn rest_auth_server() -> Option<(String, String, String)> {
    let (Some(endpoint), Some(username), Some(password)) = (
        variable("RESTICHUB_TEST_REST_AUTH_ENDPOINT"),
        variable("RESTICHUB_TEST_REST_AUTH_USERNAME"),
        variable("RESTICHUB_TEST_REST_AUTH_PASSWORD"),
    ) else {
        skip_or_fail("rest-server with authentication");
        return None;
    };
    Some((
        endpoint.trim_end_matches('/').to_owned(),
        username,
        password,
    ))
}

fn rest_auth_backend() -> Option<Backend> {
    let (endpoint, username, password) = rest_auth_server()?;
    Some(Backend {
        settings: json!({
            "kind": "rest",
            "endpoint": format!("{endpoint}/ci-{}", Uuid::now_v7()),
            "username": username,
        }),
        credentials: json!({"rest_password": password}),
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

/// Writes `len` bytes that zstd cannot compress, so the packed size equals the source size.
fn write_random_file(path: &std::path::Path, len: usize) {
    let mut state = u64::from(std::process::id()) ^ 0x9e37_79b9_7f4a_7c15;
    let mut data = Vec::with_capacity(len);
    while data.len() < len {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        data.extend_from_slice(&(z ^ (z >> 31)).to_le_bytes());
    }
    data.truncate(len);
    std::fs::write(path, data).expect("cannot write the random file");
}

/// AC-AG-10: a backup step with an upload limit makes the real restic keep the average upload
/// rate to a rest-server at the limit. The test scales 10 MB/s down to 1 MiB/s to stay short.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_10_the_real_restic_keeps_the_upload_limit() {
    const LIMIT_KIB: u64 = 1024;
    const DATA_BYTES: usize = 20 * 1024 * 1024;
    let Some(restic) = common::real_restic_or_skip() else {
        return;
    };
    let Some(endpoint) = variable("RESTICHUB_TEST_REST_ENDPOINT") else {
        skip_or_fail("rest-server");
        return;
    };
    let agent_dir = common::temp_config_dir("upload-rate-agent");
    let data = common::temp_config_dir("upload-rate-data");
    for dir in [&agent_dir, &data] {
        std::fs::create_dir_all(dir).expect("cannot create the directory");
    }
    write_random_file(&data.join("random.bin"), DATA_BYTES);
    let data_path = data.to_string_lossy().into_owned();

    let jobs = common::real_jobs(&agent_dir, &restic);
    let access = RepositoryAccess {
        repository_id: "repo-1".to_owned(),
        location: format!(
            "rest:{}/ci-{}",
            endpoint.trim_end_matches('/'),
            Uuid::now_v7()
        ),
        password: format!("CANARY-{}", Uuid::now_v7()),
        backend_credentials: Some(BackendCredentials::Rest(RestCredentials::default())),
    };
    // Init runs in its own job, so its uploads are outside the measured backup.
    let result = common::run(
        &jobs,
        &common::job(
            "job-init",
            vec![access.clone()],
            vec![common::test_access("repo-1"), common::init("repo-1")],
        ),
    )
    .await;
    assert_eq!(
        result.result().outcome,
        JobStatus::Succeeded as i32,
        "{}",
        result.log_text()
    );

    let backup = common::make_step(
        "backup",
        step::Kind::Backup(BackupStep {
            limit_upload_kib: LIMIT_KIB,
            ..common::backup("repo-1", &[&data_path])
        }),
    );
    let result = common::run(
        &jobs,
        &common::job("job-backup", vec![access], vec![backup]),
    )
    .await;
    assert_eq!(
        result.result().outcome,
        JobStatus::Succeeded as i32,
        "{}",
        result.log_text()
    );
    let Some(Output::BackupSummary(summary)) = result.step("backup").output.clone() else {
        panic!("the backup must give a typed summary");
    };
    // The packed bytes over the restic backup duration: restic counts both, so the rate does not
    // depend on the agent or on the load of the CI runner around the restic process.
    #[allow(clippy::cast_precision_loss)]
    let rate = summary.data_added_packed as f64 / summary.total_duration_seconds;
    #[allow(clippy::cast_precision_loss)]
    let limit = (LIMIT_KIB * 1024) as f64;
    println!(
        "AC-AG-10: {} bytes in {:.2} s, {:.0} B/s, {:.3} of the limit",
        summary.data_added_packed,
        summary.total_duration_seconds,
        rate,
        rate / limit
    );
    assert!(
        summary.data_added_packed >= DATA_BYTES as u64,
        "{summary:?}"
    );
    assert!(rate <= 1.1 * limit, "{rate} B/s is above the limit");
    // Without the limit the local rest-server takes this data in about one second.
    assert!(rate >= 0.5 * limit, "{rate} B/s is far below the limit");

    let _ = std::fs::remove_dir_all(&agent_dir);
    let _ = std::fs::remove_dir_all(&data);
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

/// AC-RP-01: the control plane gives restic the user name and the password of a rest-server with
/// an `--htpasswd-file`: init and test-access succeed, and a wrong repository password gives
/// `config_error`.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rp_01_the_real_restic_works_against_rest_server_with_auth() {
    if let Some(backend) = rest_auth_backend() {
        exercise("agent-backend-rest-auth", backend).await;
    }
}

/// AC-RP-01, AC-AG-04: the real restic runs init, backup and snapshots on a rest-server with
/// authentication; without the rest-server password the same repository refuses the access.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rp_01_the_real_restic_backs_up_to_rest_server_with_auth() {
    let Some(restic) = common::real_restic_or_skip() else {
        return;
    };
    let Some((endpoint, username, rest_password)) = rest_auth_server() else {
        return;
    };
    let agent_dir = common::temp_config_dir("rest-auth-agent");
    let data = common::temp_config_dir("rest-auth-data");
    for dir in [&agent_dir, &data] {
        std::fs::create_dir_all(dir).expect("cannot create the directory");
    }
    std::fs::write(data.join("file.txt"), b"rest-server data").expect("cannot write the file");
    let data_path = data.to_string_lossy().into_owned();

    let jobs = common::real_jobs(&agent_dir, &restic);
    let access = RepositoryAccess {
        repository_id: "repo-1".to_owned(),
        location: format!("rest:{endpoint}/ci-{}", Uuid::now_v7()),
        password: format!("CANARY-{}", Uuid::now_v7()),
        backend_credentials: Some(BackendCredentials::Rest(RestCredentials {
            username: username.clone(),
            password: rest_password.clone(),
        })),
    };
    let result = common::run(
        &jobs,
        &common::job(
            "job-rest-auth",
            vec![access.clone()],
            vec![
                common::test_access("repo-1"),
                common::init("repo-1"),
                common::backup_step("repo-1", &[&data_path]),
                common::snapshots_step("repo-1"),
            ],
        ),
    )
    .await;
    assert_eq!(
        result.result().outcome,
        JobStatus::Succeeded as i32,
        "{}",
        result.log_text()
    );
    let Some(Output::SnapshotList(list)) = result.step("snapshots").output.clone() else {
        panic!("snapshots must give a list");
    };
    assert_eq!(list.snapshots.len(), 1);
    assert!(!result.log_text().contains(&rest_password));

    let anonymous = RepositoryAccess {
        backend_credentials: Some(BackendCredentials::Rest(RestCredentials::default())),
        ..access
    };
    let result = common::run(
        &jobs,
        &common::job(
            "job-rest-anonymous",
            vec![anonymous],
            vec![common::snapshots_step("repo-1")],
        ),
    )
    .await;
    assert_ne!(
        result.step("snapshots").status,
        StepStatus::Success as i32,
        "{}",
        result.log_text()
    );

    let _ = std::fs::remove_dir_all(&agent_dir);
    let _ = std::fs::remove_dir_all(&data);
}
