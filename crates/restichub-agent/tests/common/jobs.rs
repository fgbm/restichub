//! Job helpers for the agent tests.
//!
//! The tests enroll the real agent, run its connect loop with a job runner, and drive the web API.
//! The restic double is the `fake-restic` example, whose hash the test computes, so the agent's hash
//! check accepts it as in production (ADR-0031).

#![allow(dead_code)]
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use restichub_agent::cli::RegisterOptions;
use restichub_agent::commands;
use restichub_agent::identity::StoredIdentity;
use restichub_agent::jobs::Jobs;
use restichub_agent::keys::KeyState;
use restichub_agent::restic::Restic;
use restichub_agent::stream::{self, Backoff, StreamOptions};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::StreamFixture;

/// A restic double used only for the registration; the jobs use the scripted binary.
pub struct FakeRestic;

impl Restic for FakeRestic {
    fn verify(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn version(&self) -> anyhow::Result<String> {
        Ok("0.19.1".to_owned())
    }
}

/// The path of the `fake-restic` example next to the test binary.
#[must_use]
pub fn fake_restic() -> PathBuf {
    let exe = std::env::current_exe().expect("cannot find the test binary");
    let dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("the test binary holds a target directory");
    let name = if cfg!(windows) {
        "fake-restic.exe"
    } else {
        "fake-restic"
    };
    dir.join("examples").join(name)
}

#[must_use]
pub fn sha256_hex(path: &Path) -> String {
    use std::fmt::Write as _;
    let bytes = std::fs::read(path).expect("cannot read the binary");
    let digest = Sha256::digest(&bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Enrolls the real agent against the listener and returns the stored identity.
pub async fn register(fixture: &StreamFixture, dir: &Path) -> StoredIdentity {
    let ca_file = super::write_temp_file("jobs-ca.pem", fixture.server.root_pem().as_bytes());
    let options = RegisterOptions {
        url: Some(fixture.server.endpoint.clone()),
        token: Some(fixture.token.clone()),
        ca_file: Some(ca_file),
        ..RegisterOptions::default()
    };
    commands::register_async(&options, dir, &FakeRestic)
        .await
        .expect("the agent must register");
    StoredIdentity::load(dir).expect("the identity must load")
}

/// A fast backoff with a fixed jitter, so a test is deterministic (FR-ID-08).
#[must_use]
pub fn options() -> StreamOptions {
    StreamOptions {
        backoff: Backoff {
            base: Duration::from_millis(50),
            max: Duration::from_millis(400),
            fraction: Some(0.5),
        },
        timeouts: stream::Timeouts::default(),
    }
}

/// A job runner that runs the scripted `fake-restic` example.
#[must_use]
pub fn program_jobs(dir: &Path) -> Jobs {
    let program = fake_restic();
    assert!(
        program.exists(),
        "the fake-restic example must be built: {}",
        program.display()
    );
    let expected = sha256_hex(&program);
    Jobs::open(program, expected, dir.join("cache"), dir).expect("the job journal must open")
}

/// A job runner with a short grace period, so a test reaches the forced kill quickly (FR-AG-10).
#[must_use]
pub fn program_jobs_with_grace(dir: &Path, grace: Duration) -> Jobs {
    program_jobs(dir).with_stop_grace(grace)
}

/// A job runner that never runs a job: the stream tests deliver no job.
///
/// The binary path does not exist, so a job that arrived would fail to start rather than run.
#[must_use]
pub fn unused_jobs(dir: &Path) -> Jobs {
    Jobs::open(
        PathBuf::from("/nonexistent/restichub-agent/restic"),
        String::new(),
        dir.join("cache"),
        dir,
    )
    .expect("the job journal must open")
}

/// Executes the steps with a real restic binary.
pub fn real_jobs(dir: &Path, program: &Path) -> Jobs {
    let expected = sha256_hex(program);
    Jobs::open(program.to_path_buf(), expected, dir.join("cache"), dir)
        .expect("the job journal must open")
}

/// Runs the connect loop of the real agent until the test stops it.
pub fn run_agent(
    dir: &Path,
    identity: &Arc<StoredIdentity>,
    jobs: &Jobs,
) -> (watch::Sender<bool>, JoinHandle<anyhow::Result<()>>) {
    let identity = Arc::clone(identity);
    let dir = dir.to_path_buf();
    let jobs = jobs.clone();
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        let mut keys = KeyState::load(&identity, &dir).expect("the key state must load");
        stream::run(
            &identity,
            &dir,
            &mut keys,
            "0.19.1",
            &jobs,
            options(),
            &mut stop_rx,
        )
        .await
    });
    (stop_tx, task)
}

/// Polls the agent registry until the agent appears (FR-ID-07).
pub async fn wait_online(fixture: &StreamFixture, agent: Uuid) -> bool {
    let registry = fixture.server.state.agent_registry();
    for _ in 0..200 {
        if registry.is_online(restichub_db::AgentId::from_uuid(agent)) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Creates a local repository with the maintenance agent and returns its identifier.
pub async fn create_local_repository(
    app: &axum::Router,
    cookie: &str,
    agent: Uuid,
    path: &Path,
    password: &str,
) -> String {
    let body = json!({
        "name": format!("repo-{}", Uuid::now_v7()),
        "backend": {"kind": "local", "path": path.to_string_lossy()},
        "password": password,
        "maintenance_agent_id": agent.to_string(),
    });
    let (status, _, repository) = super::send(
        app,
        super::request("POST", "/api/v1/repositories", Some(cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{repository}");
    repository
        .get("id")
        .and_then(Value::as_str)
        .expect("the repository id")
        .to_owned()
}

/// Starts an init or a test-access job and returns its identifier.
pub async fn start_action(
    app: &axum::Router,
    cookie: &str,
    repository_id: &str,
    action: &str,
) -> String {
    let (status, _, job) = super::send(
        app,
        super::request(
            "POST",
            &format!("/api/v1/repositories/{repository_id}/{action}"),
            Some(cookie),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    job.get("id")
        .and_then(Value::as_str)
        .expect("the job id")
        .to_owned()
}

/// Waits until the fake restic has recorded a run whose arguments hold `needle`.
pub async fn wait_for_run(path: &Path, needle: &str) -> bool {
    for _ in 0..400 {
        if recorded_arguments(path).contains(needle) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

/// Waits for the job to reach a terminal state and returns its view.
pub async fn wait_job(app: &axum::Router, cookie: &str, job_id: &str) -> Value {
    for _ in 0..400 {
        let (status, _, job) = super::send(
            app,
            super::request("GET", &format!("/api/v1/jobs/{job_id}"), Some(cookie), None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{job}");
        let state = job.get("status").and_then(Value::as_str).unwrap_or("");
        if matches!(state, "succeeded" | "failed" | "expired") {
            return job;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the job must finish");
}

pub async fn repository_view(app: &axum::Router, cookie: &str, repository_id: &str) -> Value {
    let (status, _, body) = super::send(
        app,
        super::request(
            "GET",
            &format!("/api/v1/repositories/{repository_id}"),
            Some(cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Writes the exit codes of the fake restic next to the repository path.
pub fn script_exit_codes(path: &Path, test_access: i32, init: i32) {
    std::fs::write(
        format!("{}.restichub-fake", path.display()),
        format!("{test_access} {init}"),
    )
    .expect("cannot write the control file");
}

/// Sets the exit code of one fake restic command, such as `backup`, and wins over the scripted codes.
pub fn script_code(path: &Path, command: &str, code: i32) {
    std::fs::write(
        format!("{}.restichub-code-{command}", path.display()),
        code.to_string(),
    )
    .expect("cannot write the code file");
}

/// Scripts the standard output of one fake restic command.
pub fn script_stdout(path: &Path, command: &str, text: &str) {
    std::fs::write(format!("{}.restichub-out-{command}", path.display()), text)
        .expect("cannot write the output file");
}

/// Scripts the standard error of one fake restic command.
pub fn script_stderr(path: &Path, command: &str, text: &str) {
    std::fs::write(format!("{}.restichub-err-{command}", path.display()), text)
        .expect("cannot write the error file");
}

/// The priority lines that the fake restic recorded, one run on each line.
#[must_use]
pub fn recorded_priority(path: &Path) -> String {
    std::fs::read_to_string(format!("{}.restichub-state", path.display())).unwrap_or_default()
}

/// Makes the next fake-restic runs wait this long before they exit.
pub fn script_sleep(path: &Path, millis: u64) {
    std::fs::write(
        format!("{}.restichub-sleep", path.display()),
        millis.to_string(),
    )
    .expect("cannot write the sleep file");
}

/// Makes the fake restic ignore `SIGINT`, so a cancel covers the forced kill (FR-AG-10).
pub fn script_ignore_interrupt(path: &Path) {
    std::fs::write(format!("{}.restichub-ignore", path.display()), "1")
        .expect("cannot write the ignore file");
}

/// Sends a `CancelJob` to the open stream of an agent, as the control plane does (FR-AG-10).
pub fn cancel_job(fixture: &StreamFixture, agent: Uuid, job_id: &str) -> bool {
    use restichub_proto::agent::v1::{CancelJob, ControlMessage, control_message};
    fixture.server.state.agent_registry().send_to_agent(
        restichub_db::AgentId::from_uuid(agent),
        ControlMessage {
            payload: Some(control_message::Payload::CancelJob(CancelJob {
                job_id: job_id.to_owned(),
                grace_period_seconds: 0,
            })),
        },
    )
}

/// The arguments recorded by the fake restic, one run on each line.
#[must_use]
pub fn recorded_arguments(path: &Path) -> String {
    std::fs::read_to_string(format!("{}.restichub-argv", path.display())).unwrap_or_default()
}

/// Searches the process `PATH` for an executable.
#[must_use]
pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Finds a usable real restic, or returns `None` and prints why (the test then skips).
///
/// A missing restic is a skip, unless `RESTICHUB_REQUIRE_RESTIC` asks the test to fail.
#[must_use]
pub fn real_restic_or_skip() -> Option<PathBuf> {
    let candidate = std::env::var_os("RESTICHUB_TEST_RESTIC")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| {
            let name = if cfg!(windows) {
                "restic.exe"
            } else {
                "restic"
            };
            find_in_path(name)
        });
    let Some(candidate) = candidate else {
        if std::env::var_os("RESTICHUB_REQUIRE_RESTIC").is_some() {
            panic!("RESTICHUB_REQUIRE_RESTIC is set but no restic binary was found");
        }
        println!(
            "skipping: no real restic binary is available (set RESTICHUB_TEST_RESTIC to run it)"
        );
        return None;
    };
    let version = std::process::Command::new(&candidate)
        .arg("version")
        .output();
    match version {
        Ok(output) if output.status.success() => Some(candidate),
        _ => {
            if std::env::var_os("RESTICHUB_REQUIRE_RESTIC").is_some() {
                panic!("the restic binary at {} does not run", candidate.display());
            }
            println!(
                "skipping: the restic binary at {} does not run",
                candidate.display()
            );
            None
        }
    }
}
