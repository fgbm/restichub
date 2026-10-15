//! The job detail and the job log of a backup that the real agent job runner produced (FR-MN-02,
//! FR-LG-01, AC-MN-04, AC-LG-04, NFR-SEC-05).
//!
//! The restic double is the `fake-restic` example of the agent crate. The test runs the queued
//! backup on the real runner, captures every message and sends it to the control plane. The double
//! prints the restic JSON `status` and `summary` messages, and echoes the repository password, so
//! the test proves two things end to end: the numbers on the job detail equal the restic summary,
//! and the password canary appears in no stored log line.
//!
//! A run of this package alone needs `cargo build -p restichub-agent --example fake-restic` first.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::path::{Path, PathBuf};

use restichub_agent::jobs::Jobs;
use restichub_canary_scan::{Canary, Scanner};
use restichub_proto::agent::v1::{AgentMessage, agent_message};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use queue_support::{Env, decode_job_id, eventually, test_queue};

const CANARY: &str = "CANARY-repository-password-7f3a9c1e5d";

const BACKUP_OUTPUT: &str = r#"{"message_type":"status","percent_done":0.5,"total_files":10,"files_done":5,"total_bytes":1000,"bytes_done":500,"seconds_elapsed":2}
scanning the source
{"message_type":"summary","files_new":3,"files_changed":1,"files_unmodified":6,"dirs_new":1,"dirs_changed":0,"dirs_unmodified":2,"data_blobs":4,"tree_blobs":2,"data_added":2048,"data_added_packed":1500,"total_files_processed":10,"total_bytes_processed":4096,"total_duration":1.25,"snapshot_id":"deadbeef","dry_run":false}
snapshot deadbeef saved
"#;

const BACKUP_ERRORS: &str =
    "Fatal: the key {PASSWORD} is not accepted by {REPOSITORY}\nwarning: echo {PASSWORD}\n";

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

/// Runs the queued backup on the real runner and sends every message to the control plane.
async fn run_backup(env: &Env, agent: &mut queue_support::FakeAgent) -> Uuid {
    run_backup_of_profile(env, agent).await.0
}

