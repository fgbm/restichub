//! The limits of a running step against a scripted restic: the stall watchdog and the optional
//! maximum duration (ADR-0039, FR-AG-05, FR-AG-10).

#![allow(clippy::expect_used)]

mod common;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ed25519_dalek::SigningKey;
use restichub_proto::agent::v1::{
    AgentMessage, BackupStep, Job, JobResult, JobStatus, LocalCredentials, OverwriteMode,
    RepositoryAccess, RestoreStep, Step, agent_message, repository_access::BackendCredentials,
    step,
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

fn restore_job(
    job_id: &str,
    repository: &Path,
    target: &Path,
    verify: bool,
    stall_seconds: u32,
) -> Job {
    Job {
        steps: vec![Step {
            step_id: "restore".to_owned(),
            kind: Some(step::Kind::Restore(RestoreStep {
                repository_id: "repo-1".to_owned(),
                snapshot_id: "0123abcd".to_owned(),
                target: target.to_string_lossy().into_owned(),
                overwrite: OverwriteMode::Always as i32,
                verify,
                ..RestoreStep::default()
            })),
            ..Step::default()
        }],
        ..backup_job(job_id, repository, stall_seconds, 0)
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
    jobs.greeted(false);
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
    // The script would run for 20 seconds; the watchdog must end the step well before.
    let script_length = Duration::from_secs(20);
    let job = backup_job("job-stalled", &path, 2, 0);

    let (result, elapsed) = run(&job, &path, "limits-stalled-dir").await;
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert_eq!(error_code(&result), Some("stalled"));
    assert!(
        elapsed < script_length,
        "the watchdog must stop the step, took {elapsed:?}"
    );
}

/// ADR-0039, AC-AG-12: restic reports a refused backend on the standard error and retries it; a
/// retry line is not progress, so the stall timeout of the job still stops the step.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_12_a_retry_of_an_unreachable_backend_is_not_progress() {
    let path = directory("limits-retry");
    let mut retries = String::new();
    for attempt in 0..40 {
        writeln!(
            retries,
            "Save(<data>/e5/{attempt:04x}): dial tcp 192.0.2.10:8000: connect: connection refused"
        )
        .expect("write to a string");
        writeln!(retries, "#sleep 500").expect("write to a string");
    }
    common::script_stderr(&path, "backup", &retries);
    // The script would retry for 20 seconds; the watchdog must end the step well before.
    let script_length = Duration::from_secs(20);
    let job = backup_job("job-retry", &path, 2, 0);

    let (result, elapsed) = run(&job, &path, "limits-retry-dir").await;
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert_eq!(error_code(&result), Some("stalled"));
    assert!(
        elapsed < script_length,
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
    // The script would run for 15 seconds; the limit must end the step before.
    let script_length = Duration::from_secs(15);
    let job = backup_job("job-max", &path, 3600, 2);

    let (result, elapsed) = run(&job, &path, "limits-max-dir").await;
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert_eq!(error_code(&result), Some("max_duration_exceeded"));
    assert!(
        elapsed < script_length,
        "the maximum duration must stop the step, took {elapsed:?}"
    );
}

/// AC-AG-12, ADR-0039: a restore over a target that holds the same snapshot skips those files and
/// prints no restored byte. Only the skipped counters grow, and the step is not stalled.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_12_a_restore_that_only_skips_files_is_not_stalled() {
    let path = directory("limits-restore-skip");
    let target = directory("limits-restore-skip-target");
    let mut text = String::new();
    for index in 0..12 {
        writeln!(
            text,
            r#"{{"message_type":"status","percent_done":0,"total_files":1000000,"files_restored":0,"files_skipped":{},"total_bytes":8600000000,"bytes_restored":0,"bytes_skipped":{}}}"#,
            index * 1_900,
            index * 15_000_000
        )
        .expect("write to a string");
        writeln!(text, "#sleep 300").expect("write to a string");
    }
    writeln!(
        text,
        r#"{{"message_type":"summary","total_files":1000000,"files_restored":0,"files_skipped":22800,"total_bytes":8600000000,"bytes_restored":0,"bytes_skipped":180000000}}"#
    )
    .expect("write to a string");
    common::script_stdout(&path, "restore", &text);
    // The script runs for 3.6 seconds, longer than the stall timeout.
    let job = restore_job("job-restore-skip", &path, &target, false, 2);

    let (result, elapsed) = run(&job, &path, "limits-restore-skip-dir").await;
    assert_eq!(result.outcome, JobStatus::Succeeded as i32);
    assert_eq!(error_code(&result), None);
    assert!(
        elapsed > Duration::from_secs(2),
        "the run must outlast the stall timeout, took {elapsed:?}"
    );
}

/// AC-AG-12, ADR-0051: a restore with `--verify` prints nothing while it verifies its files, so
/// the agent gives that phase the long window of `check` and `prune` and does not stop it as
/// stalled while it verifies.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_12_a_restore_with_verify_is_not_stalled_while_it_verifies() {
    let path = directory("limits-restore-verify");
    let target = directory("limits-restore-verify-target");
    common::script_stdout(
        &path,
        "restore",
        concat!(
            r#"{"message_type":"status","percent_done":1,"total_files":10,"files_restored":10,"total_bytes":2000,"bytes_restored":2000}"#,
            "\n",
            r#"{"message_type":"summary","total_files":10,"files_restored":10,"total_bytes":2000,"bytes_restored":2000}"#,
            "\n#sleep 3000\n",
        ),
    );
    // The verification takes three seconds without a line, longer than the stall timeout.
    let job = restore_job("job-restore-verify", &path, &target, true, 2);

    let (result, elapsed) = run(&job, &path, "limits-restore-verify-dir").await;
    assert_eq!(result.outcome, JobStatus::Succeeded as i32);
    assert_eq!(error_code(&result), None);
    assert!(
        elapsed > Duration::from_secs(2),
        "the verification must outlast the stall timeout, took {elapsed:?}"
    );
}

