//! Job steps, progress, logs and retention through the agent stream and the web API (FR-MN-01,
//! FR-MN-02, FR-LG-01, FR-LG-02, FR-LG-03, AC-MN-04, AC-LG-01, AC-TN-01).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use axum::http::StatusCode;
use restichub_cp::log_store::{DatabaseLogStore, sweep};
use restichub_proto::agent::v1::{
    AgentMessage, BackupSummary, ErrorDetail, JobResult, JobStatus, LogChunk, LogStream,
    StepProgress, StepResult, StepStatus, agent_message, step_result,
};
use serde_json::Value;
use uuid::Uuid;

use common::{bearer_request, send};
use queue_support::{Env, FakeAgent, decode_job_id, eventually, test_queue};

fn message(payload: agent_message::Payload) -> AgentMessage {
    AgentMessage {
        payload: Some(payload),
    }
}

fn backup_summary() -> BackupSummary {
    BackupSummary {
        files_new: 12,
        files_changed: 3,
        files_unmodified: 100,
        dirs_new: 2,
        dirs_changed: 1,
        dirs_unmodified: 9,
        data_blobs: 14,
        tree_blobs: 3,
        data_added: 123_456,
        data_added_packed: 100_000,
        total_files_processed: 115,
        total_bytes_processed: 9_999_999,
        total_duration_seconds: 4.5,
        snapshot_id: "abcdef12".to_owned(),
        dry_run: false,
    }
}

fn log_chunk(job: &str, stream: LogStream, sequence: u64, data: &str) -> LogChunk {
    LogChunk {
        job_id: job.to_owned(),
        step_id: "backup".to_owned(),
        stream: stream as i32,
        sequence,
        data: data.as_bytes().to_vec(),
    }
}

