//! The agent-side execution of the repository init and test-access jobs against the real control
//! plane (FR-PR-03, FR-PR-06, FR-RP-02, AC-RP-01, AC-AG-04).
//!
//! The tests enroll the real agent over HTTPS, open the stream and drive the web API. A scripted
//! restic double (the `fake-restic` example) reports the exit codes that the test writes next to the
//! repository path. The tests cover the init chain of exit code 10 then 0, the `config_error` of
//! exit code 12, a test-access failure that runs no init, the typed `not_initialized` result, and
//! the proof that no secret reaches the arguments or the journal.

#![allow(clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use uuid::Uuid;

/// Runs an init or a test-access job end to end with the given exit codes and returns the job view,
/// the repository view and the recorded restic arguments.
async fn run(
    name: &str,
    test_access: i32,
    init: i32,
    action: &str,
) -> Option<(Value, Value, String, std::path::PathBuf)> {
    let fixture = common::stream_fixture(name).await?;
    let dir = common::temp_config_dir(name);
    let identity = Arc::new(common::register(&fixture, &dir).await);
    let agent = Uuid::parse_str(&identity.agent_id).expect("the agent id");
    let app = common::api_router(fixture.server.state.clone());
    let admin =
        common::seed_session(fixture.db.database(), fixture.tenant, "jobs-admin", "admin").await;

    let repository_path = common::temp_config_dir(&format!("{name}-repo"));
    let canary = format!("CANARY-{}", Uuid::now_v7());
    let repository_id =
        common::create_local_repository(&app, &admin, agent, &repository_path, &canary).await;
    common::script_exit_codes(&repository_path, test_access, init);

    let jobs = common::program_jobs(&dir);
    let (stop_tx, task) = common::run_agent(&dir, &identity, &jobs);
    assert!(
        common::wait_online(&fixture, agent).await,
        "the agent must be online"
    );

    let job_id = common::start_action(&app, &admin, &repository_id, action).await;
    let job = common::wait_job(&app, &admin, &job_id).await;
    let repository = common::repository_view(&app, &admin, &repository_id).await;

    stop_tx.send(true).expect("cannot stop the agent");
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;

    // No secret reached an argument or the journal (AC-AG-04).
    let arguments = common::recorded_arguments(&repository_path);
    assert!(!arguments.contains(&canary), "an argument holds the secret");
    let journal = std::fs::read(dir.join(restichub_agent::journal::JOURNAL_FILE))
        .expect("the journal must exist");
    assert!(
        !String::from_utf8_lossy(&journal).contains(&canary),
        "the journal holds the secret"
    );

    let _ = std::fs::remove_dir_all(&dir);
    fixture.db.cleanup().await;
    Some((job, repository, arguments, repository_path))
}

/// AC-RP-01: a test_access that finds no repository (exit code 10) runs init, which succeeds, and
/// the control plane sets `initialized`.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rp_01_the_agent_initializes_a_local_repository() {
    let Some((job, repository, arguments, path)) = run("agent-jobs-init", 10, 0, "init").await
    else {
        return;
    };
    assert_eq!(job.get("status").and_then(Value::as_str), Some("succeeded"));
    assert_eq!(
        job.get("result_code").and_then(Value::as_str),
        Some("succeeded")
    );
    assert_eq!(
        repository.get("initialized").and_then(Value::as_bool),
        Some(true),
        "a successful init must initialize the repository"
    );
    // The restic child ran test_access and then init.
    assert!(arguments.contains("cat config"), "{arguments}");
    assert!(arguments.contains("init"), "{arguments}");
    let _ = std::fs::remove_dir_all(&path);
}

/// AC-RP-01: a wrong password (restic exit code 12) gives `config_error` and runs no init.
#[tokio::test(flavor = "multi_thread")]
async fn ac_rp_01_a_wrong_password_gives_config_error_and_no_init() {
    let Some((job, repository, arguments, path)) = run("agent-jobs-config", 12, 0, "init").await
    else {
        return;
    };
    assert_eq!(job.get("status").and_then(Value::as_str), Some("failed"));
    assert_eq!(
        job.get("result_code").and_then(Value::as_str),
        Some("config_error")
    );
    assert_eq!(
        job.get("result_message").and_then(Value::as_str),
        Some("wrong_password")
    );
    assert_eq!(
        repository.get("initialized").and_then(Value::as_bool),
        Some(false)
    );
    assert!(
        !ran_init(&arguments),
        "init must not run after a wrong password: {arguments}"
    );
    let _ = std::fs::remove_dir_all(&path);
}

