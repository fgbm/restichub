//! FR-SC-02: a plan whose scheduler pass fails does not stay due.
//!
//! A job insert that fails rolls back the pass of the plan. The plan moves to its next fire time and
//! keeps the cause, so it does not repeat on every pass and does not fill the claim of the tenant.

#![allow(clippy::expect_used)]

mod common;
mod schedule_failure_support;

use restichub_cp::clock::ManualClock;
use restichub_cp::scheduler;
use restichub_db::{AgentId, MissedRunPolicy, PlanId, RepositoryId, TenantId};
use time::{Duration, Month, OffsetDateTime};

use common::{
    create_tenant_with_key, plan_fire_times, seed_agent, seed_profile, seed_repository,
    seed_scheduled_plan, seed_secret, utc,
};
use schedule_failure_support::{due_rows, fail_jobs_of_agent, heal, owner_exec, plan_state};

struct Fixture {
    tenant: TenantId,
    profile: restichub_db::ProfileId,
    repository: RepositoryId,
    good_agent: AgentId,
    bad_agent: AgentId,
}

async fn fixture(db: &restichub_db::Database, name: &str) -> Fixture {
    let tenant = create_tenant_with_key(db, name).await;
    let secret = seed_secret(db, tenant, "s").await;
    Fixture {
        tenant,
        profile: seed_profile(db, tenant, "p").await,
        repository: seed_repository(db, tenant, "r", secret).await,
        good_agent: seed_agent(db, tenant, "good").await,
        bad_agent: seed_agent(db, tenant, "bad").await,
    }
}

async fn hourly_plan(
    db: &restichub_db::Database,
    fx: &Fixture,
    name: &str,
    agent: AgentId,
    next_run_at: OffsetDateTime,
) -> PlanId {
    seed_scheduled_plan(
        db,
        fx.tenant,
        name,
        fx.profile,
        agent,
        "0 * * * *",
        "UTC",
        MissedRunPolicy::RunAll,
        Some(next_run_at),
        fx.repository,
    )
    .await
}

#[tokio::test]
async fn fr_sc_02_a_failed_plan_moves_to_its_next_fire_time_and_keeps_the_cause() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let fx = fixture(database, "failed-plan").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    let plan = hourly_plan(database, &fx, "bad", fx.bad_agent, t0).await;
    fail_jobs_of_agent(&db, fx.bad_agent).await;
    let clock = ManualClock::new(t0);
    clock.advance(30);

    scheduler::run_once(database, &clock).await.expect("pass");
    let failed = plan_state(&db, fx.tenant, plan.as_uuid()).await;
    assert!(failed.enabled);
    assert_eq!(failed.next_run_at, Some(t0 + Duration::hours(1)));
    assert_eq!(
        failed.schedule_error.as_deref(),
        Some("database error: query")
    );
    assert!(plan_fire_times(database, fx.tenant, plan).await.is_empty());

    scheduler::run_once(database, &clock).await.expect("pass");
    assert_eq!(plan_state(&db, fx.tenant, plan.as_uuid()).await, failed);

    // The plan runs at its next fire time and the cause goes away.
    heal(&db).await;
    clock.advance(3600);
    scheduler::run_once(database, &clock).await.expect("pass");
    let healed = plan_state(&db, fx.tenant, plan.as_uuid()).await;
    assert_eq!(healed.schedule_error, None);
    assert_eq!(healed.next_run_at, Some(t0 + Duration::hours(2)));
    assert_eq!(
        plan_fire_times(database, fx.tenant, plan).await,
        vec![t0 + Duration::hours(1)]
    );
    db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_02_failed_plans_do_not_block_the_other_plans() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let fx = fixture(database, "failed-block").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    // More failed plans than one claim holds, all due before the good plan.
    for index in 0..205 {
        hourly_plan(database, &fx, &format!("bad-{index}"), fx.bad_agent, t0).await;
    }
    let good = hourly_plan(
        database,
        &fx,
        "good",
        fx.good_agent,
        t0 + Duration::seconds(10),
    )
    .await;
    fail_jobs_of_agent(&db, fx.bad_agent).await;
    let clock = ManualClock::new(t0);
    clock.advance(30);

    for _ in 0..2 {
        scheduler::run_once(database, &clock).await.expect("pass");
    }

    assert_eq!(
        plan_fire_times(database, fx.tenant, good).await,
        vec![t0 + Duration::seconds(10)]
    );
    assert_eq!(
        due_rows(&db, fx.tenant, "plans", clock_now(&clock)).await,
        0
    );
    db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_02_a_plan_with_an_invalid_schedule_is_disabled_with_the_cause() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let fx = fixture(database, "invalid-plan").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    let plan = hourly_plan(database, &fx, "broken", fx.good_agent, t0).await;
    owner_exec(
        &db,
        fx.tenant,
        "UPDATE plans SET cron = 'not a cron' WHERE id = $1",
        &[plan.as_uuid()],
    )
    .await;
    let clock = ManualClock::new(t0);
    clock.advance(30);

    scheduler::run_once(database, &clock).await.expect("pass");

    let state = plan_state(&db, fx.tenant, plan.as_uuid()).await;
    assert!(!state.enabled);
    assert_eq!(state.next_run_at, None);
    assert_eq!(
        state.schedule_error.as_deref(),
        Some("the cron expression is not valid")
    );
    assert_eq!(
        due_rows(&db, fx.tenant, "plans", clock_now(&clock)).await,
        0
    );
    db.cleanup().await;
}

#[tokio::test]
async fn fr_mn_05_a_failed_plan_shows_in_the_metrics() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let fx = fixture(database, "failed-metrics").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    let plan = hourly_plan(database, &fx, "bad", fx.bad_agent, t0).await;
    let other = hourly_plan(
        database,
        &fx,
        "good",
        fx.good_agent,
        t0 + Duration::hours(5),
    )
    .await;
    fail_jobs_of_agent(&db, fx.bad_agent).await;
    let clock = ManualClock::new(t0);
    clock.advance(30);
    scheduler::run_once(database, &clock).await.expect("pass");

    let mut tx = database.begin_tenant(fx.tenant).await.expect("begin");
    let snapshot = restichub_db::metrics::snapshot(&mut tx)
        .await
        .expect("snapshot");
    tx.commit().await.expect("commit");
    let body = restichub_cp::metrics::render(fx.tenant, &snapshot, |_| false);

    let tenant = fx.tenant;
    assert!(body.contains(&format!(
        "restichub_plan_schedule_failed{{tenant=\"{tenant}\",plan=\"{plan}\"}} 1"
    )));
    assert!(!body.contains(&format!("plan=\"{other}\"}} 1")));
    db.cleanup().await;
}

fn clock_now(clock: &ManualClock) -> OffsetDateTime {
    use restichub_cp::clock::Clock;
    clock.now()
}
