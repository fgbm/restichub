//! The agent ends a job whose task dies without a result (FR-AG-09, FR-AG-10, ADR-0020, ADR-0043).

#![allow(clippy::expect_used)]

mod common;

use std::time::Duration;

use ed25519_dalek::SigningKey;
use restichub_proto::agent::v1::{
    AgentMessage, Job, JobResult, JobStatus, LocalCredentials, RepositoryAccess, Step, StepResult,
    StepStatus, TestAccessStep, agent_message, repository_access::BackendCredentials, step,
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
    let now = time::OffsetDateTime::now_utc();
    prost_types::Timestamp {
        seconds: now.unix_timestamp() + offset_seconds,
        nanos: 0,
    }
}

fn test_access_job(job_id: &str, location: &std::path::Path) -> Job {
    Job {
        job_id: job_id.to_owned(),
        tenant_id: TENANT.to_owned(),
        agent_id: AGENT.to_owned(),
        issued_at: Some(at(-60)),
        expires_at: Some(at(600)),
        repositories: vec![RepositoryAccess {
            repository_id: "repo-1".to_owned(),
            location: location.to_string_lossy().into_owned(),
            password: "topsecret".to_owned(),
            backend_credentials: Some(BackendCredentials::Local(LocalCredentials {})),
        }],
        steps: vec![Step {
            step_id: "test-access-repo-1".to_owned(),
            kind: Some(step::Kind::TestAccess(TestAccessStep {
                repository_id: "repo-1".to_owned(),
            })),
            ..Step::default()
        }],
        ..Job::default()
    }
}

async fn wait_result(rx: &mut mpsc::Receiver<AgentMessage>) -> JobResult {
    wait_result_with_steps(rx).await.1
}

/// The step results that came before the job result, and the job result.
async fn wait_result_with_steps(
    rx: &mut mpsc::Receiver<AgentMessage>,
) -> (Vec<StepResult>, JobResult) {
    let mut steps = Vec::new();
    loop {
        let message = tokio::time::timeout(Duration::from_secs(15), rx.recv())
            .await
            .expect("the agent must answer")
            .expect("the channel must stay open");
        match message.payload {
            Some(agent_message::Payload::StepResult(step)) => steps.push(step),
            Some(agent_message::Payload::JobResult(result)) => return (steps, result),
            _ => {}
        }
    }
}

