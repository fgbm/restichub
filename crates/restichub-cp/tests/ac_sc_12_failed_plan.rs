//! AC-SC-12: a plan whose scheduler pass fails keeps a typed failure state (FR-SC-12).
//!
//! A database error keeps the fire time and pauses the plan, so the missed run policy decides after
//! the recovery. A schedule that the control plane cannot compute waits for an edit. The `enabled`
//! switch of the user never changes.

#![allow(clippy::expect_used)]

mod common;
mod schedule_failure_support;

use restichub_cp::clock::{Clock, ManualClock};
use restichub_cp::scheduler;
use restichub_db::{AgentId, MissedRunPolicy, PlanId, RepositoryId, TenantId};
use time::{Duration, Month, OffsetDateTime};

use common::{
    create_tenant_with_key, plan_fire_times, seed_agent, seed_profile, seed_repository,
    seed_scheduled_plan, seed_secret, utc,
};
use schedule_failure_support::{
    claimable_rows, fail_jobs_of_agent, failure_audit, failure_audit_diff, heal, owner_exec,
    plan_state,
};

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
    policy: MissedRunPolicy,
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
        policy,
        Some(next_run_at),
        fx.repository,
    )
    .await
}

#[tokio::test]
async fn ac_sc_12_a_database_error_keeps_the_fire_time_and_pauses_the_plan() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let fx = fixture(database, "failed-plan").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    let plan = hourly_plan(
        database,
        &fx,
        "bad",
        fx.bad_agent,
        MissedRunPolicy::RunAll,
        t0,
    )
    .await;
    fail_jobs_of_agent(&db, fx.bad_agent).await;
    let clock = ManualClock::new(t0);
    clock.advance(30);
    let first_pass = clock.now();

    scheduler::run_once(database, &clock).await.expect("pass");
    let failed = plan_state(&db, fx.tenant, plan.as_uuid()).await;
    assert!(failed.enabled, "the system never switches a plan off");
    assert_eq!(failed.next_run_at, Some(t0), "the fire time stays");
    assert_eq!(failed.reason.as_deref(), Some("database_error"));
    assert_eq!(failed.failed_since, Some(first_pass));
    assert_eq!(failed.retry_after, Some(first_pass + Duration::seconds(5)));
    assert_eq!(failed.attempts, 1);
    assert!(plan_fire_times(database, fx.tenant, plan).await.is_empty());

    // The pause keeps the plan out of the claim: the next pass changes nothing.
    scheduler::run_once(database, &clock).await.expect("pass");
    assert_eq!(plan_state(&db, fx.tenant, plan.as_uuid()).await, failed);

    clock.advance(5);
    scheduler::run_once(database, &clock).await.expect("pass");
    let second = plan_state(&db, fx.tenant, plan.as_uuid()).await;
    assert_eq!(second.attempts, 2);
    assert_eq!(second.failed_since, Some(first_pass));
    assert_eq!(
        second.retry_after,
        Some(clock.now() + Duration::seconds(10))
    );
    assert_eq!(second.next_run_at, Some(t0));

    // One record names the entry into the state, not one for each try.
    assert_eq!(
        failure_audit(&db, fx.tenant, plan.as_uuid()).await,
        vec!["schedule_failed"]
    );
    let diff = failure_audit_diff(&db, fx.tenant, plan.as_uuid(), "schedule_failed").await;
    assert!(diff.to_string().contains("database_error"), "{diff}");

    // After the recovery the plan runs for its original fire time and the state goes away.
    heal(&db).await;
    clock.advance(10);
    scheduler::run_once(database, &clock).await.expect("pass");
    let healed = plan_state(&db, fx.tenant, plan.as_uuid()).await;
    assert_eq!(healed.reason, None);
    assert_eq!(healed.failed_since, None);
    assert_eq!(healed.retry_after, None);
    assert_eq!(healed.attempts, 0);
    assert_eq!(healed.next_run_at, Some(t0 + Duration::hours(1)));
    assert_eq!(plan_fire_times(database, fx.tenant, plan).await, vec![t0]);
    assert_eq!(
        failure_audit(&db, fx.tenant, plan.as_uuid()).await,
        vec!["schedule_failed", "schedule_recovered"]
    );
    db.cleanup().await;
}

