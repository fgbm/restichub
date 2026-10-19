//! The locks that keep a plan complete under two writers (FR-PF-11, ADR-0047).
//!
//! Every write that can change the completeness of a plan takes one advisory lock of the tenant
//! before any row lock. Each test takes that lock in one transaction, starts the request of the
//! other writer, proves that the request waits for the advisory lock and then checks the answer
//! after the commit. A pair of profile saves that share a host completes, because the lock
//! serializes them instead of making them wait for each other in a cycle.

#![allow(clippy::expect_used)]

mod common;
mod parameters_support;

use std::time::Duration;

use axum::http::StatusCode;
use restichub_db::TenantId;
use restichub_db::test_util::TestDatabase;
use serde_json::json;

use common::{app_with_kek, create_tenant_with_key, seed_agent, seed_session};
use parameters_support::{
    create_plan, create_profile, plan_view, put_host_values, repository_fixture, update_profile,
};

/// A profile save that makes a parameter required holds the advisory lock, so a change of the host
/// values waits for it and then finds a plan without a value (FR-PF-11).
#[tokio::test]
async fn fr_pf_11_a_host_change_waits_for_a_profile_save_and_then_fails() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let profile = create_profile(
        &app,
        &cookie,
        json!({
            "name": "pg",
            "paths": {"all": ["/data"]},
            "parameters": [{"name": "pg_dir", "kind": "text"}],
        }),
    )
    .await;
    let (status, body) = create_plan(
        &app,
        &cookie,
        "nightly",
        &profile,
        agent,
        &repository,
        json!([]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // The profile save of another transaction requires the parameter under the advisory lock.
    let mut writer = scoped(&db, tenant).await;
    lock_parameters(&mut writer, tenant).await;
    sqlx::query("UPDATE profiles SET parameters = $2::jsonb WHERE id = $1")
        .bind(uuid::Uuid::parse_str(&profile).expect("the profile identifier"))
        .bind(r#"[{"name": "pg_dir", "kind": "text", "required": true}]"#)
        .execute(&mut *writer)
        .await
        .expect("cannot change the declarations");

    let (app_for_request, cookie_for_request) = (app.clone(), cookie.clone());
    let mut request_task = tokio::spawn(async move {
        put_host_values(&app_for_request, &cookie_for_request, agent, json!([])).await
    });
    wait_for_advisory_waiter(&db, &mut request_task).await;
    writer
        .commit()
        .await
        .expect("cannot commit the transaction");

    let (status, body) = request_task.await.expect("the request ended");
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["reason"], "parameter_missing_in_plans");
    assert_eq!(body["parameters"], json!(["pg_dir"]));

    db.cleanup().await;
}

/// A change of the host values holds the advisory lock, so a plan save that drops the override waits
/// for it and then finds a plan without a value (FR-PF-11).
#[tokio::test]
async fn fr_pf_11_a_plan_save_waits_for_a_host_change_and_then_fails() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let profile = create_profile(
        &app,
        &cookie,
        json!({
            "name": "pg",
            "paths": {"all": ["/data"]},
            "parameters": [{"name": "pg_dir", "kind": "text", "required": true}],
        }),
    )
    .await;
    let (status, body) = put_host_values(
        &app,
        &cookie,
        agent,
        json!([{"name": "pg_dir", "kind": "text", "value": "/host"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = create_plan(
        &app,
        &cookie,
        "nightly",
        &profile,
        agent,
        &repository,
        json!([{"name": "pg_dir", "kind": "text", "value": "/plan"}]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let plan = body["plan"]["id"]
        .as_str()
        .expect("the identifier")
        .to_owned();

    // The host change of another transaction removes the value under the advisory lock.
    let mut writer = scoped(&db, tenant).await;
    lock_parameters(&mut writer, tenant).await;
    sqlx::query("DELETE FROM agent_parameter_values WHERE agent_id = $1")
        .bind(agent.as_uuid())
        .execute(&mut *writer)
        .await
        .expect("cannot remove the values");

    let (app_for_request, cookie_for_request, profile_for_request) =
        (app.clone(), cookie.clone(), profile.clone());
    let repository_for_request = repository.clone();
    let plan_for_request = plan.clone();
    let mut request_task = tokio::spawn(async move {
        parameters_support::update_plan(
            &app_for_request,
            &cookie_for_request,
            &plan_for_request,
            &profile_for_request,
            agent,
            &repository_for_request,
            json!([]),
        )
        .await
    });
    wait_for_advisory_waiter(&db, &mut request_task).await;
    writer
        .commit()
        .await
        .expect("cannot commit the transaction");

    let (status, body) = request_task.await.expect("the request ended");
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["reason"], "parameter_missing");
    assert_eq!(body["parameters"], json!(["pg_dir"]));
    // The refused save kept the override, so the plan still runs with a value.
    assert_eq!(
        plan_view(&app, &cookie, &plan).await["parameter_values"][0]["value"],
        "/plan"
    );

    db.cleanup().await;
}

/// Two profile saves that share a host both succeed: the advisory lock serializes them, so neither
/// waits for a row that the other one holds (FR-PF-11).
#[tokio::test]
async fn fr_pf_11_two_profile_saves_of_one_host_both_succeed() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let first = create_profile(
        &app,
        &cookie,
        json!({
            "name": "first",
            "paths": {"all": ["/data"]},
            "parameters": [{"name": "pg_dir", "kind": "text", "default_value": "/a"}],
        }),
    )
    .await;
    let second = create_profile(
        &app,
        &cookie,
        json!({
            "name": "second",
            "paths": {"all": ["/other"]},
            "parameters": [{"name": "pg_dir", "kind": "text", "default_value": "/b"}],
        }),
    )
    .await;
    for (name, profile) in [("first", &first), ("second", &second)] {
        let (status, body) =
            create_plan(&app, &cookie, name, profile, agent, &repository, json!([])).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    // Both saves change the declarations of a profile of the same host at the same time.
    let body_of = |default: &str| {
        json!({
            "paths": {"all": ["/data"]},
            "parameters": [{"name": "pg_dir", "kind": "text", "default_value": default}],
        })
    };
    let first_task = {
        let (app, cookie, first) = (app.clone(), cookie.clone(), first.clone());
        let body = body_of("/a2");
        tokio::spawn(async move { update_profile(&app, &cookie, &first, body).await })
    };
    let second_task = {
        let (app, cookie, second) = (app.clone(), cookie.clone(), second.clone());
        let body = body_of("/b2");
        tokio::spawn(async move { update_profile(&app, &cookie, &second, body).await })
    };
    let (first_result, second_result) = tokio::join!(first_task, second_task);
    for (status, body) in [
        first_result.expect("the first request ended"),
        second_result.expect("the second request ended"),
    ] {
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    db.cleanup().await;
}

/// Begins a transaction with the tenant scope, as a handler does.
async fn scoped(db: &TestDatabase, tenant: TenantId) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("cannot begin the transaction");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    tx
}

/// Takes the advisory lock of the parameter writes, as a handler does.
async fn lock_parameters(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, tenant: TenantId) {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1 || $2::text, 0))")
        .bind(restichub_cp::parameters::PARAMETERS_LOCK)
        .bind(tenant.to_string())
        .execute(&mut **tx)
        .await
        .expect("cannot take the advisory lock");
}

/// Waits until a backend waits for the advisory lock of the write, so the test reads no timing.
async fn wait_for_advisory_waiter(
    db: &TestDatabase,
    request: &mut tokio::task::JoinHandle<(StatusCode, serde_json::Value)>,
) {
    for _ in 0..400 {
        assert!(
            !request.is_finished(),
            "the request ended before it waited for the advisory lock"
        );
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND NOT granted",
        )
        .fetch_one(db.pool())
        .await
        .expect("cannot read the locks");
        if waiting > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("no backend waited for the advisory lock");
}
