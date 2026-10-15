//! Helpers of the Restore tests: a real agent job runner behind the fake stream of the control
//! plane, an initialized repository, and the restic binaries (FR-RS-01 to FR-RS-04).

#![allow(dead_code)]
#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::http::StatusCode;
use prost::Message as _;
use restichub_agent::jobs::Jobs;
use restichub_agent::restore_roots::RestoreAccess;
use restichub_db::AgentId;
use restichub_proto::agent::v1::{AgentMessage, agent_message};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::queue_support::{Env, FakeAgent, decode_job_id, eventually};

/// The `fake-restic` example of the agent crate. A run of this package alone needs
/// `cargo build -p restichub-agent --example fake-restic` first.
pub fn fake_restic() -> PathBuf {
    let exe = std::env::current_exe().expect("cannot find the test binary");
    let dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("the test binary holds a target directory");
    let program = dir.join("examples").join("fake-restic");
    assert!(
        program.exists(),
        "build the fake-restic example of the agent crate first: {}",
        program.display()
    );
    program
}

pub fn sha256_hex(path: &Path) -> String {
    use std::fmt::Write as _;

    let bytes = std::fs::read(path).expect("cannot read the file");
    let mut text = String::new();
    for byte in Sha256::digest(&bytes) {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// The real restic of the machine, or `None` with a message. `RESTICHUB_REQUIRE_RESTIC` makes a
/// missing binary a failure.
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
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|dir| dir.join(name))
                    .find(|path| path.is_file())
            })
        });
    if candidate.is_none() {
        assert!(
            std::env::var_os("RESTICHUB_REQUIRE_RESTIC").is_none(),
            "RESTICHUB_REQUIRE_RESTIC is set but no restic binary was found"
        );
        println!("skipping: no real restic binary is available (set RESTICHUB_TEST_RESTIC)");
    }
    candidate
}

pub fn work_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("restichub-cp-restore-{name}-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&dir).expect("cannot create the work directory");
    dir
}

/// Creates a local repository with `agent` as maintenance agent and marks it initialized.
pub async fn initialized_repository(
    env: &Env,
    agent: AgentId,
    password: &str,
    location: &Path,
) -> Uuid {
    let repository = env
        .create_repository_at(agent, password, location.to_str().expect("a UTF-8 path"))
        .await;
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE repositories SET initialized = true WHERE id = $1")
        .bind(repository)
        .execute(&mut *tx)
        .await
        .expect("cannot mark the repository initialized");
    tx.commit().await.expect("cannot commit the mark");
    repository
}

/// A real job runner of the agent crate, behind the fake stream of the control plane.
pub struct RealAgent {
    pub jobs: Jobs,
    pub dir: PathBuf,
    rx: tokio::sync::mpsc::Receiver<AgentMessage>,
    /// The bytes that the agent sent to the control plane (AC-RS-03).
    pub sent_bytes: usize,
}

impl RealAgent {
    pub fn new(name: &str, program: &Path) -> Self {
        let dir = work_dir(name);
        let jobs = Jobs::open(
            program.to_path_buf(),
            sha256_hex(program),
            dir.join("cache"),
            &dir,
        )
        .expect("the job journal must open")
        .with_restore_access(RestoreAccess::new(
            vec![
                std::env::temp_dir()
                    .canonicalize()
                    .expect("cannot resolve the temporary directory"),
            ],
            Vec::new(),
        ));
        let (tx, rx) = tokio::sync::mpsc::channel(256);
        jobs.attach(&tx);
        Self {
            jobs,
            dir,
            rx,
            sent_bytes: 0,
        }
    }

    /// Takes the next signed job of the stream, runs it and sends every message of the runner to
    /// the control plane. Returns the job identifier.
    pub async fn run_next(&mut self, agent: &mut FakeAgent) -> String {
        let signed = agent.job().await;
        let job_id = decode_job_id(&signed);
        let ring = Env::key_ring(&agent.identity);
        self.jobs
            .handle(
                signed,
                &ring,
                &agent.identity.agent_id,
                &agent.identity.tenant_id,
            )
            .await;
        loop {
            let message = tokio::time::timeout(Duration::from_secs(120), self.rx.recv())
                .await
                .expect("the real agent must report")
                .expect("the channel stays open");
            self.sent_bytes += message.encoded_len();
            let finished = matches!(message.payload, Some(agent_message::Payload::JobResult(_)));
            agent
                .tx
                .send(AgentMessage {
                    payload: message.payload,
                })
                .await
                .expect("cannot send the message");
            if finished {
                return job_id;
            }
        }
    }
}

/// Waits until the job shows a final status and returns its detail.
pub async fn finished_job(env: &Env, job: &str) -> Value {
    let id = Uuid::parse_str(job).expect("a job id");
    eventually(|| async {
        !matches!(
            env.job_status(id).await.as_str(),
            "queued" | "sent" | "running"
        )
    })
    .await;
    env.job(id).await
}

/// Reads the diffs of the audit records of one action.
pub async fn audit_rows(env: &Env, action: &str) -> Vec<String> {
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

pub fn snapshots_uri(repository: Uuid) -> String {
    format!("/api/v1/repositories/{repository}/snapshots")
}

pub fn ls_uri(repository: Uuid) -> String {
    format!("/api/v1/repositories/{repository}/ls")
}

pub fn restore_uri(repository: Uuid) -> String {
    format!("/api/v1/repositories/{repository}/restore")
}

/// The body of a restore request that restores a whole snapshot.
pub fn restore_body(snapshot: &str, target: &Path, extra: Value) -> Value {
    let mut body = json!({
        "snapshot_id": snapshot,
        "target": target.to_str().expect("a UTF-8 path"),
        "overwrite": "always",
    });
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            body.insert(key.clone(), value.clone());
        }
    }
    body
}

pub fn assert_status(status: StatusCode, expected: StatusCode, body: &Value) {
    assert_eq!(status, expected, "{body}");
}