/// Starts a queued manual backup on the agent and returns the plan and the running job.
async fn running_job(env: &Env, agent: &mut FakeAgent) -> (Uuid, Uuid, String) {
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(serde_json::json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job = env.run_plan(plan).await;
    env.pass().await;
    let signed = agent.job().await;
    let id = decode_job_id(&signed);
    assert_eq!(id, job.to_string());
    agent.start(env, &id).await;
    (plan, job, id)
}

async fn get(env: &Env, token: &str, uri: &str) -> (StatusCode, Value) {
    env.call(token, "GET", uri, None).await
}

#[tokio::test]
async fn fr_mn_01_the_running_job_shows_the_last_progress() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("progress-agent").await;
    let (_plan, job, id) = running_job(&env, &mut agent).await;

    for done in [10u64, 50] {
        agent
            .tx
            .send(message(agent_message::Payload::StepProgress(
                StepProgress {
                    job_id: id.clone(),
                    step_id: "backup".to_owned(),
                    bytes_done: done * 100,
                    total_bytes: 10_000,
                    files_done: done,
                    total_files: 100,
                    percent_done: done as f64 / 100.0,
                    seconds_elapsed: 3.0,
                    seconds_remaining: 7,
                    current_files: vec!["/srv/data/a.bin".to_owned()],
                },
            )))
            .await
            .expect("cannot send the progress");
    }
    eventually(|| async { env.job(job).await["progress"]["files_done"] == 50 }).await;
    let detail = env.job(job).await;
    assert_eq!(detail["status"], "running");
    assert_eq!(detail["progress"]["total_files"], 100);
    assert_eq!(detail["progress"]["percent_done"], 0.5);
    assert_eq!(detail["progress"]["current_files"][0], "/srv/data/a.bin");

    agent.finish(&env, &id).await;
    assert!(env.job(job).await["progress"].is_null());
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_mn_04_the_statistics_equal_the_restic_summary() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("summary-agent").await;
    let (_plan, job, id) = running_job(&env, &mut agent).await;

    agent
        .tx
        .send(message(agent_message::Payload::StepResult(StepResult {
            job_id: id.clone(),
            step_id: "backup".to_owned(),
            status: StepStatus::Success as i32,
            exit_code: 0,
            output: Some(step_result::Output::BackupSummary(backup_summary())),
            ..StepResult::default()
        })))
        .await
        .expect("cannot send the step result");
    agent.finish(&env, &id).await;

    let detail = env.job(job).await;
    let steps = detail["steps"].as_array().expect("the steps");
    assert_eq!(steps.len(), 1, "{detail}");
    assert_eq!(steps[0]["step_id"], "backup");
    assert_eq!(steps[0]["status"], "success");
    let summary = &detail["summary"];
    assert_eq!(summary["kind"], "backup");
    assert_eq!(summary["files_new"], 12);
    assert_eq!(summary["files_changed"], 3);
    assert_eq!(summary["files_unmodified"], 100);
    assert_eq!(summary["data_added"], 123_456);
    assert_eq!(summary["total_files_processed"], 115);
    assert_eq!(summary["total_bytes_processed"], 9_999_999);
    assert_eq!(summary["total_duration_seconds"], 4.5);
    assert_eq!(summary["snapshot_id"], "abcdef12");
    assert_eq!(steps[0]["output"], *summary);
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_lg_01_the_log_pages_with_a_line_number_keyset() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("log-agent").await;
    let (_plan, job, id) = running_job(&env, &mut agent).await;

    for (stream, sequence, data) in [
        (LogStream::Stdout, 1, "scan started\nscan finished\n"),
        (LogStream::Stderr, 1, "Warning: file is locked\n"),
        (LogStream::Stdout, 2, "snapshot saved\n"),
    ] {
        agent
            .tx
            .send(message(agent_message::Payload::LogChunk(log_chunk(
                &id, stream, sequence, data,
            ))))
            .await
            .expect("cannot send the chunk");
    }
    let uri = format!("/api/v1/jobs/{job}/logs");
    eventually(|| async {
        get(&env, &env.viewer, &uri).await.1["items"]
            .as_array()
            .map(Vec::len)
            == Some(4)
    })
    .await;

    let (status, page) = get(&env, &env.viewer, &uri).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let texts: Vec<_> = page["items"]
        .as_array()
        .expect("the items")
        .iter()
        .map(|line| line["text"].as_str().expect("text"))
        .collect();
    assert_eq!(
        texts,
        [
            "scan started",
            "scan finished",
            "Warning: file is locked",
            "snapshot saved"
        ]
    );
    assert_eq!(page["items"][2]["level"], "warn");
    assert_eq!(page["items"][2]["stream"], "stderr");
    assert_eq!(page["items"][0]["seq"], 1);
    assert_eq!(page["has_more"], false);

    let (_, second) = get(&env, &env.viewer, &format!("{uri}?after_seq=1&limit=2")).await;
    assert_eq!(second["items"][0]["text"], "scan finished");
    assert_eq!(second["items"].as_array().expect("items").len(), 2);
    assert_eq!(second["has_more"], true);

    let (bad, _) = get(&env, &env.viewer, &format!("{uri}?after_seq=-1")).await;
    assert_eq!(bad, StatusCode::BAD_REQUEST);
    assert_eq!(env.job(job).await["log"]["lines"], 4);
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_pr_07_a_resent_log_tail_stores_no_line_twice() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("resend-agent").await;
    let (_plan, job, id) = running_job(&env, &mut agent).await;

    let first = log_chunk(&id, LogStream::Stdout, 1, "one\ntwo\n");
    agent
        .tx
        .send(message(agent_message::Payload::LogChunk(first.clone())))
        .await
        .expect("cannot send the chunk");
    let uri = format!("/api/v1/jobs/{job}/logs");
    eventually(|| async {
        get(&env, &env.viewer, &uri).await.1["items"]
            .as_array()
            .map(Vec::len)
            == Some(2)
    })
    .await;

    // The result carries the tail again and one new chunk.
    let tail = vec![first, log_chunk(&id, LogStream::Stdout, 2, "three\n")];
    agent
        .tx
        .send(message(agent_message::Payload::JobResult(JobResult {
            job_id: id.clone(),
            status: StepStatus::Success as i32,
            steps_total: 1,
            steps_succeeded: 1,
            outcome: JobStatus::Succeeded as i32,
            log_tail: tail.clone(),
            ..JobResult::default()
        })))
        .await
        .expect("cannot send the result");
    eventually(|| async { env.job_status(job).await == "succeeded" }).await;
    eventually(|| async {
        get(&env, &env.viewer, &uri).await.1["items"]
            .as_array()
            .map(Vec::len)
            == Some(3)
    })
    .await;

    // The same result again, as after a second restart of the agent.
    agent
        .tx
        .send(message(agent_message::Payload::JobResult(JobResult {
            job_id: id.clone(),
            status: StepStatus::Success as i32,
            outcome: JobStatus::Succeeded as i32,
            log_tail: tail,
            ..JobResult::default()
        })))
        .await
        .expect("cannot send the result");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let (_, page) = get(&env, &env.viewer, &uri).await;
    let texts: Vec<_> = page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|line| line["text"].as_str().expect("text"))
        .collect();
    assert_eq!(texts, ["one", "two", "three"]);
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_ag_08_the_log_of_a_job_stops_at_the_bound_and_says_so() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("bound-agent").await;
    let (_plan, job, id) = running_job(&env, &mut agent).await;

    let line = format!("{}\n", "y".repeat(1000));
    let data = line.repeat(8);
    for sequence in 1..=200u64 {
        agent
            .tx
            .send(message(agent_message::Payload::LogChunk(log_chunk(
                &id,
                LogStream::Stdout,
                sequence,
                &data,
            ))))
            .await
            .expect("cannot send the chunk");
    }
    eventually(|| async { env.job(job).await["log"]["truncated"] == true }).await;
    let detail = env.job(job).await;
    let lines = detail["log"]["lines"].as_i64().expect("the line count");
    assert!(lines > 900 && lines < 1000, "{lines}");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_lg_01_the_retention_deletes_old_logs_and_keeps_the_job() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("retention-agent").await;
    let (_plan, job, id) = running_job(&env, &mut agent).await;

    agent
        .tx
        .send(message(agent_message::Payload::StepResult(StepResult {
            job_id: id.clone(),
            step_id: "backup".to_owned(),
            status: StepStatus::Success as i32,
            output: Some(step_result::Output::BackupSummary(backup_summary())),
            ..StepResult::default()
        })))
        .await
        .expect("cannot send the step result");
    agent
        .tx
        .send(message(agent_message::Payload::LogChunk(log_chunk(
            &id,
            LogStream::Stdout,
            1,
            "old line\n",
        ))))
        .await
        .expect("cannot send the chunk");
    let uri = format!("/api/v1/jobs/{job}/logs");
    eventually(|| async {
        get(&env, &env.viewer, &uri).await.1["items"]
            .as_array()
            .map(Vec::len)
            == Some(1)
    })
    .await;
    agent.finish(&env, &id).await;

    // Seven days of retention and a clock that moves eight days.
    env.clock.advance(8 * 24 * 3600);
    let store = DatabaseLogStore::new(env.state.db().clone());
    sweep(env.state.db(), &store, time::Duration::days(7), env.now())
        .await
        .expect("the sweep");

    let (_, page) = get(&env, &env.viewer, &uri).await;
    assert!(
        page["items"].as_array().expect("items").is_empty(),
        "{page}"
    );
    assert_eq!(page["purged"], true);
    let detail = env.job(job).await;
    assert_eq!(detail["status"], "succeeded", "{detail}");
    assert_eq!(detail["summary"]["files_new"], 12);
    assert_eq!(detail["log"]["purged"], true);
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_pr_08_a_message_for_a_job_of_another_agent_changes_nothing() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut owner = env.connect_agent("owner-agent").await;
    let (_plan, job, id) = running_job(&env, &mut owner).await;
    let stranger = env.connect_agent("stranger-agent").await;

    stranger
        .tx
        .send(message(agent_message::Payload::LogChunk(log_chunk(
            &id,
            LogStream::Stdout,
            1,
            "injected\n",
        ))))
        .await
        .expect("cannot send the chunk");
    stranger
        .tx
        .send(message(agent_message::Payload::StepResult(StepResult {
            job_id: id.clone(),
            step_id: "backup".to_owned(),
            status: StepStatus::Failed as i32,
            exit_code: 1,
            error: Some(ErrorDetail {
                code: "injected".to_owned(),
                params: Vec::new(),
            }),
            ..StepResult::default()
        })))
        .await
        .expect("cannot send the step result");
    // The owner's own message passes through the same stream path after the stranger's messages.
    owner
        .tx
        .send(message(agent_message::Payload::LogChunk(log_chunk(
            &id,
            LogStream::Stdout,
            1,
            "genuine\n",
        ))))
        .await
        .expect("cannot send the chunk");
    let uri = format!("/api/v1/jobs/{job}/logs");
    eventually(|| async {
        get(&env, &env.viewer, &uri).await.1["items"]
            .as_array()
            .map(Vec::len)
            == Some(1)
    })
    .await;
    let (_, page) = get(&env, &env.viewer, &uri).await;
    assert_eq!(page["items"][0]["text"], "genuine");
    assert!(
        env.job(job).await["steps"]
            .as_array()
            .expect("steps")
            .is_empty()
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_01_the_job_endpoints_answer_404_for_a_job_of_another_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("tenant-agent").await;
    let (_plan, job, _id) = running_job(&env, &mut agent).await;

    let other = common::create_tenant_with_key(env.db.database(), "other-tenant").await;
    let other_viewer = common::seed_api_token(env.db.database(), other, "viewer").await;
    for uri in [
        format!("/api/v1/jobs/{job}"),
        format!("/api/v1/jobs/{job}/logs"),
    ] {
        let (status, _) = get(&env, &other_viewer, &uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        let (status, _) = get(&env, &env.viewer, &uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
    }
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_au_02_the_job_endpoints_need_a_credential_and_the_viewer_role() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("role-agent").await;
    let (_plan, job, _id) = running_job(&env, &mut agent).await;

    for uri in [
        format!("/api/v1/jobs/{job}"),
        format!("/api/v1/jobs/{job}/logs"),
    ] {
        let (status, _, _) = send(
            &env.app,
            axum::http::Request::builder()
                .method("GET")
                .uri(&uri)
                .body(axum::body::Body::empty())
                .expect("the request"),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
        for token in [&env.viewer, &env.operator, &env.admin] {
            let (status, _, _) = send(&env.app, bearer_request("GET", &uri, token, None)).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
        }
    }
    env.db.cleanup().await;
}
