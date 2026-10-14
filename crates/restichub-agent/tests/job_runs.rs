//! The agent-side job execution against a scripted restic, without a control plane (ADR-0020,
//! FR-AG-05, FR-AG-10, FR-RP-02).
//!
//! The tests drive [`Jobs::handle`] directly and read the protocol messages of the stream channel.
//! The scripted `fake-restic` example reports the exit codes and the timing that the test writes
//! next to the repository path.

#![allow(clippy::expect_used)]

mod common;

use std::path::Path;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use restichub_proto::agent::v1::{
    AgentMessage, InitStep, Job, JobResult, JobStatus, LocalCredentials, RejectReason,
    RepositoryAccess, Step, TestAccessStep, agent_message, repository_access::BackendCredentials,
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

fn now() -> prost_types::Timestamp {
    let now = time::OffsetDateTime::now_utc();
    prost_types::Timestamp {
        seconds: now.unix_timestamp(),
        nanos: now.nanosecond() as i32,
    }
}

fn at(offset_seconds: i64) -> prost_types::Timestamp {
    let now = now();
    prost_types::Timestamp {
        seconds: now.seconds + offset_seconds,
        nanos: 0,
    }
}

fn test_access_step(repository: &str) -> Step {
    Step {
        step_id: format!("test-access-{repository}"),
        kind: Some(step::Kind::TestAccess(TestAccessStep {
            repository_id: repository.to_owned(),
        })),
        ..Step::default()
    }
}

fn init_step(repository: &str) -> Step {
    Step {
        step_id: format!("init-{repository}"),
        kind: Some(step::Kind::Init(InitStep {
            repository_id: repository.to_owned(),
        })),
        ..Step::default()
    }
}

fn access(repository: &str, location: &Path) -> RepositoryAccess {
    RepositoryAccess {
        repository_id: repository.to_owned(),
        location: location.to_string_lossy().into_owned(),
        password: "topsecret".to_owned(),
        backend_credentials: Some(BackendCredentials::Local(LocalCredentials {})),
    }
}

/// A job with the given repositories and steps, valid for the current clock.
fn job(job_id: &str, repositories: Vec<RepositoryAccess>, steps: Vec<Step>) -> Job {
    Job {
        job_id: job_id.to_owned(),
        tenant_id: TENANT.to_owned(),
        agent_id: AGENT.to_owned(),
        issued_at: Some(at(-60)),
        expires_at: Some(at(600)),
        repositories,
        steps,
        ..Job::default()
    }
}

/// A temporary repository path whose parent directory exists, with the fake control files.
fn repository(name: &str) -> std::path::PathBuf {
    let path = common::temp_config_dir(name);
    std::fs::create_dir_all(&path).expect("cannot create the repository path");
    path
}

/// A temporary agent configuration directory that exists, for the journal.
fn config_dir(name: &str) -> std::path::PathBuf {
    let dir = common::temp_config_dir(name);
    std::fs::create_dir_all(&dir).expect("cannot create the configuration directory");
    dir
}

fn signed(job: &Job) -> restichub_proto::agent::v1::SignedJob {
    sign_job(job, KEY_ID, &signing_key())
}

/// Reads the next message of the stream, or panics after fifteen seconds.
async fn next_message(rx: &mut mpsc::Receiver<AgentMessage>) -> AgentMessage {
    tokio::time::timeout(Duration::from_secs(15), rx.recv())
        .await
        .expect("the agent must answer")
        .expect("the channel must stay open")
}

/// Reads the messages until the final job result arrives.
async fn wait_result(rx: &mut mpsc::Receiver<AgentMessage>) -> JobResult {
    loop {
        let message = next_message(rx).await;
        if let Some(agent_message::Payload::JobResult(result)) = message.payload {
            return result;
        }
    }
}

/// The number of runs that the fake restic recorded for `path`.
fn runs(path: &Path) -> usize {
    common::recorded_arguments(path)
        .lines()
        .filter(|line| !line.is_empty())
        .count()
}

/// ADR-0020, FR-PR-06: a running job that the control plane delivers again is acknowledged, not
/// started a second time.
#[tokio::test(flavor = "multi_thread")]
async fn adr_0020_a_running_job_is_not_started_twice() {
    let dir = config_dir("job-runs-twice");
    let path = repository("job-runs-twice-repo");
    common::script_exit_codes(&path, 0, 0);
    // The step runs long enough that the second delivery arrives while it runs.
    common::script_sleep(&path, 1_500);

    let jobs = common::program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    let job = job(
        "job-1",
        vec![access("repo-1", &path)],
        vec![test_access_step("repo-1")],
    );
    let signed = signed(&job);

    jobs.handle(signed.clone(), &ring(), AGENT, TENANT).await;
    jobs.handle(signed, &ring(), AGENT, TENANT).await;

    let result = wait_result(&mut rx).await;
    assert_eq!(result.job_id, "job-1");
    assert_eq!(result.outcome, JobStatus::Succeeded as i32);
    assert_eq!(runs(&path), 1, "the job must run once");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// FR-RP-02, AC-RP-01: an init step without a preceding test_access of the same repository fails
/// the job and runs no restic.
#[tokio::test(flavor = "multi_thread")]
async fn fr_rp_02_an_init_without_a_test_access_fails() {
    let dir = config_dir("job-runs-bare-init");
    let path = repository("job-runs-bare-init-repo");

    let jobs = common::program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    let job = job(
        "job-2",
        vec![access("repo-1", &path)],
        vec![init_step("repo-1")],
    );

    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    let result = wait_result(&mut rx).await;
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert_eq!(runs(&path), 0, "a bare init must run no restic");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// FR-RP-02, AC-RP-01: the probe result binds to the repository of the step, so an init of another
/// repository than the one test_access probed fails the job.
#[tokio::test(flavor = "multi_thread")]
async fn fr_rp_02_an_init_of_another_repository_fails() {
    let dir = config_dir("job-runs-other-repo");
    let path_a = repository("job-runs-other-a");
    let path_b = repository("job-runs-other-b");
    common::script_exit_codes(&path_a, 10, 0);
    common::script_exit_codes(&path_b, 10, 0);

    let jobs = common::program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    let job = job(
        "job-3",
        vec![access("repo-a", &path_a), access("repo-b", &path_b)],
        vec![test_access_step("repo-a"), init_step("repo-b")],
    );

    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    let result = wait_result(&mut rx).await;
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert_eq!(runs(&path_a), 1, "test_access of repo-a must run");
    assert_eq!(runs(&path_b), 0, "init of repo-b must not run");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path_a);
    let _ = std::fs::remove_dir_all(&path_b);
}

/// FR-AG-05: a step without a timeout uses the job timeout, so a job that sets one second stops
/// the step.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_05_a_step_without_a_timeout_honours_the_job_timeout() {
    let dir = config_dir("job-runs-timeout");
    let path = repository("job-runs-timeout-repo");
    common::script_exit_codes(&path, 10, 0);
    common::script_sleep(&path, 5_000);

    let jobs = common::program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    let mut job = job(
        "job-4",
        vec![access("repo-1", &path)],
        vec![test_access_step("repo-1")],
    );
    job.timeout_seconds = 1;
    job.steps[0].timeout_seconds = 0;

    let started = std::time::Instant::now();
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    let result = wait_result(&mut rx).await;
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "the job timeout must stop the step"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// ADR-0020: the agent refuses a job beyond the concurrency cap with `busy`.
#[tokio::test(flavor = "multi_thread")]
async fn adr_0020_a_job_beyond_the_concurrency_cap_is_rejected_busy() {
    let dir = config_dir("job-runs-cap");
    let path = repository("job-runs-cap-repo");
    common::script_exit_codes(&path, 0, 0);
    common::script_sleep(&path, 3_000);

    let jobs = common::program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    let mut ids = Vec::new();
    for index in 0..5 {
        let job_id = format!("job-{index}");
        ids.push(job_id.clone());
        let job = job(
            &job_id,
            vec![access("repo-1", &path)],
            vec![test_access_step("repo-1")],
        );
        jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    }

    // The first jobs run and the last one is refused with `busy`.
    let mut acks = 0;
    let mut rejected = None;
    for _ in 0..5 {
        match next_message(&mut rx).await.payload {
            Some(agent_message::Payload::JobAck(_)) => acks += 1,
            Some(agent_message::Payload::JobRejected(reason)) => rejected = Some(reason),
            _ => {}
        }
    }
    assert_eq!(acks, 4, "the cap allows four jobs");
    assert_eq!(
        rejected.and_then(|reason| RejectReason::try_from(reason.reason).ok()),
        Some(RejectReason::Busy)
    );

    for job_id in ids {
        jobs.cancel(&job_id);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// ADR-0020: the agent cannot record a new job, so it refuses it (fail closed) and runs no restic.
#[tokio::test(flavor = "multi_thread")]
async fn adr_0020_a_journal_write_failure_refuses_a_new_job() {
    let dir = config_dir("job-runs-persist");
    let path = repository("job-runs-persist-repo");
    let jobs = common::program_jobs(&dir);
    // Break the journal: a directory at the file path makes the atomic replace fail.
    let journal = dir.join(restichub_agent::journal::JOURNAL_FILE);
    let _ = std::fs::remove_file(&journal);
    std::fs::create_dir_all(&journal).expect("cannot create the blocking directory");

    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    let job = job(
        "job-5",
        vec![access("repo-1", &path)],
        vec![test_access_step("repo-1")],
    );

    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    let message = next_message(&mut rx).await;
    match message.payload {
        Some(agent_message::Payload::JobRejected(reason)) => assert_eq!(
            RejectReason::try_from(reason.reason).ok(),
            Some(RejectReason::Internal)
        ),
        other => panic!("the job must be refused, got {other:?}"),
    }
    assert_eq!(runs(&path), 0, "a refused job must run no restic");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// FR-AG-10, AC-AG-07: a step that ignores the graceful signal is killed as a tree, and the step
/// reports a possible stale lock.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_07_a_step_that_ignores_the_signal_is_killed_after_the_grace_period() {
    let dir = config_dir("job-runs-ignore");
    let path = repository("job-runs-ignore-repo");
    common::script_exit_codes(&path, 0, 0);
    common::script_sleep(&path, 5_000);
    common::script_ignore_interrupt(&path);

    let jobs = common::program_jobs_with_grace(&dir, Duration::from_millis(300));
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    let job = job(
        "job-6",
        vec![access("repo-1", &path)],
        vec![test_access_step("repo-1")],
    );
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    // Wait until restic runs, then cancel it.
    for _ in 0..200 {
        if runs(&path) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    jobs.cancel("job-6");

    let mut stale = false;
    let result = loop {
        let message = next_message(&mut rx).await;
        match message.payload {
            Some(agent_message::Payload::StepResult(step)) => {
                if step.possible_stale_lock {
                    stale = true;
                }
            }
            Some(agent_message::Payload::JobResult(result)) => break result,
            _ => {}
        }
    };
    assert_eq!(result.outcome, JobStatus::Cancelled as i32);
    assert!(stale, "a forced kill reports a possible stale lock");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}
