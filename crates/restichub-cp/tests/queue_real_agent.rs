//! The lock retry with the messages of the real agent job runner (FR-SC-05, AC-SC-05, ADR-0020).
//!
//! The restic double is the `fake-restic` example of the agent crate. Each test runs the queued
//! backup job on the real job runner, captures the step result and the job result that the runner
//! produces, and sends those messages to the control plane. Restic exit code 11 must retry the
//! backup as a new job. Restic exit code 3 must end it as `succeeded_with_warnings`.
//!
//! The example is built by `cargo test --workspace`. A run of this package alone needs
//! `cargo build -p restichub-agent --example fake-restic` first.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::path::{Path, PathBuf};

use restichub_agent::jobs::Jobs;
use restichub_proto::agent::v1::{AgentMessage, agent_message};
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

/// Runs the queued backup on the real runner with the scripted exit code and sends every message
/// of the runner to the control plane. Returns the plan and the backup job.
async fn run_backup(
    env: &Env,
    agent: &mut queue_support::FakeAgent,
    exit_code: i32,
) -> (Uuid, Uuid) {
    let ring = Env::key_ring(&agent.identity);
    let work = std::env::temp_dir().join(format!("restichub-real-agent-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&work).expect("cannot create the work directory");
    let location = work.join("repo");
    std::fs::write(work.join("repo.restichub-fake"), "0 0").expect("cannot write the script");
    std::fs::write(
        work.join("repo.restichub-code-backup"),
        exit_code.to_string(),
    )
    .expect("cannot write the backup code");
    let program = fake_restic();
    let jobs = Jobs::open(
        program.clone(),
        sha256_hex(&program),
        work.join("cache"),
        &work,
    )
    .expect("the job journal must open")
    .with_repository_access(restichub_agent::repository_roots::RepositoryAccess::unrestricted());
    let (captured_tx, mut captured) = tokio::sync::mpsc::channel(64);
    jobs.attach(&captured_tx);
    jobs.greeted(false);

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
    let signed = agent.job().await;
    assert_eq!(decode_job_id(&signed), backup.to_string());
    let tenant = agent.identity.tenant_id.clone();
    let agent_id = agent.identity.agent_id.clone();
    jobs.handle(signed, &ring, &agent_id, &tenant).await;

    let mut finished = false;
    while !finished {
        let message = tokio::time::timeout(std::time::Duration::from_secs(10), captured.recv())
            .await
            .expect("the real agent must report")
            .expect("the channel stays open");
        finished = matches!(message.payload, Some(agent_message::Payload::JobResult(_)));
        agent
            .tx
            .send(AgentMessage {
                payload: message.payload,
            })
            .await
            .expect("cannot send the message");
    }
    std::fs::remove_dir_all(&work).expect("cannot remove the work directory");
    (plan, backup)
}

#[tokio::test]
async fn ac_sc_05_the_backup_of_the_real_agent_with_exit_code_11_retries_as_a_new_job() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("real-agent").await;

    let (plan, backup) = run_backup(&env, &mut agent, 11).await;

    eventually(|| async { env.plan_jobs(plan).await.len() == 2 }).await;
    let newest = env.plan_jobs(plan).await.remove(0);
    assert_eq!(newest["status"], "queued", "{newest}");
    assert_eq!(newest["attempt"], 1);
    assert_ne!(newest["id"], backup.to_string().as_str());
    let ended = env.job(backup).await;
    assert_eq!(ended["status"], "failed");
    assert_eq!(ended["result_message"], "lock_retry");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_05_the_backup_of_the_real_agent_with_exit_code_3_ends_with_warnings() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("real-agent").await;

    let (plan, backup) = run_backup(&env, &mut agent, 3).await;

    eventually(|| async { env.job_status(backup).await == "succeeded" }).await;
    let ended = env.job(backup).await;
    assert_eq!(ended["result_code"], "succeeded_with_warnings", "{ended}");
    assert!(ended["result_message"].is_null(), "{ended}");
    assert_eq!(env.plan_jobs(plan).await.len(), 1);
    env.db.cleanup().await;
}
