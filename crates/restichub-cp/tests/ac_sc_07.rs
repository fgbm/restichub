//! AC-SC-07: the missed run policy decides the number of jobs after a downtime (FR-SC-07).
//!
//! The three policies create 3, 1 and 0 jobs for three missed intervals. A suspended tenant and a
//! plan whose agent is revoked create no jobs (AC-TN-03). Each job carries an expiry time, and the
//! expiry sweep moves an overdue job to the terminal `expired` status.

#![allow(clippy::expect_used)]

mod common;

use restichub_cp::clock::ManualClock;
use restichub_cp::scheduler;
use restichub_db::{JobStatus, MissedRunPolicy, agents, plans};

use common::{
    create_tenant_with_key, create_user, plan_fire_times, plan_jobs, seed_agent, seed_profile,
    seed_repository, seed_scheduled_plan, seed_secret, set_tenant_status, utc,
};
use time::{Duration, Month};

#[tokio::test]
async fn ac_sc_07_the_missed_run_policy_decides_the_job_count() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "missed").await;
    let secret = seed_secret(database, tenant, "s").await;
    let repository = seed_repository(database, tenant, "r", secret).await;
    let profile = seed_profile(database, tenant, "p").await;
    let agent = seed_agent(database, tenant, "h").await;

    let start = utc(2025, Month::January, 1, 0, 0);
    // The next fire time of each plan is 00:00; the control plane is down for two and a half
    // intervals.
    let run_all = seed_scheduled_plan(
        database,
        tenant,
        "run-all",
        profile,
        agent,
        "0 * * * *",
        "UTC",
        MissedRunPolicy::RunAll,
        Some(start),
        repository,
    )
    .await;
    let latest = seed_scheduled_plan(
        database,
        tenant,
        "latest",
        profile,
        agent,
        "0 * * * *",
        "UTC",
        MissedRunPolicy::LatestOnly,
        Some(start),
        repository,
    )
    .await;
    let skip = seed_scheduled_plan(
        database,
        tenant,
        "skip",
        profile,
        agent,
        "0 * * * *",
        "UTC",
        MissedRunPolicy::Skip,
        Some(start),
        repository,
    )
    .await;

    let clock = ManualClock::new(start);
    clock.advance(150 * 60);
    scheduler::run_once(database, &clock)
        .await
        .expect("the pass");

    // Run all: one job for each missed fire time (00:00, 01:00, 02:00).
    assert_eq!(
        plan_fire_times(database, tenant, run_all).await,
        vec![
            start,
            start + Duration::hours(1),
            start + Duration::hours(2),
        ]
    );
    // Run latest only: one job, for the latest missed fire time.
    assert_eq!(
        plan_fire_times(database, tenant, latest).await,
        vec![start + Duration::hours(2)]
    );
    // Skip: no job.
    assert!(plan_fire_times(database, tenant, skip).await.is_empty());

    // Every job holds an expiry time 24 h after its fire time (FR-SC-07).
    for job in plan_jobs(database, tenant, run_all).await {
        let fire_time = job.fire_time.expect("a scheduled job holds a fire time");
        assert_eq!(job.expires_at, Some(fire_time + Duration::hours(24)));
        assert_eq!(job.status, JobStatus::Queued);
    }

    // The expiry sweep moves an overdue job to `expired`.
    restichub_cp::maintenance::expire_jobs(database, start + Duration::hours(48))
        .await
        .expect("the expiry sweep");
    for job in plan_jobs(database, tenant, run_all).await {
        assert_eq!(job.status, JobStatus::Expired);
    }

    db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_07_a_suspended_tenant_creates_no_jobs() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "suspended").await;
    let secret = seed_secret(database, tenant, "s").await;
    let repository = seed_repository(database, tenant, "r", secret).await;
    let profile = seed_profile(database, tenant, "p").await;
    let agent = seed_agent(database, tenant, "h").await;

    let start = utc(2025, Month::January, 1, 0, 0);
    let plan = seed_scheduled_plan(
        database,
        tenant,
        "plan",
        profile,
        agent,
        "0 * * * *",
        "UTC",
        MissedRunPolicy::RunAll,
        Some(start),
        repository,
    )
    .await;
    // The tenant is suspended before the fire time.
    set_tenant_status(database, tenant, "suspended").await;

    let clock = ManualClock::new(start);
    clock.advance(150 * 60);
    scheduler::run_once(database, &clock)
        .await
        .expect("the pass");

    assert!(plan_jobs(database, tenant, plan).await.is_empty());

    db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_07_a_revoked_agent_creates_no_jobs() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "revoked").await;
    let secret = seed_secret(database, tenant, "s").await;
    let repository = seed_repository(database, tenant, "r", secret).await;
    let profile = seed_profile(database, tenant, "p").await;
    let agent = seed_agent(database, tenant, "h").await;

    let start = utc(2025, Month::January, 1, 0, 0);
    let plan = seed_scheduled_plan(
        database,
        tenant,
        "plan",
        profile,
        agent,
        "0 * * * *",
        "UTC",
        MissedRunPolicy::RunAll,
        Some(start),
        repository,
    )
    .await;

    // The agent is revoked before the fire time, so the plan creates no job.
    let actor = create_user(database, tenant, "admin", "phc", "admin").await;
    let mut tx = database
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    agents::revoke(&mut tx, agent, actor, start)
        .await
        .expect("cannot revoke the agent");
    tx.commit().await.expect("cannot commit the revoke");

    let clock = ManualClock::new(start);
    clock.advance(150 * 60);
    scheduler::run_once(database, &clock)
        .await
        .expect("the pass");

    assert!(plan_jobs(database, tenant, plan).await.is_empty());
    // The plan still advances, so the revoked agent does not stall it.
    let mut tx = database
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let stored = plans::get(&mut tx, plan)
        .await
        .expect("cannot read the plan")
        .expect("the plan exists");
    tx.commit().await.expect("cannot commit the read");
    assert!(
        stored
            .next_run_at
            .is_some_and(|next| next > start + Duration::hours(2)),
        "the plan advanced past the missed fire times"
    );

    db.cleanup().await;
}

