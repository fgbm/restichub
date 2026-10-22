//! AC-SC-07: the missed run policy also applies to an offline agent (FR-SC-07).
//!
//! A host that is offline takes no job. With `latest_only` or `skip`, a newer scheduled job
//! replaces the older queued ones, so the host runs one backup when it returns.

#![allow(clippy::expect_used)]

mod common;

use restichub_cp::clock::ManualClock;
use restichub_cp::scheduler;
use restichub_db::test_util::TestDatabase;
use restichub_db::{
    AgentId, Database, JobId, JobKind, JobStatus, MissedRunPolicy, PlanId, ProfileId, RepositoryId,
    TenantId, jobs, plans,
};
use time::{Duration, Month, OffsetDateTime};

use common::{
    create_tenant_with_key, exec, plan_jobs, seed_agent, seed_profile, seed_repository,
    seed_scheduled_plan, seed_secret, utc,
};

struct Fixture {
    tenant: TenantId,
    agent: AgentId,
    plan: PlanId,
    profile: ProfileId,
    repository: RepositoryId,
}

async fn fixture(
    db: &TestDatabase,
    name: &str,
    policy: MissedRunPolicy,
    start: OffsetDateTime,
) -> Fixture {
    let database = db.database();
    let tenant = create_tenant_with_key(database, name).await;
    let secret = seed_secret(database, tenant, "s").await;
    let repository = seed_repository(database, tenant, "r", secret).await;
    let profile = seed_profile(database, tenant, "p").await;
    let agent = seed_agent(database, tenant, "h").await;
    let plan = seed_scheduled_plan(
        database,
        tenant,
        "hourly",
        profile,
        agent,
        "0 * * * *",
        "UTC",
        policy,
        Some(start),
        repository,
    )
    .await;
    Fixture {
        tenant,
        agent,
        plan,
        profile,
        repository,
    }
}

/// A plan that shares the profile, host and repository of `one`, so only the plan id tells the jobs apart.
async fn second_plan(
    db: &TestDatabase,
    one: &Fixture,
    policy: MissedRunPolicy,
    start: OffsetDateTime,
) -> PlanId {
    seed_scheduled_plan(
        db.database(),
        one.tenant,
        "second",
        one.profile,
        one.agent,
        "0 * * * *",
        "UTC",
        policy,
        Some(start),
        one.repository,
    )
    .await
}

/// One scheduler pass, five seconds after the fire time at `start + offset`.
async fn pass(database: &Database, start: OffsetDateTime, offset: Duration) {
    let clock = ManualClock::new(start);
    clock.advance(offset.whole_seconds() + 5);
    scheduler::run_once(database, &clock)
        .await
        .expect("the pass");
}

async fn statuses(
    db: &Database,
    tenant: TenantId,
    plan: PlanId,
) -> Vec<(OffsetDateTime, JobStatus)> {
    plan_jobs(db, tenant, plan)
        .await
        .into_iter()
        .filter_map(|job| Some((job.fire_time?, job.status)))
        .collect()
}

