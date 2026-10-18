//! The bounds of the job log and the write window of a finished job (FR-AG-08, FR-LG-01, FR-LG-02,
//! AC-LG-01).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use axum::http::StatusCode;
use restichub_proto::agent::v1::{AgentMessage, LogChunk, LogStream, agent_message};
use serde_json::Value;
use uuid::Uuid;

use queue_support::{Env, FakeAgent, decode_job_id, eventually, test_queue};

fn message(payload: agent_message::Payload) -> AgentMessage {
    AgentMessage {
        payload: Some(payload),
    }
}

fn log_chunk(job: &str, sequence: u64, data: &str) -> LogChunk {
    LogChunk {
        job_id: job.to_owned(),
        step_id: "backup".to_owned(),
        stream: LogStream::Stdout as i32,
        sequence,
        data: data.as_bytes().to_vec(),
    }
}

async fn send_chunk(agent: &FakeAgent, chunk: LogChunk) {
    agent
        .tx
        .send(message(agent_message::Payload::LogChunk(chunk)))
        .await
        .expect("cannot send the chunk");
}

async fn running_job(env: &Env, agent: &mut FakeAgent) -> (Uuid, String) {
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(serde_json::json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job = env.run_plan(plan).await;
    env.pass().await;
    let signed = agent.job().await;
    let id = decode_job_id(&signed);
    agent.start(env, &id).await;
    (job, id)
}

async fn lines(env: &Env, job: Uuid) -> Vec<String> {
    let (status, page): (StatusCode, Value) = env
        .call(
            &env.viewer,
            "GET",
            &format!("/api/v1/jobs/{job}/logs?limit=2000"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|line| line["text"].as_str().expect("text").to_owned())
        .collect()
}

#[tokio::test]
async fn fr_ag_08_chunks_of_empty_lines_hit_the_bound() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("empty-lines-agent").await;
    let (job, id) = running_job(&env, &mut agent).await;

    let data = "\n".repeat(60_000);
    for sequence in 1..=20u64 {
        send_chunk(&agent, log_chunk(&id, sequence, &data)).await;
    }
    eventually(|| async { env.job(job).await["log"]["truncated"] == true }).await;
    let detail = env.job(job).await;
    let stored = detail["log"]["lines"].as_i64().expect("the line count");
    assert!(stored > 0 && stored <= 100_000, "{stored}");
    // 1 MiB at 65 bytes for each empty line.
    assert!(stored < 17_000, "{stored}");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_pr_07_chunks_are_kept_by_their_number_in_any_order() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("order-agent").await;
    let (job, id) = running_job(&env, &mut agent).await;

    send_chunk(&agent, log_chunk(&id, 1, "one\n")).await;
    send_chunk(&agent, log_chunk(&id, 3, "three\n")).await;
    send_chunk(&agent, log_chunk(&id, 2, "two\n")).await;
    eventually(|| async { lines(&env, job).await.len() == 3 }).await;
    // Sent again after a restart of the agent.
    send_chunk(&agent, log_chunk(&id, 2, "two\n")).await;
    send_chunk(&agent, log_chunk(&id, 4, "four\n")).await;
    eventually(|| async { lines(&env, job).await.len() == 4 }).await;
    assert_eq!(lines(&env, job).await, ["one", "three", "two", "four"]);
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_lg_02_a_finished_job_takes_a_log_line_only_inside_the_retention_window() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("window-agent").await;
    let (job, id) = running_job(&env, &mut agent).await;
    agent.finish(&env, &id).await;

    // Inside the default window of 30 days.
    env.clock.advance(29 * 24 * 3600);
    send_chunk(&agent, log_chunk(&id, 1, "inside\n")).await;
    eventually(|| async { lines(&env, job).await == ["inside"] }).await;

    env.clock.advance(2 * 24 * 3600);
    send_chunk(&agent, log_chunk(&id, 2, "outside\n")).await;
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(lines(&env, job).await, ["inside"]);
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_lg_01_format_characters_are_removed_from_a_stored_line() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("format-agent").await;
    let (job, id) = running_job(&env, &mut agent).await;

    send_chunk(
        &agent,
        log_chunk(&id, 1, "a\u{202E}b\u{200B}c\u{FEFF}\td\n"),
    )
    .await;
    eventually(|| async { lines(&env, job).await.len() == 1 }).await;
    assert_eq!(lines(&env, job).await, ["abc\td"]);
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_mn_02_the_detail_lists_all_planned_steps_and_the_trigger() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("planned-agent").await;
    let (job, _id) = running_job(&env, &mut agent).await;

    let detail = env.job(job).await;
    let planned = detail["planned_steps"].as_array().expect("planned steps");
    assert!(!planned.is_empty(), "{detail}");
    assert!(detail["steps"].as_array().expect("steps").is_empty());
    assert_eq!(detail["trigger"], "manual");
    env.db.cleanup().await;
}
