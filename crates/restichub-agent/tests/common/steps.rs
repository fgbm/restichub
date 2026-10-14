//! Step and job builders for the agent step tests, and the collector of the agent messages.

#![allow(dead_code)]
#![allow(clippy::expect_used)]

use std::path::Path;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use restichub_agent::jobs::Jobs;
use restichub_proto::agent::v1::{
    AgentMessage, BackupStep, CheckStep, ForgetStep, InitStep, Job, JobResult, LocalCredentials,
    LogChunk, PruneStep, RepositoryAccess, SignedJob, SnapshotsStep, StatsStep, Step, StepProgress,
    StepResult, TestAccessStep, UnlockAllStep, UnlockStep, agent_message,
    repository_access::BackendCredentials, step,
};
use restichub_proto::signing::{KeyRing, sign_job};
use tokio::sync::mpsc;

pub const KEY_ID: &str = "job-key-1";
pub const AGENT: &str = "agent-1";
pub const TENANT: &str = "tenant-1";

pub fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7_u8; 32])
}

pub fn ring() -> KeyRing {
    let mut ring = KeyRing::new();
    ring.insert(KEY_ID, signing_key().verifying_key())
        .expect("insert");
    ring
}

pub fn at(offset_seconds: i64) -> prost_types::Timestamp {
    let now = time::OffsetDateTime::now_utc();
    prost_types::Timestamp {
        seconds: now.unix_timestamp() + offset_seconds,
        nanos: 0,
    }
}

pub fn access(repository: &str, location: &Path, password: &str) -> RepositoryAccess {
    RepositoryAccess {
        repository_id: repository.to_owned(),
        location: location.to_string_lossy().into_owned(),
        password: password.to_owned(),
        backend_credentials: Some(BackendCredentials::Local(LocalCredentials {})),
    }
}

pub fn make_step(id: &str, kind: step::Kind) -> Step {
    Step {
        step_id: id.to_owned(),
        kind: Some(kind),
        ..Step::default()
    }
}

pub fn test_access(repository: &str) -> Step {
    make_step(
        "test-access",
        step::Kind::TestAccess(TestAccessStep {
            repository_id: repository.to_owned(),
        }),
    )
}

pub fn init(repository: &str) -> Step {
    make_step(
        "init",
        step::Kind::Init(InitStep {
            repository_id: repository.to_owned(),
        }),
    )
}

pub fn backup(repository: &str, paths: &[&str]) -> BackupStep {
    BackupStep {
        repository_id: repository.to_owned(),
        paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        ..BackupStep::default()
    }
}

pub fn backup_step(repository: &str, paths: &[&str]) -> Step {
    make_step("backup", step::Kind::Backup(backup(repository, paths)))
}

pub fn check_step(repository: &str) -> Step {
    make_step(
        "check",
        step::Kind::Check(CheckStep {
            repository_id: repository.to_owned(),
            ..CheckStep::default()
        }),
    )
}

pub fn snapshots_step(repository: &str) -> Step {
    make_step(
        "snapshots",
        step::Kind::Snapshots(SnapshotsStep {
            repository_id: repository.to_owned(),
            ..SnapshotsStep::default()
        }),
    )
}

pub fn forget_step(repository: &str, keep_last: u32) -> Step {
    make_step(
        "forget",
        step::Kind::Forget(ForgetStep {
            repository_id: repository.to_owned(),
            keep_last,
            ..ForgetStep::default()
        }),
    )
}

pub fn prune_step(repository: &str) -> Step {
    make_step(
        "prune",
        step::Kind::Prune(PruneStep {
            repository_id: repository.to_owned(),
            ..PruneStep::default()
        }),
    )
}

pub fn unlock_step(repository: &str) -> Step {
    make_step(
        "unlock",
        step::Kind::Unlock(UnlockStep {
            repository_id: repository.to_owned(),
        }),
    )
}

pub fn unlock_all_step(repository: &str) -> Step {
    make_step(
        "unlock-all",
        step::Kind::UnlockAll(UnlockAllStep {
            repository_id: repository.to_owned(),
        }),
    )
}

pub fn stats_step(repository: &str) -> Step {
    make_step(
        "stats",
        step::Kind::Stats(StatsStep {
            repository_id: repository.to_owned(),
            ..StatsStep::default()
        }),
    )
}

/// A job with the given repositories and steps, valid for the current clock.
pub fn job(job_id: &str, repositories: Vec<RepositoryAccess>, steps: Vec<Step>) -> Job {
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

pub fn signed(job: &Job) -> SignedJob {
    sign_job(job, KEY_ID, &signing_key())
}

/// Everything that the agent sent for one job.
#[derive(Default)]
pub struct Collected {
    pub progress: Vec<StepProgress>,
    pub logs: Vec<LogChunk>,
    pub steps: Vec<StepResult>,
    pub result: Option<JobResult>,
    pub messages: Vec<AgentMessage>,
}

impl Collected {
    /// The text of the log chunks, in the order of arrival.
    pub fn log_text(&self) -> String {
        self.logs
            .iter()
            .map(|chunk| String::from_utf8_lossy(&chunk.data).into_owned())
            .collect()
    }

    pub fn step(&self, step_id: &str) -> &StepResult {
        self.steps
            .iter()
            .find(|step| step.step_id == step_id)
            .unwrap_or_else(|| panic!("no result for step {step_id}"))
    }

    pub fn result(&self) -> &JobResult {
        self.result.as_ref().expect("the job result")
    }
}

/// Reads the messages until the final job result arrives, or panics after thirty seconds.
pub async fn collect(rx: &mut mpsc::Receiver<AgentMessage>) -> Collected {
    let mut collected = Collected::default();
    loop {
        let message = tokio::time::timeout(Duration::from_secs(30), rx.recv())
            .await
            .expect("the agent must answer")
            .expect("the channel must stay open");
        collected.messages.push(message.clone());
        match message.payload {
            Some(agent_message::Payload::StepProgress(progress)) => {
                collected.progress.push(progress);
            }
            Some(agent_message::Payload::LogChunk(chunk)) => collected.logs.push(chunk),
            Some(agent_message::Payload::StepResult(step)) => collected.steps.push(step),
            Some(agent_message::Payload::JobResult(result)) => {
                collected.result = Some(result);
                return collected;
            }
            _ => {}
        }
    }
}

/// Runs one job on a fresh channel and collects the messages.
pub async fn run(jobs: &Jobs, job: &Job) -> Collected {
    let (tx, mut rx) = mpsc::channel(256);
    jobs.attach(&tx);
    jobs.handle(signed(job), &ring(), AGENT, TENANT).await;
    collect(&mut rx).await
}
