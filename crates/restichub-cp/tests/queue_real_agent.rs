//! The lock retry with the messages of the real agent job runner (FR-SC-05, AC-SC-05, ADR-0020).
//!
//! The agent on this branch runs `init` and `test_access` steps only, so it cannot run a queued
//! backup. The restic double is the `fake-restic` example of the agent crate. The test runs a
//! `test_access` job on the real job runner with restic exit code 11 and captures the step result
//! and the job result that the runner produces. It then sends those messages, with the job
//! identifier of a queued backup, to the control plane. The control plane must retry the backup.
//!
//! The example is built by `cargo test --workspace`. A run of this package alone needs
//! `cargo build -p restichub-agent --example fake-restic` first.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::path::{Path, PathBuf};

use restichub_agent::jobs::Jobs;
use restichub_proto::agent::v1::{AgentMessage, StepStatus, agent_message};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use queue_support::{Env, decode_job_id, eventually, test_queue};

fn fake_restic() -> PathBuf {
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

fn sha256_hex(path: &Path) -> String {
    use std::fmt::Write as _;

    let bytes = std::fs::read(path).expect("cannot read the binary");
    let mut text = String::new();
    for byte in Sha256::digest(&bytes) {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[tokio::test]
async fn ac_sc_05_the_messages_of_the_real_agent_for_exit_code_11_retry_the_job() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("real-agent").await;
    let ring = Env::key_ring(&agent.identity);

    let work = std::env::temp_dir().join(format!("restichub-real-agent-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&work).expect("cannot create the work directory");
    let location = work.join("repo");
    std::fs::write(work.join("repo.restichub-fake"), "11 0").expect("cannot write the script");
    let program = fake_restic();
    let jobs = Jobs::open(
        program.clone(),
        sha256_hex(&program),
        work.join("cache"),
        &work,
    )
    .expect("the job journal must open");
    let (captured_tx, mut captured) = tokio::sync::mpsc::channel(32);
    jobs.attach(&captured_tx);

    let repository = env
        .create_repository_at(
            agent.id,
            "pw",
            location.to_str().expect("the path is UTF-8"),
        )
        .await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let backup = env.run_plan(plan).await;
    env.pass().await;
    let sent = decode_job_id(&agent.job().await);
    assert_eq!(sent, backup.to_string());
    agent.start(&env, &sent).await;

    // A test-access job runs on the real runner and exits with code 11.
    let (status, probe) = env
        .call(
            &env.admin,
            "POST",
            &format!("/api/v1/repositories/{repository}/test-access"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, axum::http::StatusCode::ACCEPTED, "{probe}");
    let signed = agent.job().await;
    let tenant = agent.identity.tenant_id.clone();
    let agent_id = agent.identity.agent_id.clone();
    jobs.handle(signed, &ring, &agent_id, &tenant).await;

    let mut step = None;
    let mut result = None;
    while result.is_none() {
        let message = tokio::time::timeout(std::time::Duration::from_secs(10), captured.recv())
            .await
            .expect("the real agent must report")
            .expect("the channel stays open");
        match message.payload {
            Some(agent_message::Payload::StepResult(found)) => step = Some(found),
            Some(agent_message::Payload::JobResult(found)) => result = Some(found),
            _ => {}
        }
    }
    let (mut step, mut result) = (step.expect("a step result"), result.expect("a job result"));
    assert_eq!(step.exit_code, 11);
    assert_eq!(step.status, StepStatus::Retry as i32);
    assert_eq!(
        result.status,
        StepStatus::Failed as i32,
        "the real agent reports the stopped job as failed"
    );

    // The control plane gets the same messages for the queued backup.
    step.job_id = sent.clone();
    result.job_id = sent.clone();
    for payload in [
        agent_message::Payload::StepResult(step),
        agent_message::Payload::JobResult(result),
    ] {
        agent
            .tx
            .send(AgentMessage {
                payload: Some(payload),
            })
            .await
            .expect("cannot send the message");
    }
    eventually(|| async { env.plan_jobs(plan).await.len() == 2 }).await;
    let newest = env.plan_jobs(plan).await.remove(0);
    assert_eq!(newest["status"], "queued", "{newest}");
    assert_eq!(newest["attempt"], 1);
    assert_ne!(newest["id"], sent.as_str());
    let ended = env.job(backup).await;
    assert_eq!(ended["status"], "failed");
    assert_eq!(ended["result_message"], "lock_retry");
    std::fs::remove_dir_all(&work).expect("cannot remove the work directory");
    env.db.cleanup().await;
}
