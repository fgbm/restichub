//! AC-SC-02: the job creation is idempotent on `(plan_id, fire_time)` (FR-SC-02, ADR-0010).
//!
//! Two scheduler passes run on one database at the same instant, as two control plane instances
//! would. The unique key and `SELECT ... FOR UPDATE SKIP LOCKED` give exactly one job for each
//! `(plan_id, fire_time)` pair.

#![allow(clippy::expect_used)]

mod common;

use restichub_cp::clock::ManualClock;
use restichub_cp::scheduler;
use restichub_db::MissedRunPolicy;
use time::Month;

use common::{
    create_tenant_with_key, plan_fire_times, seconds_between, seed_agent, seed_profile,
    seed_repository, seed_scheduled_plan, seed_secret, utc,
};

#[tokio::test]
async fn ac_sc_02_two_instances_create_one_job_for_each_plan_and_fire_time() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "sched-two").await;
    let secret = seed_secret(database, tenant, "s").await;
    let repository = seed_repository(database, tenant, "r", secret).await;
    let profile = seed_profile(database, tenant, "p").await;
    let agent = seed_agent(database, tenant, "h").await;

    let start = utc(2025, Month::January, 1, 0, 0);
    let mut plans = Vec::new();
    for index in 0..20 {
        plans.push(
            seed_scheduled_plan(
                database,
                tenant,
                &format!("plan-{index}"),
                profile,
                agent,
                "0 * * * *",
                "UTC",
                MissedRunPolicy::RunAll,
                Some(start),
                repository,
            )
            .await,
        );
    }

    let clock = ManualClock::new(start);
    // Two hours later each plan missed the fire times at 00:00, 01:00 and 02:00.
    clock.advance(seconds_between(start, utc(2025, Month::January, 1, 2, 30)));

    let first = scheduler::run_once(database, &clock);
    let second = scheduler::run_once(database, &clock);
    tokio::try_join!(first, second).expect("the two passes");

    let expected = vec![
        start,
        start + time::Duration::hours(1),
        start + time::Duration::hours(2),
    ];
    for plan in &plans {
        assert_eq!(
            plan_fire_times(database, tenant, *plan).await,
            expected,
            "the plan {plan} holds one job for each fire time"
        );
    }

    // A further pass at the same instant creates nothing: the fire times are advanced.
    scheduler::run_once(database, &clock)
        .await
        .expect("the pass");
    for plan in &plans {
        assert_eq!(plan_fire_times(database, tenant, *plan).await.len(), 3);
    }

    db.cleanup().await;
}
