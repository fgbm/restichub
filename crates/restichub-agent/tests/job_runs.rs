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
    RepositoryAccess, Step, StepResult, StepStatus, TestAccessStep, agent_message,
    repository_access::BackendCredentials, step,
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
    jobs.greeted(false);
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
    jobs.greeted(false);
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
    jobs.greeted(false);
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
    jobs.greeted(false);
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
    jobs.greeted(false);
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
    jobs.greeted(false);
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
    jobs.greeted(false);
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

/// AC-SC-06: restic killed by a signal from outside leaves a possible stale lock, so the control
/// plane can schedule the `unlock`.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_sc_06_a_restic_killed_from_outside_reports_a_possible_stale_lock() {
    let dir = config_dir("job-runs-outside-kill");
    let path = repository("job-runs-outside-kill-repo");
    common::script_exit_codes(&path, 0, 0);
    common::script_sleep(&path, 200);
    common::script_kill(&path);

    let jobs = common::program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    jobs.greeted(false);
    let job = job(
        "job-outside-kill",
        vec![access("repo-1", &path)],
        vec![test_access_step("repo-1")],
    );
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;

    let (steps, result) = collect_job(&mut rx).await;
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status, StepStatus::Failed as i32);
    assert!(
        steps[0].possible_stale_lock,
        "a signal exit reports a possible stale lock"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// Runs one `test_access` step that ends with `code` and returns its `possible_stale_lock` flag.
async fn stale_flag_for_exit(name: &str, code: i32) -> bool {
    let dir = config_dir(name);
    let path = repository(&format!("{name}-repo"));
    common::script_exit_codes(&path, code, 0);
    let jobs = common::program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    jobs.greeted(false);
    let job = job(
        name,
        vec![access("repo-1", &path)],
        vec![test_access_step("repo-1")],
    );
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    let (steps, _) = collect_job(&mut rx).await;
    let flag = steps[0].possible_stale_lock;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
    flag
}

/// AC-SC-06: exit code 2 is a panic of the Go runtime, which leaves the lock in place.
#[tokio::test(flavor = "multi_thread")]
async fn ac_sc_06_a_go_panic_exit_code_reports_a_possible_stale_lock() {
    assert!(stale_flag_for_exit("job-runs-panic-exit", 2).await);
}

/// AC-SC-06: Windows has no signal, so a restic that an outside tool terminates ends with the code
/// of that tool. On Linux, code 1 is an ordinary failure of restic.
#[tokio::test(flavor = "multi_thread")]
async fn ac_sc_06_an_unknown_exit_code_marks_a_stale_lock_on_windows_only() {
    assert_eq!(
        stale_flag_for_exit("job-runs-unknown-exit", 1).await,
        cfg!(windows)
    );
}

/// Reads the messages until the job result arrives and returns the step results before it.
async fn collect_job(rx: &mut mpsc::Receiver<AgentMessage>) -> (Vec<StepResult>, JobResult) {
    let mut steps = Vec::new();
    loop {
        match next_message(rx).await.payload {
            Some(agent_message::Payload::StepResult(step)) => steps.push(step),
            Some(agent_message::Payload::JobResult(result)) => return (steps, result),
            _ => {}
        }
    }
}

/// Waits until the fake restic of `path` has started a run.
#[cfg(windows)]
async fn wait_for_start(path: &Path) {
    for _ in 0..400 {
        if runs(path) > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the fake restic must start");
}

/// FR-AG-05: a job whose deadline passes stops the job, and no later step starts.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_05_a_job_deadline_prevents_the_next_step() {
    let dir = config_dir("job-runs-deadline");
    let path_a = repository("job-runs-deadline-a");
    let path_b = repository("job-runs-deadline-b");
    common::script_exit_codes(&path_a, 0, 0);
    common::script_sleep(&path_a, 4_000);
    common::script_exit_codes(&path_b, 0, 0);

    let jobs = common::program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    jobs.greeted(false);
    let mut job = job(
        "job-deadline",
        vec![access("repo-a", &path_a), access("repo-b", &path_b)],
        vec![test_access_step("repo-a"), test_access_step("repo-b")],
    );
    job.timeout_seconds = 1;
    job.steps[0].timeout_seconds = 0;
    job.steps[1].timeout_seconds = 0;

    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    let (steps, result) = collect_job(&mut rx).await;
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert_eq!(steps.len(), 1, "the second step must not report a run");
    assert_eq!(steps[0].status, StepStatus::Failed as i32);
    assert_eq!(runs(&path_b), 0, "the second step must not start");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path_a);
    let _ = std::fs::remove_dir_all(&path_b);
}

