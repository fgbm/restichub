//! AC-SC-12: a failed schedule does not mute the RPO alert, and a removed creator membership does
//! not stop the scheduler (FR-SC-12).

#![allow(clippy::expect_used)]

mod alert_support;
mod common;
mod schedule_failure_support;

use alert_support::{judged_view, local_sender, plan_created_at};
use restichub_cp::alerts;
use restichub_cp::clock::ManualClock;
use restichub_cp::config::TenancyConfig;
use restichub_cp::scheduler;
use restichub_cp::server::{AppState, router};
use restichub_db::plans::{NewPlan, create};
use restichub_db::{MissedRunPolicy, PlanId};
use serde_json::Value;
use time::{Duration, Month};

use common::{
    create_tenant_with_key, create_user, plan_fire_times, request, seed_agent,
    seed_memberless_account, seed_profile, seed_repository, seed_scheduled_plan, seed_secret,
    seed_session, send, test_kek, utc,
};
use schedule_failure_support::{
    fail_jobs_of_agent, failure_audit, owner_exec, owner_tx, plan_state,
};

/// AC-SC-12: a plan that cannot run keeps its RPO alert. The plan stays enabled, so the alert rule
/// of ADR-0036 still judges it.
#[tokio::test]
async fn ac_sc_12_a_failing_plan_keeps_the_rpo_alert_open() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "failure-rpo").await;
    let app = router(
        AppState::new(database.clone())
            .with_kek(test_kek())
            .with_tenancy(TenancyConfig::multi())
            .with_sender(local_sender(None)),
    );
    let cookie = seed_session(database, tenant, "admin", "admin").await;
    let secret = seed_secret(database, tenant, "rpo.password").await;
    let repository = seed_repository(database, tenant, "rpo-repo", secret).await;
    let profile = seed_profile(database, tenant, "rpo-profile").await;
    let agent = seed_agent(database, tenant, "rpo-host").await;
    let plan = seed_scheduled_plan(
        database,
        tenant,
        "nightly",
        profile,
        agent,
        "0 2 * * *",
        "UTC",
        MissedRunPolicy::RunAll,
        None,
        repository,
    )
    .await;
    let t0 = plan_created_at(&db, tenant, plan).await;
    fail_jobs_of_agent(&db, agent).await;
    // The plan is due one minute after its creation, and the job insert fails on every pass.
    set_next_run_at(&db, tenant, plan, t0 + Duration::minutes(1)).await;
    let clock = ManualClock::new(t0 + Duration::minutes(2));
    scheduler::run_once(database, &clock).await.expect("pass");
    let state = plan_state(&db, tenant, plan.as_uuid()).await;
    assert_eq!(state.reason.as_deref(), Some("database_error"));
    assert!(state.enabled);

    // The objective of a daily plan is 26 hours. The plan never succeeded, so it violates it.
    let late = t0 + Duration::hours(27);
    for now in [late, late + Duration::minutes(10)] {
        let kek = test_kek();
        let sender = local_sender(None);
        let clock = ManualClock::new(now).shared();
        alerts::run_once(database, &sender, kek, clock, judged_view(&db, now).await)
            .await
            .expect("the alert pass works");
        let (status, _, body) = send(
            &app,
            request("GET", "/api/v1/alerts?status=all", Some(&cookie), None),
        )
        .await;
        assert!(status.is_success(), "{body}");
        let open: Vec<&Value> = body["items"]
            .as_array()
            .expect("items")
            .iter()
            .filter(|alert| alert["kind"] == "rpo_violated" && alert["status"] == "open")
            .collect();
        assert_eq!(open.len(), 1, "the alert stays open at {now}: {body}");
    }
    assert!(plan_state(&db, tenant, plan.as_uuid()).await.enabled);
    db.cleanup().await;
}

async fn set_next_run_at(
    db: &restichub_db::test_util::TestDatabase,
    tenant: restichub_db::TenantId,
    plan: PlanId,
    at: time::OffsetDateTime,
) {
    let mut tx = owner_tx(db, tenant).await;
    sqlx::query("UPDATE plans SET next_run_at = $2 WHERE id = $1")
        .bind(plan.as_uuid())
        .bind(at)
        .execute(&mut *tx)
        .await
        .expect("cannot set the fire time");
    tx.commit().await.expect("commit");
}

