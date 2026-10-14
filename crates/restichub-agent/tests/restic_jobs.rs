//! The agent-side execution of the repository jobs against the real restic (AC-RP-01, FR-RP-02).
//!
//! The test runs the real restic 0.19.1 against a local-path repository if a binary is available on
//! the runner. It initializes the repository (test_access exit code 10, then a successful init) and
//! then tests the access (exit code 0). A missing restic skips the test, unless
//! `RESTICHUB_REQUIRE_RESTIC` asks it to fail.

#![allow(clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use uuid::Uuid;

/// AC-RP-01: the real restic initializes a local repository, and a later test-access succeeds.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rp_01_the_real_restic_initializes_a_local_repository() {
    let Some(restic) = common::real_restic_or_skip() else {
        return;
    };
    let Some(fixture) = common::stream_fixture("agent-real-restic").await else {
        return;
    };
    let dir = common::temp_config_dir("agent-real-restic");
    let identity = Arc::new(common::register(&fixture, &dir).await);
    let agent = Uuid::parse_str(&identity.agent_id).expect("the agent id");
    let app = common::api_router(fixture.server.state.clone());
    let admin =
        common::seed_session(fixture.db.database(), fixture.tenant, "jobs-admin", "admin").await;

    let repository_path = common::temp_config_dir("agent-real-restic-repo");
    let canary = format!("CANARY-{}", Uuid::now_v7());
    let repository_id =
        common::create_local_repository(&app, &admin, agent, &repository_path, &canary).await;

    let jobs = common::real_jobs(&dir, &restic);
    let (stop_tx, task) = common::run_agent(&dir, &identity, &jobs);
    assert!(
        common::wait_online(&fixture, agent).await,
        "the agent must be online"
    );

    // The repository does not exist: test_access exits with code 10 and init creates it.
    let job_id = common::start_action(&app, &admin, &repository_id, "init").await;
    let job = common::wait_job(&app, &admin, &job_id).await;
    assert_eq!(
        job.get("status").and_then(Value::as_str),
        Some("succeeded"),
        "{job}"
    );
    assert_eq!(
        job.get("result_code").and_then(Value::as_str),
        Some("succeeded")
    );
    assert_eq!(
        common::repository_view(&app, &admin, &repository_id)
            .await
            .get("initialized")
            .and_then(Value::as_bool),
        Some(true),
        "a successful init must initialize the repository"
    );

    // A later test-access finds the repository and succeeds.
    let job_id = common::start_action(&app, &admin, &repository_id, "test-access").await;
    let job = common::wait_job(&app, &admin, &job_id).await;
    assert_eq!(
        job.get("status").and_then(Value::as_str),
        Some("succeeded"),
        "{job}"
    );
    assert_eq!(
        job.get("result_code").and_then(Value::as_str),
        Some("succeeded")
    );
    // No secret reaches the journal.
    let journal = std::fs::read(dir.join(restichub_agent::journal::JOURNAL_FILE))
        .expect("the journal must exist");
    assert!(!String::from_utf8_lossy(&journal).contains(&canary));

    stop_tx.send(true).expect("cannot stop the agent");
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&repository_path);
    fixture.db.cleanup().await;
}