/// Like [`run_backup`], and returns the profile that the job ran.
async fn run_backup_of_profile(env: &Env, agent: &mut queue_support::FakeAgent) -> (Uuid, Uuid) {
    let ring = Env::key_ring(&agent.identity);
    let work = std::env::temp_dir().join(format!("restichub-logs-agent-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&work).expect("cannot create the work directory");
    let location = work.join("repo");
    std::fs::write(work.join("repo.restichub-fake"), "0 0").expect("cannot write the script");
    std::fs::write(work.join("repo.restichub-code-backup"), "0").expect("cannot write the code");
    std::fs::write(work.join("repo.restichub-out-backup"), BACKUP_OUTPUT)
        .expect("cannot write the output");
    std::fs::write(work.join("repo.restichub-err-backup"), BACKUP_ERRORS)
        .expect("cannot write the errors");
    let program = fake_restic();
    let jobs = Jobs::open(
        program.clone(),
        sha256_hex(&program),
        work.join("cache"),
        &work,
    )
    .expect("the job journal must open");
    let (captured_tx, mut captured) = tokio::sync::mpsc::channel(64);
    jobs.attach(&captured_tx);
    jobs.greeted(false);

    let repository = env
        .create_repository_at(
            agent.id,
            CANARY,
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
    (backup, profile)
}

async fn all_lines(env: &Env, job: Uuid) -> Vec<Value> {
    let (status, page) = env
        .call(
            &env.viewer,
            "GET",
            &format!("/api/v1/jobs/{job}/logs?limit=2000"),
            None,
        )
        .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{page}");
    page["items"].as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn ac_mn_04_the_statistics_of_the_real_runner_equal_the_restic_summary() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("real-summary-agent").await;

    let backup = run_backup(&env, &mut agent).await;
    eventually(|| async { env.job_status(backup).await == "succeeded" }).await;

    let detail = env.job(backup).await;
    let summary = &detail["summary"];
    assert_eq!(summary["kind"], "backup", "{detail}");
    assert_eq!(summary["files_new"], 3);
    assert_eq!(summary["files_changed"], 1);
    assert_eq!(summary["files_unmodified"], 6);
    assert_eq!(summary["dirs_new"], 1);
    assert_eq!(summary["dirs_unmodified"], 2);
    assert_eq!(summary["data_blobs"], 4);
    assert_eq!(summary["tree_blobs"], 2);
    assert_eq!(summary["data_added"], 2048);
    assert_eq!(summary["data_added_packed"], 1500);
    assert_eq!(summary["total_files_processed"], 10);
    assert_eq!(summary["total_bytes_processed"], 4096);
    assert_eq!(summary["total_duration_seconds"], 1.25);
    assert_eq!(summary["snapshot_id"], "deadbeef");
    assert_eq!(summary["dry_run"], false);
    assert_eq!(detail["steps"][0]["status"], "success");

    let texts: Vec<String> = all_lines(&env, backup)
        .await
        .iter()
        .filter_map(|line| line["text"].as_str().map(str::to_owned))
        .collect();
    assert!(
        texts.iter().any(|text| text == "scanning the source"),
        "{texts:?}"
    );
    assert!(
        texts.iter().any(|text| text == "snapshot deadbeef saved"),
        "{texts:?}"
    );
    // The JSON status and summary lines are typed output, not log text.
    assert!(
        texts.iter().all(|text| !text.contains("message_type")),
        "{texts:?}"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_lg_04_the_password_canary_appears_in_no_stored_log_line() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("real-canary-agent").await;

    let backup = run_backup(&env, &mut agent).await;
    eventually(|| async { env.job_status(backup).await == "succeeded" }).await;

    let lines = all_lines(&env, backup).await;
    // The double echoed the password twice, so the log holds the lines with the mask in its place.
    assert!(
        lines.iter().any(|line| line["text"]
            .as_str()
            .is_some_and(|text| text.contains("Fatal:"))),
        "{lines:?}"
    );
    let scanner = Scanner::new(&[Canary::parse(CANARY)]);

    // The log rows as the database holds them, and the whole API view of the job.
    let (dump,): (String,) = sqlx::query_as(
        "SELECT COALESCE(string_agg(text, E'\\n'), '') FROM job_logs WHERE job_id = $1",
    )
    .bind(backup)
    .fetch_one(env.db.owner_pool())
    .await
    .expect("cannot dump the log rows");
    let findings = scanner.scan("job_logs", dump.as_bytes());
    assert!(findings.is_empty(), "the log rows hold the canary");
    let detail = env.job(backup).await.to_string();
    assert!(scanner.scan("job detail", detail.as_bytes()).is_empty());
    let logs = Value::Array(lines).to_string();
    assert!(scanner.scan("job log api", logs.as_bytes()).is_empty());
    env.db.cleanup().await;
}

/// AC-PF-01: the command line in the step log is the command line of the preview, with the secrets
/// masked.
#[tokio::test]
async fn ac_pf_01_the_step_log_holds_the_masked_command_line_of_the_preview() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("real-command-agent").await;

    let (backup, profile) = run_backup_of_profile(&env, &mut agent).await;
    eventually(|| async { env.job_status(backup).await == "succeeded" }).await;

    let (status, preview) = env
        .call(
            &env.admin,
            "POST",
            &format!("/api/v1/profiles/{profile}/preview"),
            Some(json!({"agent_id": agent.id.to_string()})),
        )
        .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{preview}");
    let previewed = preview["steps"]
        .as_array()
        .and_then(|steps| steps.iter().find(|step| step["kind"] == "backup"))
        .and_then(|step| step["command"].as_str())
        .expect("the backup command of the preview");

    let lines = all_lines(&env, backup).await;
    let logged: Vec<&str> = lines
        .iter()
        .filter(|line| line["stream"] == "agent")
        .filter_map(|line| line["text"].as_str())
        .filter(|text| text.starts_with("$ "))
        .collect();
    assert_eq!(logged.len(), 1, "{lines:?}");
    assert_eq!(logged[0], format!("$ {previewed}"));
    assert!(
        !logged[0].contains(CANARY),
        "the command line holds no secret"
    );
    env.db.cleanup().await;
}