/// AC-SC-12: the scheduler advances a plan and records a failure for a plan whose creator lost the
/// membership, and it does not repeat a failed pass at once.
#[tokio::test]
async fn ac_sc_12_a_removed_creator_membership_does_not_stop_the_scheduler() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "failure-creator").await;
    let secret = seed_secret(database, tenant, "creator.password").await;
    let repository = seed_repository(database, tenant, "creator-repo", secret).await;
    let profile = seed_profile(database, tenant, "creator-profile").await;
    let good_agent = seed_agent(database, tenant, "good").await;
    let bad_agent = seed_agent(database, tenant, "bad").await;
    let creator = create_user(database, tenant, "leaver", "phc", "operator").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    let mut plans = Vec::new();
    for (name, agent) in [("good", good_agent), ("bad", bad_agent)] {
        let plan = seed_scheduled_plan(
            database,
            tenant,
            name,
            profile,
            agent,
            "0 * * * *",
            "UTC",
            MissedRunPolicy::RunAll,
            Some(t0),
            repository,
        )
        .await;
        owner_exec(
            &db,
            tenant,
            "UPDATE plans SET created_by = $2 WHERE id = $1",
            &[plan.as_uuid(), creator.as_uuid()],
        )
        .await;
        plans.push(plan);
    }
    owner_exec(
        &db,
        tenant,
        "DELETE FROM memberships WHERE user_id = $1",
        &[creator.as_uuid()],
    )
    .await;
    fail_jobs_of_agent(&db, bad_agent).await;
    let clock = ManualClock::new(t0);
    clock.advance(30);

    scheduler::run_once(database, &clock).await.expect("pass");

    let (good, bad) = (plans[0], plans[1]);
    assert_eq!(plan_fire_times(database, tenant, good).await, vec![t0]);
    let advanced = plan_state(&db, tenant, good.as_uuid()).await;
    assert_eq!(advanced.next_run_at, Some(t0 + Duration::hours(1)));
    let failed = plan_state(&db, tenant, bad.as_uuid()).await;
    assert_eq!(failed.reason.as_deref(), Some("database_error"));
    assert_eq!(failed.attempts, 1);
    assert_eq!(
        failure_audit(&db, tenant, bad.as_uuid()).await,
        vec!["schedule_failed"]
    );

    // A paused row is not claimed again at once, so the second pass changes nothing.
    scheduler::run_once(database, &clock).await.expect("pass");
    assert_eq!(plan_state(&db, tenant, bad.as_uuid()).await, failed);
    assert_eq!(plan_state(&db, tenant, good.as_uuid()).await, advanced);
    db.cleanup().await;
}

/// AC-SC-12: the creator check still runs on an insert and on a change of the creator.
#[tokio::test]
async fn ac_sc_12_the_creator_check_still_runs_on_insert_and_on_a_change_of_the_creator() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "failure-trigger").await;
    let secret = seed_secret(database, tenant, "trigger.password").await;
    let repository = seed_repository(database, tenant, "trigger-repo", secret).await;
    let profile = seed_profile(database, tenant, "trigger-profile").await;
    let agent = seed_agent(database, tenant, "host").await;
    let outsider = seed_memberless_account(&db, "outsider", "phc").await;
    let plan = seed_scheduled_plan(
        database,
        tenant,
        "plan",
        profile,
        agent,
        "0 * * * *",
        "UTC",
        MissedRunPolicy::RunAll,
        None,
        repository,
    )
    .await;

    let mut tx = database.begin_tenant(tenant).await.expect("begin");
    let error = create(
        &mut tx,
        &NewPlan {
            id: PlanId::new(),
            name: "outsider-plan",
            profile_id: profile,
            agent_id: agent,
            repository_id: repository,
            cron: "0 * * * *",
            time_zone: "UTC",
            missed_run_policy: MissedRunPolicy::RunAll,
            enabled: true,
            rpo_seconds: None,
            next_run_at: None,
            created_by: Some(outsider),
        },
    )
    .await
    .expect_err("an insert for a creator without a membership fails");
    assert_eq!(error.sqlstate().as_deref(), Some("23503"), "{error}");
    drop(tx);

    let mut tx = owner_tx(&db, tenant).await;
    let error = sqlx::query("UPDATE plans SET created_by = $2 WHERE id = $1")
        .bind(plan.as_uuid())
        .bind(outsider.as_uuid())
        .execute(&mut *tx)
        .await
        .expect_err("a change to a creator without a membership fails");
    let message = error.to_string();
    assert!(message.contains("holds no membership"), "{message}");
    tx.rollback().await.expect("rollback");
    db.cleanup().await;
}
