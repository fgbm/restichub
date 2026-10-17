//! The repository roots of a host against a scripted restic (ADR-0045).

#![allow(clippy::expect_used)]

mod common;

use std::time::Duration;

use common::*;
use restichub_agent::repository_roots::RepositoryAccess;
use restichub_proto::agent::v1::{AgentMessage, JobStatus, RejectReason, agent_message};
use tokio::sync::mpsc;

const PASSWORD: &str = "CANARY-repository-roots-password";

async fn next_message(rx: &mut mpsc::Receiver<AgentMessage>) -> AgentMessage {
    tokio::time::timeout(Duration::from_secs(20), rx.recv())
        .await
        .expect("the agent must answer in time")
        .expect("the stream must stay open")
}

/// The agent refuses a local repository outside the roots before restic starts, and an init below
/// a root runs: its access test reports the missing repository with its own code.
#[tokio::test(flavor = "multi_thread")]
async fn adr_0045_a_local_repository_outside_the_roots_is_rejected_before_restic() {
    let dir = temp_config_dir("repository-roots-agent");
    std::fs::create_dir_all(&dir).expect("cannot create the configuration directory");
    // A canonical Windows path has the verbatim prefix, which the job validator refuses.
    let root = temp_config_dir("repository-roots-allowed");
    std::fs::create_dir_all(&root).expect("cannot create the root");
    let inside = root.join("local-test");
    let outside_parent = temp_config_dir("repository-roots-outside");
    std::fs::create_dir_all(&outside_parent).expect("cannot create the outside directory");
    let outside = outside_parent.join("local-test");
    script_exit_codes(&inside, 10, 0);
    script_exit_codes(&outside, 10, 0);
    let jobs = program_jobs(&dir)
        .with_repository_access(RepositoryAccess::new(vec![root.clone()], vec![dir.clone()]));
    let (tx, mut rx) = mpsc::channel(64);
    jobs.attach(&tx);
    jobs.greeted(false);

    let refused = job(
        "job-outside",
        vec![access("repo-1", &outside, PASSWORD)],
        vec![test_access("repo-1"), init("repo-1")],
    );
    jobs.handle(signed(&refused), &ring(), AGENT, TENANT).await;
    match next_message(&mut rx).await.payload {
        Some(agent_message::Payload::JobRejected(rejected)) => assert_eq!(
            RejectReason::try_from(rejected.reason).ok(),
            Some(RejectReason::RepositoryPathDenied)
        ),
        other => panic!("the job must be rejected: {other:?}"),
    }
    assert!(
        recorded_arguments(&outside).is_empty(),
        "a refused repository starts no restic"
    );

    let accepted = job(
        "job-inside",
        vec![access("repo-1", &inside, PASSWORD)],
        vec![test_access("repo-1"), init("repo-1")],
    );
    jobs.handle(signed(&accepted), &ring(), AGENT, TENANT).await;
    let mut probe_code = None;
    let result = loop {
        match next_message(&mut rx).await.payload {
            Some(agent_message::Payload::JobRejected(rejected)) => {
                panic!("a repository below a root must run: {rejected:?}")
            }
            Some(agent_message::Payload::StepResult(step)) if step.step_id == "test-access" => {
                probe_code = step.error.map(|error| error.code);
            }
            Some(agent_message::Payload::JobResult(result)) => break result,
            _ => {}
        }
    };
    assert_eq!(result.outcome, JobStatus::Succeeded as i32);
    assert_eq!(probe_code.as_deref(), Some("repository_missing"));
}
