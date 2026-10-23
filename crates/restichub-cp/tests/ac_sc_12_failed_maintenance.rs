//! AC-SC-12: a maintenance schedule whose scheduler pass fails keeps a typed failure state
//! (FR-SC-12), as a plan does.

#![allow(clippy::expect_used)]

mod common;
mod schedule_failure_support;

use restichub_cp::clock::{Clock, ManualClock};
use restichub_cp::scheduler;
use restichub_db::maintenance::{self, NewMaintenanceSchedule};
use restichub_db::{AgentId, Database, MaintenanceTask, RepositoryId, SecretId, TenantId};
use time::{Duration, Month, OffsetDateTime};

use common::{create_tenant_with_key, seed_agent, seed_repository, seed_secret, utc};
use schedule_failure_support::{
    arm_repository, check_schedule_state, claimable_rows, fail_jobs_of_agent, failure_audit,
    failure_audit_diff, heal, owner_exec, repository_jobs,
};

const MONTHLY_CHECK: &str = "0 4 1 * *";

/// A repository with an initialized maintenance agent and a check schedule.
async fn repository_with_check(
    db: &restichub_db::test_util::TestDatabase,
    tenant: TenantId,
    secret: SecretId,
    name: &str,
    agent: AgentId,
    cron: &str,
    next_run_at: Option<OffsetDateTime>,
) -> RepositoryId {
    let database: &Database = db.database();
    let repository = seed_repository(database, tenant, name, secret).await;
    arm_repository(db, tenant, repository, agent).await;
    let mut tx = database.begin_tenant(tenant).await.expect("begin");
    maintenance::create(
        &mut tx,
        &NewMaintenanceSchedule {
            repository_id: repository,
            task: MaintenanceTask::Check,
            cron,
            time_zone: "UTC",
            enabled: true,
            next_run_at,
        },
    )
    .await
    .expect("cannot create the schedule");
    tx.commit().await.expect("commit");
    repository
}

async fn check_schedule_id(
    db: &restichub_db::test_util::TestDatabase,
    tenant: TenantId,
    repository: RepositoryId,
) -> uuid::Uuid {
    let mut tx = schedule_failure_support::owner_tx(db, tenant).await;
    let id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM maintenance_schedules WHERE repository_id = $1 AND task = 'check'",
    )
    .bind(repository.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("cannot read the schedule");
    tx.commit().await.expect("commit");
    id
}

