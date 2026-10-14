//! AC-SC-01: the cron schedule with a time zone and the daylight saving rule of the accepted
//! decision (FR-SC-01).
//!
//! The scheduler reads the next fire time from the plan. The clock is injectable, so the test
//! advances it over a daylight saving change without a sleep. A fire time in the spring gap runs
//! once, right after the clock change; a fire time in the autumn overlap runs once, at its first
//! occurrence.

#![allow(clippy::expect_used)]

mod common;

use restichub_cp::clock::ManualClock;
use restichub_cp::schedule::Schedule;
use restichub_cp::scheduler;
use restichub_db::{Database, MissedRunPolicy, TenantId};
use time::{Month, OffsetDateTime};

use common::{
    create_tenant_with_key, plan_fire_times, seconds_between, seed_agent, seed_profile,
    seed_repository, seed_scheduled_plan, seed_secret, utc,
};

/// A profile and an agent of a fresh tenant, for a plan.
async fn fixture(
    db: &Database,
    name: &str,
) -> (TenantId, restichub_db::ProfileId, restichub_db::AgentId) {
    let tenant = create_tenant_with_key(db, name).await;
    let secret = seed_secret(db, tenant, "s").await;
    let repository = seed_repository(db, tenant, "r", secret).await;
    let profile = seed_profile(db, tenant, "p", repository).await;
    let agent = seed_agent(db, tenant, "h").await;
    (tenant, profile, agent)
}

/// Creates a plan whose next fire time is the first after `start`, and returns it.
#[allow(clippy::too_many_arguments)]
async fn plan_at(
    db: &Database,
    tenant: TenantId,
    profile: restichub_db::ProfileId,
    agent: restichub_db::AgentId,
    name: &str,
    cron: &str,
    time_zone: &str,
    start: OffsetDateTime,
) -> restichub_db::PlanId {
    let schedule = Schedule::parse(cron, time_zone).expect("the schedule parses");
    let next = schedule.next_after(start).expect("a next fire time");
    seed_scheduled_plan(
        db,
        tenant,
        name,
        profile,
        agent,
        cron,
        time_zone,
        MissedRunPolicy::RunAll,
        Some(next),
    )
    .await
}

/// Europe/Moscow does not observe daylight saving time, so `0 2 * * *` runs at 02:00 local, which is
/// 23:00 UTC of the day before (AC-SC-01).
#[tokio::test]
async fn ac_sc_01_moscow_runs_at_the_local_wall_clock() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let (tenant, profile, agent) = fixture(database, "dst-moscow").await;

    let start = utc(2025, Month::June, 1, 0, 0);
    let plan = plan_at(
        database,
        tenant,
        profile,
        agent,
        "moscow",
        "0 2 * * *",
        "Europe/Moscow",
        start,
    )
    .await;

    let clock = ManualClock::new(start);
    // Before the fire time the pass creates no job.
    scheduler::run_once(database, &clock)
        .await
        .expect("the pass");
    assert!(plan_fire_times(database, tenant, plan).await.is_empty());

    clock.advance(seconds_between(start, utc(2025, Month::June, 1, 23, 0)));
    scheduler::run_once(database, &clock)
        .await
        .expect("the pass");
    assert_eq!(
        plan_fire_times(database, tenant, plan).await,
        vec![utc(2025, Month::June, 1, 23, 0)]
    );

    db.cleanup().await;
}

/// The spring change in Europe/Berlin puts 02:00 to 03:00 on 2025-03-30 into a gap. The plan
/// `30 2 * * *` runs once on each day, and the run of the change day is right after the change
/// (FR-SC-01).
#[tokio::test]
async fn ac_sc_01_the_spring_change_gives_one_run_right_after_the_clock_change() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let (tenant, profile, agent) = fixture(database, "dst-spring").await;

    let start = utc(2025, Month::March, 29, 12, 0);
    let plan = plan_at(
        database,
        tenant,
        profile,
        agent,
        "spring",
        "30 2 * * *",
        "Europe/Berlin",
        start,
    )
    .await;

    let clock = ManualClock::new(start);
    clock.advance(seconds_between(start, utc(2025, Month::April, 1, 12, 0)));
    scheduler::run_once(database, &clock)
        .await
        .expect("the pass");

    let fires = plan_fire_times(database, tenant, plan).await;
    // One run for each day from 2025-03-30 to 2025-04-01. In the gap the run is at the first instant
    // after the change, 03:00 local (01:00 UTC).
    assert_eq!(
        fires,
        vec![
            utc(2025, Month::March, 30, 1, 0),
            utc(2025, Month::March, 31, 0, 30),
            utc(2025, Month::April, 1, 0, 30),
        ]
    );
    assert_eq!(
        fires
            .iter()
            .filter(|fire| fire.date() == utc(2025, Month::March, 30, 0, 0).date())
            .count(),
        1,
        "exactly one run on the spring change day"
    );

    db.cleanup().await;
}

/// The autumn change puts 02:00 to 03:00 on 2025-10-26 into an overlap. The plan `30 2 * * *` runs
/// once on each day, and the run of the change day is the first occurrence (FR-SC-01).
#[tokio::test]
async fn ac_sc_01_the_autumn_change_gives_one_run_at_the_first_occurrence() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let (tenant, profile, agent) = fixture(database, "dst-autumn").await;

    let start = utc(2025, Month::October, 25, 12, 0);
    let plan = plan_at(
        database,
        tenant,
        profile,
        agent,
        "autumn",
        "30 2 * * *",
        "Europe/Berlin",
        start,
    )
    .await;

    let clock = ManualClock::new(start);
    clock.advance(seconds_between(start, utc(2025, Month::October, 27, 12, 0)));
    scheduler::run_once(database, &clock)
        .await
        .expect("the pass");

    let fires = plan_fire_times(database, tenant, plan).await;
    // On 2025-10-26 the run is at the first occurrence, 02:30 CEST (00:30 UTC). The next day is
    // 2025-10-27, at 02:30 CET (01:30 UTC).
    assert_eq!(
        fires,
        vec![
            utc(2025, Month::October, 26, 0, 30),
            utc(2025, Month::October, 27, 1, 30),
        ]
    );
    assert_eq!(
        fires
            .iter()
            .filter(|fire| fire.date() == utc(2025, Month::October, 26, 0, 0).date())
            .count(),
        1,
        "exactly one run on the autumn change day"
    );

    db.cleanup().await;
}