/// FR-AG-10: a timeout that needs the forced kill reports a possible stale lock.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_10_a_timeout_with_a_forced_kill_reports_a_possible_stale_lock() {
    let dir = config_dir("job-runs-timeout-kill");
    let path = repository("job-runs-timeout-kill-repo");
    common::script_exit_codes(&path, 0, 0);
    common::script_sleep(&path, 20_000);
    common::script_ignore_interrupt(&path);

    let jobs = common::program_jobs_with_grace(&dir, Duration::from_millis(300));
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    jobs.greeted(false);
    let mut job = job(
        "job-timeout-kill",
        vec![access("repo-1", &path)],
        vec![test_access_step("repo-1")],
    );
    job.steps[0].timeout_seconds = 1;

    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    let (steps, result) = collect_job(&mut rx).await;
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert_eq!(steps.len(), 1);
    assert!(
        steps[0].possible_stale_lock,
        "a forced kill on a timeout may leave a lock"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// ADR-0020: a redelivery of a completed job returns the stored result, even at the cap.
#[tokio::test(flavor = "multi_thread")]
async fn adr_0020_a_completed_job_is_answered_before_the_cap() {
    let dir = config_dir("job-runs-completed-cap");
    let fast = repository("job-runs-completed-cap-fast");
    let slow = repository("job-runs-completed-cap-slow");
    common::script_exit_codes(&fast, 0, 0);
    common::script_exit_codes(&slow, 0, 0);
    common::script_sleep(&slow, 5_000);

    let jobs = common::program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel(64);
    jobs.attach(&tx);
    jobs.greeted(false);
    let done = signed(&job(
        "job-done",
        vec![access("repo-fast", &fast)],
        vec![test_access_step("repo-fast")],
    ));
    jobs.handle(done.clone(), &ring(), AGENT, TENANT).await;
    assert_eq!(wait_result(&mut rx).await.job_id, "job-done");

    let mut ids = Vec::new();
    for index in 0..4 {
        let job_id = format!("job-slow-{index}");
        ids.push(job_id.clone());
        let slow_job = job(
            &job_id,
            vec![access("repo-slow", &slow)],
            vec![test_access_step("repo-slow")],
        );
        jobs.handle(signed(&slow_job), &ring(), AGENT, TENANT).await;
    }
    jobs.handle(done, &ring(), AGENT, TENANT).await;

    let answer = loop {
        match next_message(&mut rx).await.payload {
            Some(payload @ agent_message::Payload::JobResult(_)) => break payload,
            Some(payload @ agent_message::Payload::JobRejected(_)) => break payload,
            _ => {}
        }
    };
    match answer {
        agent_message::Payload::JobResult(result) => assert_eq!(result.job_id, "job-done"),
        other => panic!("the stored result must answer, got {other:?}"),
    }
    assert_eq!(runs(&fast), 1, "the completed job must not run again");

    for job_id in ids {
        jobs.cancel(&job_id);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&fast);
    let _ = std::fs::remove_dir_all(&slow);
}

/// ADR-0020, FR-AG-12: a stored result goes out at every connection until its entry expires.
#[tokio::test(flavor = "multi_thread")]
async fn adr_0020_a_stored_result_is_sent_at_every_connection() {
    let dir = config_dir("job-runs-resend");
    let path = repository("job-runs-resend-repo");
    common::script_exit_codes(&path, 0, 0);

    let jobs = common::program_jobs(&dir);
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    jobs.greeted(false);
    let job = job(
        "job-resend",
        vec![access("repo-1", &path)],
        vec![test_access_step("repo-1")],
    );
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    assert_eq!(wait_result(&mut rx).await.job_id, "job-resend");

    for _ in 0..2 {
        let (next_tx, mut next_rx) = mpsc::channel(32);
        jobs.attach(&next_tx);
        jobs.greeted(false);
        jobs.send_stored_results().await;
        assert_eq!(wait_result(&mut next_rx).await.job_id, "job-resend");
    }
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// Cancels a long step and checks that the graceful `CTRL_BREAK` stops it without a forced kill.
#[cfg(windows)]
async fn cancel_with_ctrl_break(name: &str) {
    let dir = config_dir(name);
    let path = repository(&format!("{name}-repo"));
    common::script_exit_codes(&path, 0, 0);
    common::script_sleep(&path, 20_000);

    restichub_agent::exec::allocate_hidden_console().expect("cannot allocate the console");
    // The grace period is long, so only the graceful stop can end the step in time.
    let jobs = common::program_jobs_with_grace(&dir, Duration::from_secs(15));
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    jobs.greeted(false);
    let job = job(
        name,
        vec![access("repo-1", &path)],
        vec![test_access_step("repo-1")],
    );
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
    wait_for_start(&path).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let cancelled_at = std::time::Instant::now();
    jobs.cancel(name);
    let (steps, result) = collect_job(&mut rx).await;
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(10),
        "CTRL_BREAK must stop the step before the grace period ends"
    );
    assert_eq!(result.outcome, JobStatus::Cancelled as i32);
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status, StepStatus::Cancelled as i32);
    assert!(
        !steps[0].possible_stale_lock,
        "a graceful stop leaves no stale lock"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// AC-AG-06: a `CTRL_BREAK` cancels the step of the agent.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_06_ctrl_break_cancels_a_step() {
    cancel_with_ctrl_break("job-runs-ctrl-break").await;
}

/// The body of the test below. It runs in a process that has no console, as a service does.
#[cfg(windows)]
#[allow(unsafe_code)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_ag_06_cancel_without_console_inner() {
    use windows_sys::Win32::System::Console::GetConsoleProcessList;
    if std::env::var_os("RESTICHUB_TEST_DETACHED").is_none() {
        return;
    }
    let mut process = 0_u32;
    // SAFETY: the buffer holds one element, as the size argument says.
    let attached = unsafe { GetConsoleProcessList(&mut process, 1) };
    assert_eq!(attached, 0, "the test process must start without a console");
    cancel_with_ctrl_break("job-runs-ctrl-break-detached").await;
}

/// AC-AG-06: the agent cancels a step with `CTRL_BREAK` in a process without a console, as in a
/// service.
#[cfg(windows)]
#[test]
fn ac_ag_06_ctrl_break_cancels_a_step_without_a_console() {
    use std::os::windows::process::CommandExt as _;
    use windows_sys::Win32::System::Threading::DETACHED_PROCESS;
    let exe = std::env::current_exe().expect("cannot find the test executable");
    let status = std::process::Command::new(exe)
        .args([
            "--exact",
            "ac_ag_06_cancel_without_console_inner",
            "--nocapture",
        ])
        .env("RESTICHUB_TEST_DETACHED", "1")
        .creation_flags(DETACHED_PROCESS)
        .status()
        .expect("cannot start the detached test");
    assert!(status.success(), "the detached run failed: {status}");
}