#[tokio::test]
async fn ac_sc_12_a_database_error_keeps_the_fire_time_of_a_maintenance_schedule() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "failed-maintenance").await;
    let secret = seed_secret(database, tenant, "s").await;
    let good_agent = seed_agent(database, tenant, "good").await;
    let bad_agent = seed_agent(database, tenant, "bad").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    let bad = repository_with_check(
        &db,
        tenant,
        secret,
        "bad",
        bad_agent,
        MONTHLY_CHECK,
        Some(t0),
    )
    .await;
    let good = repository_with_check(
        &db,
        tenant,
        secret,
        "good",
        good_agent,
        MONTHLY_CHECK,
        Some(t0),
    )
    .await;
    let schedule = check_schedule_id(&db, tenant, bad).await;
    fail_jobs_of_agent(&db, bad_agent).await;
    let clock = ManualClock::new(t0);
    clock.advance(30);
    let first_pass = clock.now();

    scheduler::run_once(database, &clock).await.expect("pass");
    let failed = check_schedule_state(&db, tenant, bad).await;
    assert!(failed.enabled, "the system never switches a schedule off");
    assert_eq!(failed.next_run_at, Some(t0), "the fire time stays");
    assert_eq!(failed.reason.as_deref(), Some("database_error"));
    assert_eq!(failed.failed_since, Some(first_pass));
    assert_eq!(failed.retry_after, Some(first_pass + Duration::seconds(5)));
    assert_eq!(failed.attempts, 1);
    assert_eq!(repository_jobs(&db, tenant, bad).await, 0);
    assert_eq!(repository_jobs(&db, tenant, good).await, 1);

    // The pause keeps the schedule out of the claim.
    scheduler::run_once(database, &clock).await.expect("pass");
    assert_eq!(check_schedule_state(&db, tenant, bad).await, failed);

    clock.advance(5);
    scheduler::run_once(database, &clock).await.expect("pass");
    let second = check_schedule_state(&db, tenant, bad).await;
    assert_eq!(second.attempts, 2);
    assert_eq!(
        second.retry_after,
        Some(clock.now() + Duration::seconds(10))
    );
    assert_eq!(
        failure_audit(&db, tenant, schedule).await,
        vec!["schedule_failed"]
    );
    let diff = failure_audit_diff(&db, tenant, schedule, "schedule_failed").await;
    let text = diff.to_string();
    assert!(
        text.contains("database_error") && text.contains("check"),
        "{diff}"
    );

    // After the recovery the schedule runs for its original fire time.
    heal(&db).await;
    clock.advance(10);
    scheduler::run_once(database, &clock).await.expect("pass");
    let healed = check_schedule_state(&db, tenant, bad).await;
    assert_eq!(healed.reason, None);
    assert_eq!(healed.retry_after, None);
    assert_eq!(healed.attempts, 0);
    assert_eq!(repository_jobs(&db, tenant, bad).await, 1);
    assert_eq!(
        failure_audit(&db, tenant, schedule).await,
        vec!["schedule_failed", "schedule_recovered"]
    );
    db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_12_failed_maintenance_schedules_do_not_block_the_other_schedules() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "failed-maintenance-block").await;
    let secret = seed_secret(database, tenant, "s").await;
    let good_agent = seed_agent(database, tenant, "good").await;
    let bad_agent = seed_agent(database, tenant, "bad").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    // More failed schedules than one claim holds, all due before the good schedule.
    for index in 0..205 {
        repository_with_check(
            &db,
            tenant,
            secret,
            &format!("bad-{index}"),
            bad_agent,
            MONTHLY_CHECK,
            Some(t0),
        )
        .await;
    }
    let good = repository_with_check(
        &db,
        tenant,
        secret,
        "good",
        good_agent,
        MONTHLY_CHECK,
        Some(t0 + Duration::seconds(10)),
    )
    .await;
    fail_jobs_of_agent(&db, bad_agent).await;
    let clock = ManualClock::new(t0);
    clock.advance(30);

    for _ in 0..2 {
        scheduler::run_once(database, &clock).await.expect("pass");
    }

    assert_eq!(repository_jobs(&db, tenant, good).await, 1);
    assert_eq!(
        claimable_rows(&db, tenant, "maintenance_schedules", clock.now()).await,
        0
    );
    db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_12_an_invalid_maintenance_cron_keeps_the_schedule_enabled() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "invalid-maintenance").await;
    let secret = seed_secret(database, tenant, "s").await;
    let agent = seed_agent(database, tenant, "a").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    let due =
        repository_with_check(&db, tenant, secret, "due", agent, "not a cron", Some(t0)).await;
    let unarmed =
        repository_with_check(&db, tenant, secret, "unarmed", agent, "not a cron", None).await;
    let clock = ManualClock::new(t0);
    clock.advance(30);

    scheduler::run_once(database, &clock).await.expect("pass");

    for repository in [due, unarmed] {
        let state = check_schedule_state(&db, tenant, repository).await;
        assert!(state.enabled, "the system never switches a schedule off");
        assert_eq!(state.next_run_at, None);
        assert_eq!(state.reason.as_deref(), Some("invalid_cron"));
        assert_eq!(state.retry_after, None, "only an edit helps");
        let schedule = check_schedule_id(&db, tenant, repository).await;
        assert_eq!(
            failure_audit(&db, tenant, schedule).await,
            vec!["schedule_failed"]
        );

        // The failed schedule has no fire time and the claim of unarmed rows skips it.
        scheduler::run_once(database, &clock).await.expect("pass");
        assert_eq!(check_schedule_state(&db, tenant, repository).await, state);
    }
    db.cleanup().await;
}

#[tokio::test]
async fn fr_mn_05_a_failed_maintenance_schedule_shows_in_the_metrics() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "failed-maintenance-metrics").await;
    let secret = seed_secret(database, tenant, "s").await;
    let agent = seed_agent(database, tenant, "a").await;
    let t0 = utc(2025, Month::January, 1, 0, 0);
    let broken = repository_with_check(
        &db,
        tenant,
        secret,
        "broken",
        agent,
        MONTHLY_CHECK,
        Some(t0),
    )
    .await;
    owner_exec(
        &db,
        tenant,
        "UPDATE maintenance_schedules SET cron = 'not a cron' WHERE repository_id = $1 AND task = 'check'",
        &[broken.as_uuid()],
    )
    .await;
    let clock = ManualClock::new(t0);
    clock.advance(30);
    scheduler::run_once(database, &clock).await.expect("pass");

    let mut tx = database.begin_tenant(tenant).await.expect("begin");
    let snapshot = restichub_db::metrics::snapshot(&mut tx)
        .await
        .expect("snapshot");
    tx.commit().await.expect("commit");
    let body = restichub_cp::metrics::render(tenant, &snapshot, |_| false);

    assert!(body.contains(&format!(
        "restichub_maintenance_schedule_failed{{tenant=\"{tenant}\",repository=\"{broken}\",task=\"check\"}} 1"
    )));
    db.cleanup().await;
}