/// FR-RP-02: any other test_access failure (restic exit code 1) stops the job and runs no init.
#[tokio::test(flavor = "multi_thread")]
async fn fr_rp_02_a_test_access_failure_runs_no_init() {
    let Some((job, repository, arguments, path)) = run("agent-jobs-failed", 1, 0, "init").await
    else {
        return;
    };
    assert_eq!(job.get("status").and_then(Value::as_str), Some("failed"));
    assert_eq!(
        job.get("result_code").and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(
        repository.get("initialized").and_then(Value::as_bool),
        Some(false)
    );
    assert!(
        !ran_init(&arguments),
        "a failed test_access must not run init: {arguments}"
    );
    let _ = std::fs::remove_dir_all(&path);
}

/// FR-RP-02: a test-access on a missing repository gives the typed `not_initialized` result.
#[tokio::test(flavor = "multi_thread")]
async fn fr_rp_02_a_test_access_on_a_missing_repository_gives_not_initialized() {
    let Some((job, repository, _arguments, path)) =
        run("agent-jobs-test-access", 10, 0, "test-access").await
    else {
        return;
    };
    assert_eq!(
        job.get("result_code").and_then(Value::as_str),
        Some("not_initialized")
    );
    assert_eq!(
        repository.get("initialized").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        job.get("result_message").and_then(Value::as_str),
        Some("repository_missing")
    );
    let _ = std::fs::remove_dir_all(&path);
}

/// Reports whether the recorded runs hold an `init` step.
fn ran_init(arguments: &str) -> bool {
    arguments
        .lines()
        .any(|line| line.split_whitespace().next() == Some("init"))
}

/// ADR-0020, FR-AG-12: the stream drops while a step runs. The result must reach the control plane
/// after the agent reconnects, and the repository must be initialized.
#[tokio::test(flavor = "multi_thread")]
async fn adr_0020_a_result_after_a_dropped_stream_reaches_the_control_plane() {
    let Some(fixture) = common::stream_fixture("agent-jobs-reconnect").await else {
        return;
    };
    let dir = common::temp_config_dir("agent-jobs-reconnect");
    let identity = Arc::new(common::register(&fixture, &dir).await);
    let agent = Uuid::parse_str(&identity.agent_id).expect("the agent id");
    let app = common::api_router(fixture.server.state.clone());
    let admin = common::seed_session(
        fixture.db.database(),
        fixture.tenant,
        "jobs-reconnect",
        "admin",
    )
    .await;

    let repository_path = common::temp_config_dir("agent-jobs-reconnect-repo");
    let canary = format!("CANARY-{}", Uuid::now_v7());
    let repository_id =
        common::create_local_repository(&app, &admin, agent, &repository_path, &canary).await;
    common::script_exit_codes(&repository_path, 10, 0);
    // The step runs long enough that the stream drops and the agent reconnects before it ends.
    common::script_sleep(&repository_path, 2_000);

    let jobs = common::program_jobs(&dir);
    let (stop_tx, task) = common::run_agent(&dir, &identity, &jobs);
    assert!(
        common::wait_online(&fixture, agent).await,
        "the agent must be online"
    );

    let job_id = common::start_action(&app, &admin, &repository_id, "init").await;
    assert!(
        common::wait_for_run(&repository_path, "cat config").await,
        "test_access must start"
    );
    // Drop the stream while restic runs. The result is recorded after the reconnect, so only the
    // stream that is open then carries it.
    assert!(
        fixture
            .server
            .state
            .agent_registry()
            .close_by_agent(restichub_db::AgentId::from_uuid(agent)),
        "the stream must be open"
    );

    let job = common::wait_job(&app, &admin, &job_id).await;
    assert_eq!(
        job.get("status").and_then(Value::as_str),
        Some("succeeded"),
        "{job}"
    );
    assert_eq!(
        common::repository_view(&app, &admin, &repository_id)
            .await
            .get("initialized")
            .and_then(Value::as_bool),
        Some(true),
        "the re-sent result must initialize the repository"
    );

    stop_tx.send(true).expect("cannot stop the agent");
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&repository_path);
    fixture.db.cleanup().await;
}

/// FR-AG-10: a cancel while `test_access` runs stops the job, and `init` never runs.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_10_a_cancel_after_test_access_runs_no_init() {
    let Some(fixture) = common::stream_fixture("agent-jobs-cancel").await else {
        return;
    };
    let dir = common::temp_config_dir("agent-jobs-cancel");
    let identity = Arc::new(common::register(&fixture, &dir).await);
    let agent = Uuid::parse_str(&identity.agent_id).expect("the agent id");
    let app = common::api_router(fixture.server.state.clone());
    let admin = common::seed_session(
        fixture.db.database(),
        fixture.tenant,
        "jobs-cancel",
        "admin",
    )
    .await;

    let repository_path = common::temp_config_dir("agent-jobs-cancel-repo");
    let canary = format!("CANARY-{}", Uuid::now_v7());
    let repository_id =
        common::create_local_repository(&app, &admin, agent, &repository_path, &canary).await;
    common::script_exit_codes(&repository_path, 10, 0);
    // The step holds open, so the cancel arrives while test_access runs.
    common::script_sleep(&repository_path, 2_000);

    let jobs = common::program_jobs(&dir);
    let (stop_tx, task) = common::run_agent(&dir, &identity, &jobs);
    assert!(
        common::wait_online(&fixture, agent).await,
        "the agent must be online"
    );

    let job_id = common::start_action(&app, &admin, &repository_id, "init").await;
    assert!(
        common::wait_for_run(&repository_path, "cat config").await,
        "test_access must start"
    );
    assert!(
        common::cancel_job(&fixture, agent, &job_id),
        "the cancel must reach the agent"
    );

    let job = common::wait_job(&app, &admin, &job_id).await;
    assert_eq!(
        job.get("status").and_then(Value::as_str),
        Some("failed"),
        "{job}"
    );
    assert_eq!(
        job.get("result_code").and_then(Value::as_str),
        Some("cancelled"),
        "{job}"
    );
    let arguments = common::recorded_arguments(&repository_path);
    assert!(
        !ran_init(&arguments),
        "a cancelled job must not run init: {arguments}"
    );
    assert_eq!(
        common::repository_view(&app, &admin, &repository_id)
            .await
            .get("initialized")
            .and_then(Value::as_bool),
        Some(false),
        "a cancelled job must not initialize the repository"
    );

    stop_tx.send(true).expect("cannot stop the agent");
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&repository_path);
    fixture.db.cleanup().await;
}
