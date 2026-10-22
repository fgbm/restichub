//! AC-SC-07: the missed run policy also applies to an offline agent (FR-SC-07).
//!
//! A host that is offline takes no job. With `latest_only`, a newer scheduled job replaces the older
//! queued ones, so the host runs one backup when it returns. With `skip`, an offline host gets no
//! job.

#![allow(clippy::expect_used)]

mod common;

use restichub_cp::clock::ManualClock;
use restichub_cp::scheduler;
use restichub_db::test_util::TestDatabase;
use restichub_db::{
    AgentId, Database, JobKind, JobStatus, MissedRunPolicy, PlanId, TenantId, jobs, plans,
};
use time::{Duration, Month, OffsetDateTime};

use common::{
    create_tenant_with_key, plan_jobs, seed_agent, seed_profile, seed_repository,
    seed_scheduled_plan, seed_secret, set_agent_online, utc,
};

struct Fixture {
    tenant: TenantId,
    agent: AgentId,
    plan: PlanId,
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
    }
}

/// Runs one scheduler pass at `start + offset`.
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
    // Another tenant with the same plan keeps its queue: the replacement is per tenant (ADR-0006).
    let other = fixture(&db, "other-tenant", MissedRunPolicy::LatestOnly, start).await;

    // The host is offline for three fire times. The control plane runs on time each hour.
    pass(database, start, Duration::ZERO).await;
    pass(database, start, Duration::hours(1)).await;
    pass(database, start, Duration::hours(2)).await;

    for fixture in [&one, &other] {
        let state = statuses(database, fixture.tenant, fixture.plan).await;
        // Only the latest job waits for the host; the older ones expired.
        assert_eq!(
            state,
            vec![
                (start, JobStatus::Expired),
                (start + Duration::hours(1), JobStatus::Expired),
                (start + Duration::hours(2), JobStatus::Queued),
            ]
        );
    }

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
async fn ac_sc_07_a_replacement_keeps_manual_sent_and_run_all_jobs() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let start = utc(2025, Month::January, 1, 0, 0);
    let latest = fixture(&db, "keep", MissedRunPolicy::LatestOnly, start).await;
    let run_all = fixture(&db, "run-all", MissedRunPolicy::RunAll, start).await;

    pass(database, start, Duration::ZERO).await;
    pass(database, start, Duration::hours(1)).await;

    // A manual job of the plan (no fire time) and a job that the queue already sent.
    let mut tx = database
        .begin_tenant(latest.tenant)
        .await
        .expect("cannot begin the transaction");
    let manual = jobs::create(
        &mut tx,
        &jobs::NewJob {
            id: restichub_db::JobId::new(),
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
    let sent_id = plan_jobs(database, latest.tenant, latest.plan)
        .await
        .into_iter()
        .find(|job| job.fire_time == Some(start + Duration::hours(1)))
        .expect("the job of the second fire time")
        .id;
    assert!(
        jobs::mark_sent(&mut tx, sent_id, start + Duration::hours(1), 3600)
            .await
            .expect("cannot send the job")
    );
    tx.commit().await.expect("cannot commit the setup");

    pass(database, start, Duration::hours(2)).await;

    let jobs_of_plan = plan_jobs(database, latest.tenant, latest.plan).await;
    let status_of = |id: restichub_db::JobId| {
        jobs_of_plan
            .iter()
            .find(|job| job.id == id)
            .map(|job| job.status)
    };
    assert_eq!(status_of(manual), Some(JobStatus::Queued));
    assert_eq!(status_of(sent_id), Some(JobStatus::Sent));
    let first = jobs_of_plan
        .iter()
        .find(|job| job.fire_time == Some(start))
        .expect("the first scheduled job");
    assert_eq!(first.status, JobStatus::Expired);

    // The policy run_all keeps every queued job.
    let kept = statuses(database, run_all.tenant, run_all.plan).await;
    assert_eq!(kept.len(), 3);
    assert!(kept.iter().all(|(_, status)| *status == JobStatus::Queued));

    db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_07_skip_queues_no_job_for_an_offline_host() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let start = utc(2025, Month::January, 1, 0, 0);
    let offline = fixture(&db, "skip-offline", MissedRunPolicy::Skip, start).await;
    let online = fixture(&db, "skip-online", MissedRunPolicy::Skip, start).await;
    set_agent_online(
        &db,
        online.tenant,
        online.agent,
        start + Duration::seconds(3),
    )
    .await;

    pass(database, start, Duration::ZERO).await;

    assert!(
        plan_jobs(database, offline.tenant, offline.plan)
            .await
            .is_empty(),
        "an offline host queues nothing under skip"
    );
    assert_eq!(
        statuses(database, online.tenant, online.plan).await,
        vec![(start, JobStatus::Queued)],
        "an online host runs its on-time fire time"
    );

    // The plan still advances, so the skipped fire time does not return.
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
            .is_some_and(|next| next > start + Duration::minutes(1))
    );

    db.cleanup().await;
}
