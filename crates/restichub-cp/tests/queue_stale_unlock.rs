//! The stale lock recovery on a repository that never goes idle (FR-SC-06, AC-SC-06).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use restichub_db::JobKind;
use restichub_proto::agent::v1::{JobStatus, StepStatus, step};
use serde_json::json;
use uuid::Uuid;

use queue_support::{
    Env, FakeAgent, decode_job_id, eventually, killed_backup, marks, repository_jobs, test_queue,
};

const QUIET: Duration = Duration::from_millis(300);

const PAST_THE_WAIT: i64 = 31 * 60;

/// The number of jobs of a kind on the repository, whatever their status.
async fn count_of(env: &Env, repository: Uuid, kind: &str) -> usize {
    repository_jobs(env, repository)
        .await
        .iter()
        .filter(|job| job["kind"] == kind)
        .count()
}

/// Fails the test unless the agent gets an `unlock` job next, and returns its identifier.
async fn expect_unlock(env: &Env, agent: &mut FakeAgent) -> String {
    let signed = agent.job().await;
    let job = env.verify(&signed, agent).await;
    assert_eq!(job.steps.len(), 1);
    assert!(
        matches!(job.steps[0].kind, Some(step::Kind::Unlock(_))),
        "the agent gets an unlock first: {job:?}"
    );
    decode_job_id(&signed)
}

/// Marks a job as a possible stale lock, as a second killed job would.
async fn mark_stale(env: &Env, job: Uuid, seconds_ago: i64) {
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE jobs SET stale_lock_since = $2 WHERE id = $1")
        .bind(job)
        .bind(env.now() - time::Duration::seconds(seconds_ago))
        .execute(&mut *tx)
        .await
        .expect("cannot mark the job");
    tx.commit().await.expect("cannot end the write");
}