/// The plan stays failed for hours, then recovers. The policy of the plan decides which of the
/// missed fire times run (FR-SC-07).
async fn fire_times_after_a_long_failure(
    db: restichub_db::test_util::TestDatabase,
    policy: MissedRunPolicy,
) -> Vec<OffsetDateTime> {
    let database = db.database();
    let fx = fixture(database, "failed-policy").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    let plan = hourly_plan(database, &fx, "bad", fx.bad_agent, policy, t0).await;
    fail_jobs_of_agent(&db, fx.bad_agent).await;
    let clock = ManualClock::new(t0);
    clock.advance(30);
    scheduler::run_once(database, &clock).await.expect("pass");
    assert_eq!(
        plan_state(&db, fx.tenant, plan.as_uuid())
            .await
            .reason
            .as_deref(),
        Some("database_error")
    );

    heal(&db).await;
    clock.set(t0 + Duration::hours(3) + Duration::minutes(30));
    scheduler::run_once(database, &clock).await.expect("pass");
    let fire_times = plan_fire_times(database, fx.tenant, plan).await;
    db.cleanup().await;
    fire_times
}

#[tokio::test]
async fn ac_sc_12_run_all_creates_every_missed_job_after_the_recovery() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let t0 = utc(2025, Month::January, 1, 0, 0);
    assert_eq!(
        fire_times_after_a_long_failure(db, MissedRunPolicy::RunAll).await,
        (0..4)
            .map(|hour| t0 + Duration::hours(hour))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn ac_sc_12_latest_only_creates_one_job_after_the_recovery() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let t0 = utc(2025, Month::January, 1, 0, 0);
    assert_eq!(
        fire_times_after_a_long_failure(db, MissedRunPolicy::LatestOnly).await,
        vec![t0 + Duration::hours(3)]
    );
}

#[tokio::test]
async fn ac_sc_12_an_invalid_schedule_keeps_the_plan_enabled_and_waits_for_an_edit() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let fx = fixture(database, "invalid-plan").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    let broken_cron = hourly_plan(
        database,
        &fx,
        "broken-cron",
        fx.good_agent,
        MissedRunPolicy::RunAll,
        t0,
    )
    .await;
    owner_exec(
        &db,
        fx.tenant,
        "UPDATE plans SET cron = 'not a cron' WHERE id = $1",
        &[broken_cron.as_uuid()],
    )
    .await;
    let broken_zone = hourly_plan(
        database,
        &fx,
        "broken-zone",
        fx.good_agent,
        MissedRunPolicy::RunAll,
        t0,
    )
    .await;
    owner_exec(
        &db,
        fx.tenant,
        "UPDATE plans SET time_zone = 'Mars/Olympus' WHERE id = $1",
        &[broken_zone.as_uuid()],
    )
    .await;
    let clock = ManualClock::new(t0);
    clock.advance(30);

    scheduler::run_once(database, &clock).await.expect("pass");

    for (plan, reason) in [
        (broken_cron, "invalid_cron"),
        (broken_zone, "unknown_time_zone"),
    ] {
        let state = plan_state(&db, fx.tenant, plan.as_uuid()).await;
        assert!(
            state.enabled,
            "{reason}: the system never switches a plan off"
        );
        assert_eq!(state.next_run_at, None, "{reason}");
        assert_eq!(state.reason.as_deref(), Some(reason));
        assert_eq!(state.retry_after, None, "{reason}: only an edit helps");
        assert_eq!(state.attempts, 1);
        assert_eq!(
            failure_audit(&db, fx.tenant, plan.as_uuid()).await,
            vec!["schedule_failed"]
        );

        // The plan has no fire time, so the next pass does not touch it.
        scheduler::run_once(database, &clock).await.expect("pass");
        assert_eq!(plan_state(&db, fx.tenant, plan.as_uuid()).await, state);
    }
    assert_eq!(
        claimable_rows(&db, fx.tenant, "plans", clock.now()).await,
        0
    );
    db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_12_failed_plans_do_not_block_the_other_plans() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let fx = fixture(database, "failed-block").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    // More failed plans than one claim holds, all due before the good plan.
    for index in 0..205 {
        hourly_plan(
            database,
            &fx,
            &format!("bad-{index}"),
            fx.bad_agent,
            MissedRunPolicy::RunAll,
            t0,
        )
        .await;
    }
    let good = hourly_plan(
        database,
        &fx,
        "good",
        fx.good_agent,
        MissedRunPolicy::RunAll,
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
        claimable_rows(&db, fx.tenant, "plans", clock.now()).await,
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
    let plan = hourly_plan(
        database,
        &fx,
        "bad",
        fx.bad_agent,
        MissedRunPolicy::RunAll,
        t0,
    )
    .await;
    let other = hourly_plan(
        database,
        &fx,
        "good",
        fx.good_agent,
        MissedRunPolicy::RunAll,
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
