//! The limits of a running step against a scripted restic: the stall watchdog and the optional
//! maximum duration (ADR-0039, FR-AG-05, FR-AG-10).

#![allow(clippy::expect_used)]

mod common;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ed25519_dalek::SigningKey;
use restichub_proto::agent::v1::{
    AgentMessage, BackupStep, Job, JobResult, JobStatus, LocalCredentials, RepositoryAccess, Step,
    agent_message, repository_access::BackendCredentials, step,
};
use restichub_proto::signing::{KeyRing, sign_job};
use tokio::sync::mpsc;

const KEY_ID: &str = "job-key-1";
const AGENT: &str = "agent-1";
const TENANT: &str = "tenant-1";

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7_u8; 32])
}

fn ring() -> KeyRing {
    let mut ring = KeyRing::new();
    ring.insert(KEY_ID, signing_key().verifying_key())
        .expect("insert");
    ring
}

fn at(offset_seconds: i64) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: time::OffsetDateTime::now_utc().unix_timestamp() + offset_seconds,
        nanos: 0,
    }
}

fn directory(name: &str) -> PathBuf {
    let path = common::temp_config_dir(name);
    std::fs::create_dir_all(&path).expect("cannot create the directory");
    path
}

fn backup_job(job_id: &str, repository: &Path, stall_seconds: u32, max_seconds: u32) -> Job {
    Job {
        job_id: job_id.to_owned(),
        tenant_id: TENANT.to_owned(),
        agent_id: AGENT.to_owned(),
        issued_at: Some(at(-60)),
        expires_at: Some(at(600)),
        timeout_seconds: max_seconds,
        stall_timeout_seconds: stall_seconds,
        repositories: vec![RepositoryAccess {
            repository_id: "repo-1".to_owned(),
            location: repository.to_string_lossy().into_owned(),
            password: "topsecret".to_owned(),
            backend_credentials: Some(BackendCredentials::Local(LocalCredentials {})),
        }],
        steps: vec![Step {
            step_id: "backup".to_owned(),
            kind: Some(step::Kind::Backup(BackupStep {
                repository_id: "repo-1".to_owned(),
                paths: vec!["/data".to_owned()],
                retry_lock: true,
                ..BackupStep::default()
            })),
            ..Step::default()
        }],
    }
}

/// The text of a restic backup that reports `count` status lines, `pause_ms` apart. With
/// `advance` false every line repeats the first one.
fn status_script(count: u64, pause_ms: u64, advance: bool) -> String {
    let mut text = String::new();
    for index in 0..count {
        let (files, bytes) = if advance {
            (index, index * 1000)
        } else {
            (0, 0)
        };
        writeln!(
            text,
            r#"{{"message_type":"status","percent_done":0.1,"total_files":10,"files_done":{files},"total_bytes":100000,"bytes_done":{bytes},"current_files":["/data/file"]}}"#
        )
        .expect("write to a string");
        writeln!(text, "#sleep {pause_ms}").expect("write to a string");
    }
    text
}

async fn run(job: &Job, repository: &Path, name: &str) -> (JobResult, Duration) {
    let dir = directory(name);
    let jobs = common::program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel::<AgentMessage>(256);
    jobs.attach(&tx);
    let started = Instant::now();
    jobs.handle(
        sign_job(job, KEY_ID, &signing_key()),
        &ring(),
        AGENT,
        TENANT,
    )
    .await;
    let result = loop {
        let message = tokio::time::timeout(Duration::from_secs(30), rx.recv())
            .await
            .expect("the agent must answer")
            .expect("the channel must stay open");
        if let Some(agent_message::Payload::JobResult(result)) = message.payload {
            break result;
        }
    };
    let elapsed = started.elapsed();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(repository);
    (result, elapsed)
}

fn error_code(result: &JobResult) -> Option<&str> {
    result.error.as_ref().map(|error| error.code.as_str())
}

/// ADR-0039: a step whose status never changes is stopped with the reason `stalled`.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_12_a_stalled_step_is_stopped_with_the_reason_stalled() {
    let path = directory("limits-stalled");
    common::script_stdout(&path, "backup", &status_script(40, 500, false));
    let job = backup_job("job-stalled", &path, 2, 0);

    let (result, elapsed) = run(&job, &path, "limits-stalled-dir").await;
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert_eq!(error_code(&result), Some("stalled"));
    assert!(
        elapsed < Duration::from_secs(12),
        "the watchdog must stop the step, took {elapsed:?}"
    );
}

/// ADR-0039: a step that makes progress for longer than the stall timeout is not stopped, and the
/// job has no maximum duration.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_12_a_step_that_makes_progress_is_not_stopped() {
    let path = directory("limits-progress");
    common::script_stdout(&path, "backup", &status_script(10, 600, true));
    let job = backup_job("job-progress", &path, 2, 0);

    let (result, elapsed) = run(&job, &path, "limits-progress-dir").await;
    assert_eq!(result.outcome, JobStatus::Succeeded as i32);
    assert_eq!(error_code(&result), None);
    assert!(
        elapsed > Duration::from_secs(2),
        "the run must outlast the stall timeout, took {elapsed:?}"
    );
}

/// ADR-0039: the optional maximum duration stops a step that still makes progress.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_13_the_maximum_duration_stops_a_step_that_makes_progress() {
    let path = directory("limits-max");
    common::script_stdout(&path, "backup", &status_script(60, 250, true));
    let job = backup_job("job-max", &path, 3600, 2);

    let (result, elapsed) = run(&job, &path, "limits-max-dir").await;
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert_eq!(error_code(&result), Some("max_duration_exceeded"));
    assert!(
        elapsed < Duration::from_secs(10),
        "the maximum duration must stop the step, took {elapsed:?}"
    );
}