/// FR-SC-05, ADR-0039: restic waits for a lock of another host without a status line. The agent
/// asks restic to end the wait before the stall timeout, so the step ends with the lock code and
/// the control plane can retry the job.
#[tokio::test(flavor = "multi_thread")]
async fn fr_sc_05_a_lock_wait_ends_with_the_lock_code_and_not_as_stalled() {
    let path = directory("limits-lock-wait");
    // The window is 3 seconds, so the test is short. restic prints its retry clock only to the
    // standard error, which is not progress, and it stops one second before the window closes.
    common::script_stdout(&path, "backup", "#sleep 2000\n");
    common::script_code(&path, "backup", 11);
    let job = backup_job("job-lock-wait", &path, 3, 0);

    let (result, elapsed) = run(&job, &path, "limits-lock-wait-dir").await;
    assert_eq!(result.outcome, JobStatus::RepositoryLocked as i32);
    assert_eq!(error_code(&result), Some("repository_locked"));
    assert!(
        elapsed > Duration::from_secs(1),
        "restic must end the wait itself, took {elapsed:?}"
    );
    // The wait of the command is the one of production, in minutes. It must fit in the shortest
    // stall timeout of a profile, 5 minutes, or the watchdog stops the step before restic gives
    // up and the control plane reads `stalled` instead of `repository_locked`.
    let arguments = common::recorded_arguments(&path);
    let minutes = arguments
        .split_whitespace()
        .find_map(|argument| argument.strip_prefix("--retry-lock="))
        .and_then(|value| value.strip_suffix('m'))
        .and_then(|value| value.parse::<u64>().ok())
        .expect("the agent must pass --retry-lock to restic");
    assert!(
        minutes * 60 < 5 * 60,
        "the lock wait of {minutes} minutes must be shorter than the shortest stall timeout of a profile"
    );
}