#[tokio::test]
async fn ac_sc_06_a_repository_with_three_busy_plans_gets_the_unlock_before_the_queued_backups() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("busy-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let first = env.create_plan(profile, agent.id, repository).await;
    let second = env.create_plan(profile, agent.id, repository).await;
    let third = env.create_plan(profile, agent.id, repository).await;
    killed_backup(&env, repository, &mut agent, first).await;

    // The third plan waits for the lock, so its job goes to the retry chain.
    env.run_plan(third).await;
    env.pass().await;
    let waiting = decode_job_id(&agent.job().await);
    agent.start(&env, &waiting).await;
    agent.lock_held(&waiting).await;
    eventually(|| async { env.plan_jobs(third).await.len() == 2 }).await;
    let retry = env.plan_jobs(third).await.remove(0)["id"]
        .as_str()
        .expect("id")
        .to_owned();

    // The second plan keeps a backup running and the first plan queues the next one.
    env.run_plan(second).await;
    env.pass().await;
    let running = decode_job_id(&agent.job().await);
    agent.start(&env, &running).await;
    let queued = env.run_plan(first).await;

    // The repository is busy, and the unlock still comes, behind the barrier only.
    env.clock.advance(PAST_THE_WAIT);
    env.pass().await;
    assert!(
        agent.try_job(QUIET).await.is_none(),
        "the unlock waits for the running backup, and the queued backups wait for the unlock"
    );
    assert_eq!(count_of(&env, repository, "unlock").await, 1);
    assert_eq!(
        marks(&env, repository).await,
        1,
        "the mark stays until an unlock succeeds"
    );

    // The running backup ends. The unlock is newer than both queued backups and still goes first.
    agent.finish(&env, &running).await;
    env.pass().await;
    let unlock = expect_unlock(&env, &mut agent).await;
    assert!(
        agent.try_job(QUIET).await.is_none(),
        "no backup starts next to the unlock"
    );

    // The next backups, the retry included, succeed after the unlock.
    agent.start(&env, &unlock).await;
    agent.finish(&env, &unlock).await;
    env.pass().await;
    assert_eq!(
        marks(&env, repository).await,
        0,
        "a successful unlock clears the mark"
    );
    let mut started = agent.jobs_within(Duration::from_secs(2)).await;
    started.sort();
    let mut expected = vec![retry.clone(), queued.to_string()];
    expected.sort();
    assert_eq!(started, expected, "the waiting backups resume");
    for id in &started {
        agent.start(&env, id).await;
        agent.finish(&env, id).await;
    }
    assert_eq!(
        env.job_status(Uuid::parse_str(&retry).expect("job id"))
            .await,
        "succeeded"
    );
    assert_eq!(count_of(&env, repository, "unlock").await, 1);
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_06_no_unlock_while_an_exclusive_job_of_the_repository_runs() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("exclusive-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let other = env.create_repository(agent.id, "pw").await;
    let other_plan = env.create_plan(profile, agent.id, other).await;
    killed_backup(&env, repository, &mut agent, plan).await;
    killed_backup(&env, other, &mut agent, other_plan).await;

    let check = env
        .enqueue(JobKind::Check, agent.id, repository, None)
        .await;
    env.pass().await;
    let id = decode_job_id(&agent.job().await);
    assert_eq!(id, check.to_string());
    agent.start(&env, &id).await;

    env.clock.advance(PAST_THE_WAIT);
    env.run_plan(plan).await;
    env.pass().await;
    assert_eq!(
        count_of(&env, repository, "unlock").await,
        0,
        "no unlock next to a running check"
    );
    expect_unlock(&env, &mut agent).await;
    assert_eq!(
        count_of(&env, other, "unlock").await,
        1,
        "the other repository of the tenant gets its unlock in the same pass"
    );
    assert_eq!(marks(&env, repository).await, 1, "the mark stays");

    // The check ends without a success, so the lock is still a suspect.
    agent
        .result(&id, JobStatus::Failed, StepStatus::Failed)
        .await;
    eventually(|| async { env.job_status(check).await == "failed" }).await;
    env.pass().await;
    expect_unlock(&env, &mut agent).await;
    assert_eq!(count_of(&env, repository, "unlock").await, 1);
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_06_a_queued_or_running_unlock_gets_no_duplicate() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("duplicate-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let first = env.create_plan(profile, agent.id, repository).await;
    let second = env.create_plan(profile, agent.id, repository).await;
    let other = env.create_repository(agent.id, "pw").await;
    let other_plan = env.create_plan(profile, agent.id, other).await;
    let killed = killed_backup(&env, repository, &mut agent, first).await;
    killed_backup(&env, other, &mut agent, other_plan).await;

    env.run_plan(second).await;
    env.pass().await;
    let running = decode_job_id(&agent.job().await);
    agent.start(&env, &running).await;
    env.clock.advance(PAST_THE_WAIT);
    env.pass().await;
    assert_eq!(count_of(&env, repository, "unlock").await, 1, "queued");
    expect_unlock(&env, &mut agent).await;
    assert_eq!(
        count_of(&env, other, "unlock").await,
        1,
        "the other repository of the tenant gets its unlock in the same pass"
    );

    // Another job marks the lock while the unlock waits, and the passes run again.
    mark_stale(&env, killed, PAST_THE_WAIT).await;
    env.pass().await;
    env.pass().await;
    assert_eq!(
        count_of(&env, repository, "unlock").await,
        1,
        "still queued"
    );

    agent.finish(&env, &running).await;
    env.pass().await;
    let unlock = expect_unlock(&env, &mut agent).await;
    agent.start(&env, &unlock).await;
    mark_stale(&env, killed, PAST_THE_WAIT).await;
    env.pass().await;
    assert_eq!(count_of(&env, repository, "unlock").await, 1, "running");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_06_a_failed_unlock_is_repeated_after_the_wait_not_at_every_pass() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("failing-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    killed_backup(&env, repository, &mut agent, plan).await;

    env.clock.advance(PAST_THE_WAIT);
    env.pass().await;
    let first = expect_unlock(&env, &mut agent).await;
    agent.start(&env, &first).await;
    agent
        .result(&first, JobStatus::Failed, StepStatus::Failed)
        .await;
    let first_id = Uuid::parse_str(&first).expect("job id");
    eventually(|| async { env.job_status(first_id).await == "failed" }).await;

    env.pass().await;
    assert!(
        agent.try_job(QUIET).await.is_none(),
        "no new unlock right after the failure"
    );
    env.clock.advance(29 * 60);
    env.pass().await;
    assert_eq!(count_of(&env, repository, "unlock").await, 1);
    assert_eq!(marks(&env, repository).await, 1, "the mark stays");

    env.clock.advance(2 * 60);
    env.pass().await;
    let second = expect_unlock(&env, &mut agent).await;
    assert_ne!(first, second);
    assert_eq!(count_of(&env, repository, "unlock").await, 2);

    agent.start(&env, &second).await;
    agent.finish(&env, &second).await;
    env.pass().await;
    assert_eq!(marks(&env, repository).await, 0);
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_06_an_expired_unlock_is_repeated_after_the_wait() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("expiring-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let first = env.create_plan(profile, agent.id, repository).await;
    let second = env.create_plan(profile, agent.id, repository).await;
    killed_backup(&env, repository, &mut agent, first).await;

    // A long backup keeps the unlock in the queue until it expires.
    env.run_plan(second).await;
    env.pass().await;
    let running = decode_job_id(&agent.job().await);
    agent.start(&env, &running).await;
    env.clock.advance(PAST_THE_WAIT);
    env.pass().await;
    assert_eq!(count_of(&env, repository, "unlock").await, 1);

    env.clock.advance(24 * 60 * 60);
    restichub_cp::maintenance::expire_jobs(env.db.database(), env.now())
        .await
        .expect("the expiry sweep");
    env.pass().await;
    assert_eq!(
        count_of(&env, repository, "unlock").await,
        1,
        "no new unlock right after the expiry"
    );

    env.clock.advance(31 * 60);
    env.pass().await;
    assert_eq!(count_of(&env, repository, "unlock").await, 2);
    env.db.cleanup().await;
}

/// The dispatcher reads a bounded batch for each host and repository. The unlock is the newest job
/// of the pair, so only its place in the order keeps it inside the batch.
#[tokio::test]
async fn ac_sc_06_the_unlock_is_dispatched_first_behind_a_hundred_queued_backups() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("crowded-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    killed_backup(&env, repository, &mut agent, plan).await;

    let mut tx = env
        .db
        .database()
        .begin_tenant(env.tenant)
        .await
        .expect("cannot begin the transaction");
    for offset in 0..101 {
        restichub_db::jobs::create(
            &mut tx,
            &restichub_db::jobs::NewJob {
                id: restichub_db::JobId::new(),
                kind: JobKind::Backup,
                agent_id: agent.id,
                repository_id: Some(restichub_db::RepositoryId::from_uuid(repository)),
                plan_id: Some(restichub_db::PlanId::from_uuid(plan)),
                fire_time: Some(env.now() + time::Duration::seconds(offset)),
                expires_at: Some(env.now() + time::Duration::days(2)),
                location_fingerprint: None,
                created_by: None,
            },
        )
        .await
        .expect("cannot create the backup");
    }
    tx.commit().await.expect("cannot commit the backups");

    env.clock.advance(PAST_THE_WAIT);
    env.pass().await;
    expect_unlock(&env, &mut agent).await;
    assert!(
        agent.try_job(QUIET).await.is_none(),
        "no backup starts next to the unlock"
    );
    env.db.cleanup().await;
}
