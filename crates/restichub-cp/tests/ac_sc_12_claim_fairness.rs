//! AC-SC-12: failing rows never delay a healthy plan or maintenance schedule beyond the first
//! pass that finds it due (FR-SC-12). The passes run at the cadence of the scheduler.

#![allow(clippy::expect_used)]

mod common;
mod schedule_failure_support;

use restichub_cp::clock::{Clock, ManualClock};
use restichub_cp::scheduler;
use restichub_db::maintenance::{self, NewMaintenanceSchedule};
use restichub_db::{AgentId, MaintenanceTask, MissedRunPolicy, RepositoryId, SecretId, TenantId};
use time::{Duration, Month, OffsetDateTime};

use common::{
    create_tenant_with_key, plan_fire_times, seed_agent, seed_profile, seed_repository,
    seed_scheduled_plan, seed_secret, utc,
};
use schedule_failure_support::{arm_repository, fail_jobs_of_agent, repository_jobs};

const PASS_STEP: i64 = 5;

/// AC-SC-12: 600 failing plans, all due since the start, and one healthy plan that falls due
/// between two passes. The first pass after the due time creates its job.
#[tokio::test]
async fn ac_sc_12_many_failing_plans_do_not_delay_a_healthy_plan() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "claim-fairness").await;
    let secret = seed_secret(database, tenant, "s").await;
    let profile = seed_profile(database, tenant, "p").await;
    let repository = seed_repository(database, tenant, "r", secret).await;
    let good_agent = seed_agent(database, tenant, "good").await;
    let bad_agent = seed_agent(database, tenant, "bad").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    for index in 0..600 {
        seed_scheduled_plan(
            database,
            tenant,
            &format!("bad-{index}"),
            profile,
            bad_agent,
            "0 * * * *",
            "UTC",
            MissedRunPolicy::Skip,
            Some(t0),
            repository,
        )
        .await;
    }
    let due = t0 + Duration::seconds(12);
    let good = seed_scheduled_plan(
        database,
        tenant,
        "good",
        profile,
        good_agent,
        "0 * * * *",
        "UTC",
        MissedRunPolicy::Skip,
        Some(due),
        repository,
    )
    .await;
    fail_jobs_of_agent(&db, bad_agent).await;
    let clock = ManualClock::new(t0);

    for pass in 0..=3 {
        scheduler::run_once(database, &clock).await.expect("pass");
        let fired = plan_fire_times(database, tenant, good).await;
        if clock.now() < due {
            assert!(
                fired.is_empty(),
                "pass {pass} runs the plan before it is due"
            );
        } else {
            assert_eq!(
                fired,
                vec![due],
                "pass {pass} is the first after the due time"
            );
        }
        clock.advance(PASS_STEP);
    }
    db.cleanup().await;
}

async fn check_schedule(
    db: &restichub_db::test_util::TestDatabase,
    tenant: TenantId,
    secret: SecretId,
    name: &str,
    agent: AgentId,
    next_run_at: OffsetDateTime,
) -> RepositoryId {
    let database = db.database();
    let repository = seed_repository(database, tenant, name, secret).await;
    arm_repository(db, tenant, repository, agent).await;
    let mut tx = database.begin_tenant(tenant).await.expect("begin");
    maintenance::create(
        &mut tx,
        &NewMaintenanceSchedule {
            repository_id: repository,
            task: MaintenanceTask::Check,
            cron: "0 4 1 * *",
            time_zone: "UTC",
            enabled: true,
            next_run_at: Some(next_run_at),
        },
    )
    .await
    .expect("cannot create the schedule");
    tx.commit().await.expect("commit");
    repository
}

/// AC-SC-12: 250 failing maintenance schedules and one healthy schedule that falls due between
/// two passes. The first pass after the due time creates its job.
#[tokio::test]
async fn ac_sc_12_many_failing_maintenance_schedules_do_not_delay_a_healthy_schedule() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "claim-fairness-maintenance").await;
    let secret = seed_secret(database, tenant, "s").await;
    let good_agent = seed_agent(database, tenant, "good").await;
    let bad_agent = seed_agent(database, tenant, "bad").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    for index in 0..250 {
        check_schedule(&db, tenant, secret, &format!("bad-{index}"), bad_agent, t0).await;
    }
    let due = t0 + Duration::seconds(3);
    let good = check_schedule(&db, tenant, secret, "good", good_agent, due).await;
    fail_jobs_of_agent(&db, bad_agent).await;
    let clock = ManualClock::new(t0);

    scheduler::run_once(database, &clock).await.expect("pass");
    assert_eq!(repository_jobs(&db, tenant, good).await, 0);
    clock.advance(PASS_STEP);
    scheduler::run_once(database, &clock).await.expect("pass");
    assert_eq!(repository_jobs(&db, tenant, good).await, 1);
    db.cleanup().await;
}
