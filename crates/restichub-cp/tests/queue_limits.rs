//! The repository barriers and the limits of the queue (FR-SC-04, FR-SC-08, AC-SC-03, AC-SC-08).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use restichub_db::JobKind;
use serde_json::json;

use queue_support::{Env, test_queue};

const QUIET: Duration = Duration::from_millis(400);

#[tokio::test]
async fn ac_sc_03_a_prune_waits_for_three_backups_and_holds_the_later_backups() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("barrier-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env
        .create_profile(repository, json!({"retention": {"keep_daily": 7}}))
        .await;
    let plan = env.create_plan(profile, agent.id).await;

    // Three backups run on the repository.
    let mut running = Vec::new();
    for _ in 0..3 {
        running.push(env.run_fresh_plan(repository, agent.id).await.to_string());
    }
    env.pass().await;
    let sent = agent.jobs_within(QUIET).await;
    assert_eq!(sent.len(), 3, "three backups start");
    for id in &running {
        agent.start(&env, id).await;
    }

    // A prune and a later backup queue up behind them.
    let prune = env
        .enqueue(JobKind::Prune, agent.id, repository, Some(plan))
        .await;
    let late_backup = env.run_fresh_plan(repository, agent.id).await;
    env.pass().await;
    assert!(agent.try_job(QUIET).await.is_none(), "the prune waits");

    // The prune starts only after the last backup ended.
    agent.finish(&env, &running[0]).await;
    agent.finish(&env, &running[1]).await;
    env.pass().await;
    assert!(
        agent.try_job(QUIET).await.is_none(),
        "one backup still runs"
    );
    agent.finish(&env, &running[2]).await;
    env.pass().await;
    let started = agent.jobs_within(QUIET).await;
    assert_eq!(started, vec![prune.to_string()], "the prune starts alone");

    // The backup that queued during the prune starts after the prune.
    agent.start(&env, &prune.to_string()).await;
    env.pass().await;
    assert!(
        agent.try_job(QUIET).await.is_none(),
        "the backup waits for the prune"
    );
    assert_eq!(env.job_status(late_backup).await, "queued");
    agent.finish(&env, &prune.to_string()).await;
    env.pass().await;
    assert_eq!(
        agent.jobs_within(QUIET).await,
        vec![late_backup.to_string()],
        "the backup starts after the prune"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_08_one_job_for_each_agent_never_runs_two_jobs_at_once() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut queue = test_queue();
    queue.agent_limit = 1;
    let env = Env::start(db, queue).await;
    let mut agent = env.connect_agent("limit-agent").await;
    let repository = env.create_repository(agent.id, "pw").await;
    for _ in 0..10 {
        env.run_fresh_plan(repository, agent.id).await;
    }

    let mut seen = 0;
    while seen < 10 {
        env.pass().await;
        let started = agent.jobs_within(Duration::from_millis(250)).await;
        assert_eq!(started.len(), 1, "the agent limit allows one job at a time");
        agent.start(&env, &started[0]).await;
        env.pass().await;
        assert!(
            agent.try_job(Duration::from_millis(100)).await.is_none(),
            "no second job starts while one runs"
        );
        agent.finish(&env, &started[0]).await;
        seen += 1;
    }
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_08_two_jobs_for_each_repository_is_the_most() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut queue = test_queue();
    queue.repository_limit = 2;
    let env = Env::start(db, queue).await;
    let mut agent = env.connect_agent("limit-repo").await;
    let repository = env.create_repository(agent.id, "pw").await;
    for _ in 0..10 {
        env.run_fresh_plan(repository, agent.id).await;
    }

    env.pass().await;
    let first = agent.jobs_within(QUIET).await;
    assert_eq!(first.len(), 2, "two backups start at once");
    env.pass().await;
    assert!(agent.try_job(Duration::from_millis(100)).await.is_none());

    agent.start(&env, &first[0]).await;
    agent.finish(&env, &first[0]).await;
    env.pass().await;
    assert_eq!(
        agent.jobs_within(QUIET).await.len(),
        1,
        "one slot is free again"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_08_the_global_limit_holds_across_agents() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut queue = test_queue();
    queue.global_limit = 3;
    let env = Env::start(db, queue).await;
    let mut first = env.connect_agent("global-a").await;
    let mut second = env.connect_agent("global-b").await;
    for agent in [&first.id, &second.id] {
        let repository = env.create_repository(*agent, "pw").await;
        for _ in 0..5 {
            env.run_fresh_plan(repository, *agent).await;
        }
    }
    env.pass().await;
    let total = first.jobs_within(QUIET).await.len() + second.jobs_within(QUIET).await.len();
    assert_eq!(total, 3, "the whole tenant runs three jobs at most");
    env.db.cleanup().await;
}