/// True when `/proc` still lists the process: a child that exited and that nobody waited for stays
/// there as a zombie. The wait covers a grandchild that init reaps.
#[cfg(target_os = "linux")]
async fn is_reaped(pid: &str) -> bool {
    for _ in 0..40 {
        if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// FR-AG-09: a job whose task panics while restic runs ends as failed with a result for the control
/// plane, its slot is free, and the agent stopped restic (FR-AG-10).
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_09_a_panic_in_the_job_task_fails_the_job_and_frees_the_slot() {
    let dir = common::temp_config_dir("task-loss");
    std::fs::create_dir_all(&dir).expect("cannot create the configuration directory");
    let path = common::temp_config_dir("task-loss-repo");
    std::fs::create_dir_all(&path).expect("cannot create the repository path");
    common::script_exit_codes(&path, 0, 0);
    let restic_runs_for = Duration::from_secs(3);
    common::script_sleep(&path, restic_runs_for.as_millis() as u64);

    let jobs = common::program_jobs(&dir).with_panic_after_spawn();
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    jobs.greeted(false);
    let job = test_access_job("job-panic", &path);
    jobs.handle(
        sign_job(&job, KEY_ID, &signing_key()),
        &ring(),
        AGENT,
        TENANT,
    )
    .await;

    let (steps, result) = wait_result_with_steps(&mut rx).await;
    assert_eq!(result.job_id, "job-panic");
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert_eq!(
        result.error.as_ref().map(|error| error.code.as_str()),
        Some("agent_internal_error")
    );
    assert_eq!(
        jobs.stop_running(Duration::from_secs(1)).await,
        0,
        "the job must leave the running set"
    );
    assert_eq!(
        jobs.held_job_ids(),
        vec!["job-panic".to_owned()],
        "the hello lists the stored result once, so the control plane stops the job"
    );

    // FR-AG-10, AC-AG-07: the forced kill of restic may leave its lock, and the control plane
    // learns it from the step result that comes before the job result.
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].step_id, "test-access-repo-1");
    assert!(
        steps[0].possible_stale_lock,
        "a restic killed by the agent may have left a lock"
    );
    assert_eq!(steps[0].status, StepStatus::Failed as i32);

    tokio::time::sleep(restic_runs_for + Duration::from_secs(1)).await;
    assert!(
        !std::path::Path::new(&format!("{}.restichub-finished", path.display())).exists(),
        "restic must not outlive the task of its job"
    );
    #[cfg(target_os = "linux")]
    {
        let pid = std::fs::read_to_string(format!("{}.restichub-pid", path.display()))
            .expect("the fake restic must write its pid");
        assert!(is_reaped(&pid).await, "the killed restic must be reaped");
    }
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// FR-AG-09, ADR-0020: a task that dies after it stored the result still delivers the stored
/// result, and the result is on the disk.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_09_a_task_that_dies_after_it_stored_the_result_still_delivers_it() {
    let dir = common::temp_config_dir("task-loss-stored");
    std::fs::create_dir_all(&dir).expect("cannot create the configuration directory");
    let path = common::temp_config_dir("task-loss-stored-repo");
    std::fs::create_dir_all(&path).expect("cannot create the repository path");
    common::script_exit_codes(&path, 0, 0);

    let jobs = common::program_jobs(&dir).with_panic_after_result();
    let (tx, mut rx) = mpsc::channel(32);
    jobs.attach(&tx);
    jobs.greeted(false);
    let job = test_access_job("job-stored", &path);
    jobs.handle(
        sign_job(&job, KEY_ID, &signing_key()),
        &ring(),
        AGENT,
        TENANT,
    )
    .await;

    let result = wait_result(&mut rx).await;
    assert_eq!(result.job_id, "job-stored");
    assert_eq!(
        result.outcome,
        JobStatus::Succeeded as i32,
        "the stored result stands, it is no internal error"
    );
    assert_eq!(jobs.stop_running(Duration::from_secs(1)).await, 0);
    assert_eq!(
        common::program_jobs(&dir).held_job_ids(),
        vec!["job-stored".to_owned()],
        "the result must be on the disk"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&path);
}

/// FR-AG-09, FR-AG-10: a panic in a script step fails the job and kills the script, and it marks
/// no stale lock, because a script holds none.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_09_a_panic_in_a_script_step_fails_the_job_without_a_stale_lock() {
    use common::scripts::{Host, simple};
    use restichub_proto::agent::v1::HookPhase;

    let host = Host::new("task-loss-script");
    host.enable();
    let pid_file = host.marks().join("started");
    let body = format!("echo $$ > '{}'\nsleep 300\n", pid_file.display());
    let job = host.job(
        "job-script",
        vec![simple("script-0", HookPhase::Before, &body, vec![])],
    );
    let jobs = host.jobs().with_panic_after_spawn();
    let collected = common::run(&jobs, &job).await;

    let result = collected.result();
    assert_eq!(result.outcome, JobStatus::Failed as i32);
    assert_eq!(
        result.error.as_ref().map(|error| error.code.as_str()),
        Some("agent_internal_error")
    );
    assert_eq!(collected.steps.len(), 1);
    assert_eq!(collected.steps[0].step_id, "script-0");
    assert!(!collected.steps[0].possible_stale_lock);
    assert_eq!(jobs.stop_running(Duration::from_secs(1)).await, 0);
    let pid = std::fs::read_to_string(&pid_file).expect("the script must write its pid");
    assert!(
        is_reaped(pid.trim()).await,
        "the killed script must be reaped"
    );
}
