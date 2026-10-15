//! The control plane acknowledges stored log chunks and stores a resent chunk once, in order
//! (FR-LG-07, AC-LG-05, AC-PR-07).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use axum::http::StatusCode;
use restichub_proto::AgentCapability;
use restichub_proto::agent::v1::{
    AgentMessage, LogAck, LogChunk, LogStream, agent_message, control_message,
};
use serde_json::Value;
use uuid::Uuid;

use queue_support::{Env, FakeAgent, decode_job_id, eventually, test_queue};

fn chunk(job: &str, sequence: u64, data: &str) -> AgentMessage {
    AgentMessage {
        payload: Some(agent_message::Payload::LogChunk(LogChunk {
            job_id: job.to_owned(),
            step_id: "backup".to_owned(),
            stream: LogStream::Stdout as i32,
            sequence,
            data: data.as_bytes().to_vec(),
        })),
    }
}

async fn running_job(env: &Env, agent: &mut FakeAgent) -> (Uuid, String) {
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, serde_json::json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    let job = env.run_plan(plan).await;
    env.pass().await;
    let signed = agent.job().await;
    let id = decode_job_id(&signed);
    agent.start(env, &id).await;
    (job, id)
}

async fn next_ack(agent: &mut FakeAgent, wait: Duration) -> Option<LogAck> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let message = tokio::time::timeout(left, agent.inbound.message())
            .await
            .ok()?
            .ok()??;
        if let Some(control_message::Payload::LogAck(ack)) = message.payload {
            return Some(ack);
        }
    }
}

async fn texts(env: &Env, job: Uuid) -> Vec<String> {
    let (status, page): (StatusCode, Value) = env
        .call(
            &env.viewer,
            "GET",
            &format!("/api/v1/jobs/{job}/logs"),
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
async fn ac_lg_05_a_resent_log_is_stored_once_in_order_and_acknowledged() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("ack-agent").await;
    let (job, id) = running_job(&env, &mut agent).await;

    for (sequence, text) in [(1, "one\n"), (2, "two\n")] {
        agent
            .tx
            .send(chunk(&id, sequence, text))
            .await
            .expect("cannot send the chunk");
    }
    let ack = next_ack(&mut agent, Duration::from_secs(10)).await;
    assert_eq!(ack.expect("an acknowledgement").job_id, id);
    eventually(|| async { texts(&env, job).await.len() == 2 }).await;

    // The agent reconnects on a new stream and sends everything it still holds from the first
    // chunk.
    let capabilities = AgentCapability::ALL
        .iter()
        .map(|capability| capability.as_str().to_owned())
        .collect();
    let mut agent = env.reconnect_agent(&agent, capabilities).await;
    for (sequence, text) in [(1, "one\n"), (2, "two\n"), (3, "three\n"), (4, "four\n")] {
        agent
            .tx
            .send(chunk(&id, sequence, text))
            .await
            .expect("cannot send the chunk");
    }
    eventually(|| async { texts(&env, job).await.len() == 4 }).await;
    assert_eq!(texts(&env, job).await, ["one", "two", "three", "four"]);

    let mut newest = 0;
    while let Some(ack) = next_ack(&mut agent, Duration::from_millis(500)).await {
        for stream in ack.streams {
            assert_eq!(stream.stream, LogStream::Stdout as i32);
            newest = newest.max(stream.sequence);
        }
    }
    assert_eq!(newest, 4, "the control plane acknowledges the newest chunk");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_lg_05_an_agent_without_the_capability_gets_no_acknowledgement() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let capabilities = AgentCapability::ALL
        .iter()
        .filter(|capability| **capability != AgentCapability::LogAckV1)
        .map(|capability| capability.as_str().to_owned())
        .collect();
    let mut agent = env.connect_agent_with("old-agent", capabilities).await;
    let (job, id) = running_job(&env, &mut agent).await;
    agent
        .tx
        .send(chunk(&id, 1, "one\n"))
        .await
        .expect("cannot send the chunk");
    eventually(|| async { texts(&env, job).await.len() == 1 }).await;
    assert!(next_ack(&mut agent, Duration::from_secs(1)).await.is_none());
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_lg_05_a_chunk_larger_than_the_bound_is_refused_and_does_not_stop_the_acknowledgements()
{
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("big-agent").await;
    let (job, id) = running_job(&env, &mut agent).await;
    let oversize = "x".repeat(restichub_cp::log_store::MAX_CHUNK_BYTES + 1);
    for (sequence, text) in [(1, "one\n"), (2, oversize.as_str()), (3, "three\n")] {
        agent
            .tx
            .send(chunk(&id, sequence, text))
            .await
            .expect("cannot send the chunk");
    }
    eventually(|| async { texts(&env, job).await.len() == 2 }).await;
    assert_eq!(texts(&env, job).await, ["one", "three"]);
    let mut newest = 0;
    while let Some(ack) = next_ack(&mut agent, Duration::from_secs(1)).await {
        for stream in ack.streams {
            newest = newest.max(stream.sequence);
        }
    }
    assert_eq!(
        newest, 3,
        "the refused chunk does not hold back the next ones"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_lg_05_acknowledgements_of_many_chunks_are_coalesced() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("many-agent").await;
    let (job, id) = running_job(&env, &mut agent).await;
    for sequence in 1..=60 {
        agent
            .tx
            .send(chunk(&id, sequence, "line\n"))
            .await
            .expect("cannot send the chunk");
    }
    eventually(|| async { texts(&env, job).await.len() == 60 }).await;
    let mut count = 0;
    let mut newest = 0;
    while let Some(ack) = next_ack(&mut agent, Duration::from_millis(600)).await {
        count += 1;
        for stream in ack.streams {
            newest = newest.max(stream.sequence);
        }
    }
    assert_eq!(newest, 60);
    assert!(count < 30, "{count} acknowledgements for 60 chunks");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_lg_05_a_chunk_of_another_job_is_refused_and_never_stored() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("foreign-agent").await;
    let (_job, _id) = running_job(&env, &mut agent).await;
    let stray = Uuid::new_v4().to_string();
    agent
        .tx
        .send(chunk(&stray, 1, "stray\n"))
        .await
        .expect("cannot send the chunk");
    // The refusal is for good, so the agent may drop its copy; the log of no job holds the line.
    let ack = next_ack(&mut agent, Duration::from_secs(5)).await;
    assert_eq!(ack.expect("an acknowledgement").job_id, stray);
    assert!(texts(&env, _job).await.is_empty());
    env.db.cleanup().await;
}