#[tokio::test]
async fn ac_sc_07_latest_only_expires_the_queued_jobs_of_an_offline_host() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let start = utc(2025, Month::January, 1, 0, 0);
    let one = fixture(&db, "offline", MissedRunPolicy::LatestOnly, start).await;
    // The replacement is per plan: the queue of another plan of the tenant stays whole.
    let other = second_plan(&db, &one, MissedRunPolicy::RunAll, start).await;

    // The host is offline for three fire times. The control plane runs on time each hour.
    pass(database, start, Duration::ZERO).await;
    pass(database, start, Duration::hours(1)).await;
    pass(database, start, Duration::hours(2)).await;

    // Only the latest job waits for the host; the older ones expired.
    assert_eq!(
        statuses(database, one.tenant, one.plan).await,
        vec![
            (start, JobStatus::Expired),
            (start + Duration::hours(1), JobStatus::Expired),
            (start + Duration::hours(2), JobStatus::Queued),
        ]
    );
    let kept = statuses(database, one.tenant, other).await;
    assert_eq!(kept.len(), 3);
    assert!(kept.iter().all(|(_, status)| *status == JobStatus::Queued));

    // A replaced job is not a failure: no failed job and no latest job of the plan counts for an
    // alert or for the dashboard (FR-AL-01).
    let mut tx = database
        .begin_tenant(one.tenant)
        .await
        .expect("cannot begin the transaction");
    let latest = restichub_db::dashboard::latest_jobs(&mut tx, start - Duration::days(1))
        .await
        .expect("cannot read the latest jobs");
    let failed =
        restichub_db::dashboard::recent_failed_jobs(&mut tx, start - Duration::days(1), 10)
            .await
            .expect("cannot read the failed jobs");
    tx.commit().await.expect("cannot commit the read");
    assert!(latest.is_empty(), "an expired job is nobody's latest job");
    assert!(failed.is_empty(), "a replaced job is not a failure");

    db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_07_a_pass_with_an_on_time_and_an_older_run_keeps_both_jobs() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let start = utc(2025, Month::January, 1, 0, 0);
    let one = fixture(&db, "downtime", MissedRunPolicy::LatestOnly, start).await;

    pass(database, start, Duration::ZERO).await;
    // The control plane was down for longer than the interval: the pass at 02:00:05 finds the
    // missed 01:00 run and the on-time 02:00 run.
    pass(database, start, Duration::hours(2)).await;

    assert_eq!(
        statuses(database, one.tenant, one.plan).await,
        vec![
            (start, JobStatus::Expired),
            (start + Duration::hours(1), JobStatus::Queued),
            (start + Duration::hours(2), JobStatus::Queued),
        ]
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_07_a_replacement_keeps_manual_sent_retried_and_run_all_jobs() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let start = utc(2025, Month::January, 1, 0, 0);
    let latest = fixture(&db, "keep", MissedRunPolicy::LatestOnly, start).await;
    let run_all = fixture(&db, "run-all", MissedRunPolicy::RunAll, start).await;
    let job_at = |hours: i64| {
        let fire_time = start + Duration::hours(hours);
        async move {
            plan_jobs(database, latest.tenant, latest.plan)
                .await
                .into_iter()
                .find(|job| job.fire_time == Some(fire_time))
                .expect("the job of the fire time")
                .id
        }
    };

    pass(database, start, Duration::ZERO).await;
    let retried = job_at(0).await;
    exec(
        &db,
        latest.tenant,
        "UPDATE jobs SET attempt = 1 WHERE id = $1",
        retried.as_uuid(),
    )
    .await;

    pass(database, start, Duration::hours(1)).await;
    let sent = job_at(1).await;
    let mut tx = database
        .begin_tenant(latest.tenant)
        .await
        .expect("cannot begin the transaction");
    assert!(
        jobs::mark_sent(&mut tx, sent, start + Duration::hours(1), 3600)
            .await
            .expect("cannot send the job")
    );
    tx.commit().await.expect("cannot commit the setup");

    pass(database, start, Duration::hours(2)).await;
    let delivered = job_at(2).await;
    exec(
        &db,
        latest.tenant,
        "UPDATE jobs SET deliveries = 1 WHERE id = $1",
        delivered.as_uuid(),
    )
    .await;

    pass(database, start, Duration::hours(3)).await;
    let mut tx = database
        .begin_tenant(latest.tenant)
        .await
        .expect("cannot begin the transaction");
    let manual = jobs::create(
        &mut tx,
        &jobs::NewJob {
            id: JobId::new(),
            kind: JobKind::Backup,
            agent_id: latest.agent,
            repository_id: None,
            plan_id: Some(latest.plan),
            fire_time: None,
            expires_at: None,
            location_fingerprint: None,
            created_by: None,
        },
    )
    .await
    .expect("cannot create the manual job");
    tx.commit().await.expect("cannot commit the setup");

    pass(database, start, Duration::hours(4)).await;

    let jobs_of_plan = plan_jobs(database, latest.tenant, latest.plan).await;
    let status_of = |id: JobId| {
        jobs_of_plan
            .iter()
            .find(|job| job.id == id)
            .map(|job| job.status)
    };
    assert_eq!(status_of(retried), Some(JobStatus::Queued));
    assert_eq!(status_of(sent), Some(JobStatus::Sent));
    assert_eq!(status_of(delivered), Some(JobStatus::Queued));
    assert_eq!(status_of(manual), Some(JobStatus::Queued));
    // The plain queued job is the control: the replacement ran.
    assert_eq!(status_of(job_at(3).await), Some(JobStatus::Expired));

    // Under this policy every queued job stays.
    let kept = statuses(database, run_all.tenant, run_all.plan).await;
    assert_eq!(kept.len(), 5);
    assert!(kept.iter().all(|(_, status)| *status == JobStatus::Queued));

    db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_07_a_replacement_never_reaches_the_plan_of_another_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let start = utc(2025, Month::January, 1, 0, 0);
    let one = fixture(&db, "tenant-a", MissedRunPolicy::LatestOnly, start).await;
    let other = fixture(&db, "tenant-b", MissedRunPolicy::LatestOnly, start).await;
    pass(database, start, Duration::ZERO).await;
    pass(database, start, Duration::hours(1)).await;

    // Tenant A names the plan of tenant B: Row-Level Security hides its jobs (ADR-0006).
    let mut tx = database
        .begin_tenant(one.tenant)
        .await
        .expect("cannot begin the transaction");
    let changed = jobs::expire_superseded(
        &mut tx,
        other.plan,
        start + Duration::days(10),
        start - Duration::days(10),
        start + Duration::days(10),
    )
    .await
    .expect("cannot run the replacement");
    tx.commit().await.expect("cannot commit the replacement");

    assert_eq!(changed, 0);
    assert_eq!(
        statuses(database, other.tenant, other.plan).await,
        vec![
            (start, JobStatus::Expired),
            (start + Duration::hours(1), JobStatus::Queued),
        ]
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_07_skip_replaces_the_queued_jobs_of_an_offline_host() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let start = utc(2025, Month::January, 1, 0, 0);
    let offline = fixture(&db, "skip-offline", MissedRunPolicy::Skip, start).await;

    // The host is offline for three fire times. The control plane runs on time each hour.
    pass(database, start, Duration::ZERO).await;
    pass(database, start, Duration::hours(1)).await;
    pass(database, start, Duration::hours(2)).await;

    let all = plan_jobs(database, offline.tenant, offline.plan).await;
    let newest = start + Duration::hours(2);
    let states: Vec<_> = all
        .iter()
        .filter_map(|job| Some((job.fire_time?, job.status)))
        .collect();
    assert_eq!(
        states,
        vec![
            (start, JobStatus::Expired),
            (start + Duration::hours(1), JobStatus::Expired),
            (newest, JobStatus::Queued),
        ]
    );
    // No job has a short deadline: a host that waits for a limit slot keeps its job.
    for job in &all {
        let fire_time = job.fire_time.expect("a scheduled job has a fire time");
        assert_eq!(job.expires_at, Some(fire_time + Duration::hours(24)));
    }

    // The control plane was down for longer than the interval: the missed runs are skipped and
    // the on-time run replaces the queued job.
    pass(database, start, Duration::hours(5)).await;
    assert_eq!(
        statuses(database, offline.tenant, offline.plan).await,
        vec![
            (start, JobStatus::Expired),
            (start + Duration::hours(1), JobStatus::Expired),
            (newest, JobStatus::Expired),
            (start + Duration::hours(5), JobStatus::Queued),
        ]
    );

    // The plan advances, so the fire time does not return.
    let mut tx = database
        .begin_tenant(offline.tenant)
        .await
        .expect("cannot begin the transaction");
    let stored = plans::get(&mut tx, offline.plan)
        .await
        .expect("cannot read the plan")
        .expect("the plan exists");
    tx.commit().await.expect("cannot commit the read");
    assert!(
        stored
            .next_run_at
            .is_some_and(|next| next > start + Duration::hours(5))
    );

    db.cleanup().await;
}