/// A fire time within the grace window of the pass is on time and always runs, whatever the missed
/// run policy: the policy applies only to the older fire times (FR-SC-07). AC-SC-07 counts the jobs
/// for the fire times that the pass missed, so the on-time run is in addition to that count.
#[tokio::test]
async fn ac_sc_07_an_on_time_run_creates_one_job_for_every_policy() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "on-time").await;
    let secret = seed_secret(database, tenant, "s").await;
    let repository = seed_repository(database, tenant, "r", secret).await;
    let profile = seed_profile(database, tenant, "p").await;
    let agent = seed_agent(database, tenant, "h").await;

    let start = utc(2025, Month::January, 1, 0, 0);
    let mut plan_list = Vec::new();
    for (name, policy) in [
        ("run-all", MissedRunPolicy::RunAll),
        ("latest", MissedRunPolicy::LatestOnly),
        ("skip", MissedRunPolicy::Skip),
    ] {
        plan_list.push(
            seed_scheduled_plan(
                database,
                tenant,
                name,
                profile,
                agent,
                "0 * * * *",
                "UTC",
                policy,
                Some(start),
                repository,
            )
            .await,
        );
    }

    // The pass runs five seconds after the fire time: within the grace window.
    let clock = ManualClock::new(start);
    clock.advance(5);
    scheduler::run_once(database, &clock)
        .await
        .expect("the pass");

    for plan in plan_list {
        assert_eq!(
            plan_fire_times(database, tenant, plan).await,
            vec![start],
            "an on-time run creates one job for every policy"
        );
    }

    db.cleanup().await;
}

/// After a 72 hour downtime the run_all policy creates a job only for the fire times that a job
/// still outlives (24 h lifetime): the latest ones, at most 100 (FR-SC-07).
#[tokio::test]
async fn ac_sc_07_run_all_keeps_the_latest_unexpired_fire_times() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "downtime").await;
    let secret = seed_secret(database, tenant, "s").await;
    let repository = seed_repository(database, tenant, "r", secret).await;
    let profile = seed_profile(database, tenant, "p").await;
    let agent = seed_agent(database, tenant, "h").await;

    let start = utc(2025, Month::January, 1, 0, 0);
    let plan = seed_scheduled_plan(
        database,
        tenant,
        "hourly",
        profile,
        agent,
        "0 * * * *",
        "UTC",
        MissedRunPolicy::RunAll,
        Some(start),
        repository,
    )
    .await;

    let clock = ManualClock::new(start);
    clock.advance(72 * 3600);
    scheduler::run_once(database, &clock)
        .await
        .expect("the pass");

    let jobs = plan_jobs(database, tenant, plan).await;
    assert!(jobs.len() <= 100, "the cap bounds one pass");
    let now = start + Duration::hours(72);
    for job in &jobs {
        let fire_time = job.fire_time.expect("a scheduled job holds a fire time");
        assert!(
            fire_time + Duration::hours(24) > now,
            "no job is created for an expired fire time"
        );
    }
    // The 24 hour lifetime keeps the latest unexpired hourly fire times: 49:00 through 72:00. The
    // fire time at 48:00 is expired because its job would end exactly at now.
    let fires = plan_fire_times(database, tenant, plan).await;
    assert_eq!(fires.len(), 24);
    assert_eq!(fires.first(), Some(&(start + Duration::hours(49))));
    assert_eq!(fires.last(), Some(&now));

    db.cleanup().await;
}

/// One failing plan does not roll back the whole tenant pass: each plan runs in its own savepoint,
/// so the other due plans of the tenant still create their jobs (AC-SC-02).
#[tokio::test]
async fn ac_sc_02_a_failing_plan_does_not_roll_back_the_tenant_pass() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "isolation").await;
    let secret = seed_secret(database, tenant, "s").await;
    let repository = seed_repository(database, tenant, "r", secret).await;
    let profile = seed_profile(database, tenant, "p").await;
    let agent = seed_agent(database, tenant, "h").await;

    let start = utc(2025, Month::January, 1, 0, 0);
    // The first plan holds a cron that names no instant, so the scheduler fails on it. It must not
    // stop the pass: the second plan still creates its job.
    let impossible = seed_scheduled_plan(
        database,
        tenant,
        "impossible",
        profile,
        agent,
        "0 0 31 2 *",
        "UTC",
        MissedRunPolicy::RunAll,
        Some(start),
        repository,
    )
    .await;
    let good = seed_scheduled_plan(
        database,
        tenant,
        "good",
        profile,
        agent,
        "0 * * * *",
        "UTC",
        MissedRunPolicy::RunAll,
        Some(start),
        repository,
    )
    .await;

    let clock = ManualClock::new(start);
    clock.advance(5);
    scheduler::run_once(database, &clock)
        .await
        .expect("the pass");

    assert_eq!(
        plan_fire_times(database, tenant, good).await,
        vec![start],
        "the good plan creates its job"
    );
    assert!(
        plan_fire_times(database, tenant, impossible)
            .await
            .is_empty(),
        "the failing plan creates no job"
    );

    db.cleanup().await;
}
