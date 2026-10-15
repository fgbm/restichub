//! A progress write never outlives or overwrites the state of its job (FR-AG-08, FR-MN-01).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use restichub_db::job_steps::{self, JobProgress};
use restichub_proto::agent::v1::{AgentMessage, StepProgress, agent_message};
use time::OffsetDateTime;
use uuid::Uuid;

use queue_support::{Env, FakeAgent, decode_job_id, eventually, test_queue};

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

fn progress(job: &str, files_done: u64) -> AgentMessage {
    AgentMessage {
        payload: Some(agent_message::Payload::StepProgress(StepProgress {
            job_id: job.to_owned(),
            step_id: "backup".to_owned(),
            files_done,
            total_files: 100,
            ..StepProgress::default()
        })),
    }
}

fn row(files_done: i64, updated_at: OffsetDateTime) -> JobProgress {
    JobProgress {
        step_id: "backup".to_owned(),
        bytes_done: 0,
        total_bytes: 0,
        files_done,
        total_files: 100,
        percent_done: 0.0,
        seconds_elapsed: 0.0,
        seconds_remaining: 0,
        current_files: Vec::new(),
        updated_at,
    }
}

async fn progress_rows(env: &Env) -> i64 {
    let mut tx = env.scoped().await;
    sqlx::query_scalar("SELECT count(*) FROM job_progress")
        .fetch_one(&mut *tx)
        .await
        .expect("cannot count")
}

#[tokio::test]
async fn fr_ag_08_a_held_progress_leaves_no_row_after_the_job_ends() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("race-agent").await;
    let (job, id) = running_job(&env, &mut agent).await;

    for done in [10, 20, 30] {
        agent
            .tx
            .send(progress(&id, done))
            .await
            .expect("cannot send the progress");
    }
    // The second and third message wait for the flush of the window. The job ends before it.
    agent.finish(&env, &id).await;
    tokio::time::sleep(Duration::from_millis(1600)).await;

    assert!(env.job(job).await["progress"].is_null());
    assert_eq!(progress_rows(&env).await, 0);
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_ag_08_a_late_write_neither_revives_nor_overwrites_the_progress() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("late-agent").await;
    let (job, id) = running_job(&env, &mut agent).await;
    let job_id = restichub_db::JobId::from(job);

    agent
        .tx
        .send(progress(&id, 50))
        .await
        .expect("cannot send the progress");
    eventually(|| async { env.job(job).await["progress"]["files_done"] == 50 }).await;

    // A delayed write of an older message arrives after the newer one.
    let mut tx = env
        .db
        .database()
        .begin_tenant(env.tenant)
        .await
        .expect("cannot begin");
    let old = row(5, OffsetDateTime::now_utc() - time::Duration::minutes(5));
    assert!(
        job_steps::upsert_progress(&mut tx, job_id, &old)
            .await
            .expect("cannot write")
    );
    tx.commit().await.expect("cannot commit");
    assert_eq!(env.job(job).await["progress"]["files_done"], 50);

    // A delayed write that arrives after the result changes nothing.
    agent.finish(&env, &id).await;
    let mut tx = env
        .db
        .database()
        .begin_tenant(env.tenant)
        .await
        .expect("cannot begin");
    let late = row(99, OffsetDateTime::now_utc());
    assert!(
        !job_steps::upsert_progress(&mut tx, job_id, &late)
            .await
            .expect("cannot write")
    );
    tx.commit().await.expect("cannot commit");
    assert_eq!(progress_rows(&env).await, 0);
    assert!(env.job(job).await["progress"].is_null());
    env.db.cleanup().await;
}
