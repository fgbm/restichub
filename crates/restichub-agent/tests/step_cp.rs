//! A backup job from the real control plane listener to the agent and its scripted restic (FR-PR-03,
//! FR-PR-10, FR-AG-08, FR-AG-12, ADR-0020, AC-AG-03).
//!
//! The control plane signs the job with its own job signing key and sends it on the open agent
//! stream. The agent verifies it, runs the steps and keeps the typed result in its journal. The job
//! has no row in the database, so the control plane ignores the result; the journal is the witness.

#![allow(clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use restichub_agent::journal::{FileJournal, JOURNAL_FILE};
use restichub_proto::agent::v1::{
    ControlMessage, JobStatus, StepStatus, control_message, step_result::Output,
};
use restichub_proto::signing::{JobJournal as _, JournalState};
use uuid::Uuid;

const SUMMARY: &str = r#"{"message_type":"summary","files_new":5,"files_changed":0,"files_unmodified":0,"dirs_new":1,"dirs_changed":0,"dirs_unmodified":0,"data_blobs":5,"tree_blobs":1,"data_added":512,"data_added_packed":256,"total_files_processed":5,"total_bytes_processed":900,"total_duration":0.25,"snapshot_id":"c0ffee"}"#;

/// FR-AG-08, FR-PR-10: a signed backup job from the control plane runs with the arguments of the
/// compiler, and the typed summary lands in the journal without the secret.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_08_a_signed_backup_job_from_the_control_plane_runs_and_is_journaled() {
    let Some(fixture) = stream_fixture("agent-step-cp").await else {
        return;
    };
    let dir = temp_config_dir("agent-step-cp");
    let identity = Arc::new(register(&fixture, &dir).await);
    let agent = Uuid::parse_str(&identity.agent_id).expect("the agent id");
    let repository = temp_config_dir("agent-step-cp-repo");
    std::fs::create_dir_all(&repository).expect("cannot create the repository path");
    script_stdout(&repository, "backup", &format!("{SUMMARY}\n"));

    let jobs = program_jobs(&dir);
    let (stop_tx, task) = run_agent(&dir, &identity, &jobs);
    assert!(
        wait_online(&fixture, agent).await,
        "the agent must be online"
    );

    let canary = format!("CANARY-{}", Uuid::now_v7());
    let mut backup = backup("repo-1", &["/srv/data"]);
    backup.limit_upload_kib = 512;
    backup.retry_lock = true;
    let mut job = job(
        "00000000-0000-7000-8000-00000000a001",
        vec![access("repo-1", &repository, &canary)],
        vec![make_step(
            "backup",
            restichub_proto::agent::v1::step::Kind::Backup(backup),
        )],
    );
    job.agent_id.clone_from(&identity.agent_id);
    job.tenant_id.clone_from(&identity.tenant_id);
    let signed = fixture
        .server
        .state
        .job_signing()
        .expect("the control plane signs jobs")
        .sign_job(fixture.db.database(), &job)
        .await
        .expect("cannot sign the job");
    assert!(fixture.server.state.agent_registry().send_to_agent(
        restichub_db::AgentId::from_uuid(agent),
        ControlMessage {
            payload: Some(control_message::Payload::SignedJob(signed)),
        },
    ));

    let journal_path = dir.join(JOURNAL_FILE);
    let mut stored = None;
    for _ in 0..200 {
        if let Ok(journal) = FileJournal::load(journal_path.clone())
            && let JournalState::Completed(result) = journal.lookup(&job.job_id)
        {
            stored = Some(result);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let result = stored.expect("the agent must journal the result");
    assert_eq!(result.outcome, JobStatus::Succeeded as i32);
    assert_eq!(result.steps.len(), 1);
    assert_eq!(result.steps[0].status, StepStatus::Success as i32);
    let Some(Output::BackupSummary(summary)) = result.steps[0].output.clone() else {
        panic!("the journal must hold the backup summary");
    };
    assert_eq!(summary.files_new, 5);
    assert_eq!(summary.snapshot_id, "c0ffee");

    let arguments = recorded_arguments(&repository);
    assert_eq!(
        arguments.trim(),
        "backup --json --limit-upload=512 --retry-lock=5m -- /srv/data"
    );
    assert!(!arguments.contains(&canary));
    let journal = std::fs::read(&journal_path).expect("the journal file");
    assert!(!String::from_utf8_lossy(&journal).contains(&canary));

    stop_tx.send(true).expect("cannot stop the agent");
    let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&repository);
    fixture.db.cleanup().await;
}
