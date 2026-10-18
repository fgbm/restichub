//! The locks that keep a plan complete under two writers (FR-PF-11, ADR-0047).
//!
//! A change of the host values and a profile save meet on the profile row; a change of the host
//! values and a plan save meet on the host row. Each test holds the lock of one writer, starts the
//! request of the other writer and proves that it waits for the commit and then fails.

#![allow(clippy::expect_used)]

mod common;
mod parameters_support;

use std::time::Duration;

use axum::http::StatusCode;
use serde_json::json;

use common::{app_with_kek, create_tenant_with_key, seed_agent, seed_session};
use parameters_support::{
    create_plan, create_profile, plan_view, put_host_values, repository_fixture,
};

/// A profile save that makes a parameter required holds the profile row, so a change of the host
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

    // The profile save of another transaction requires the parameter and holds the profile row.
    let mut writer = scoped(&db, tenant).await;
    lock_profile(&mut writer, profile.as_str()).await;
    sqlx::query("UPDATE profiles SET parameters = $2::jsonb WHERE id = $1")
        .bind(uuid::Uuid::parse_str(&profile).expect("the profile identifier"))
        .bind(r#"[{"name": "pg_dir", "kind": "text", "required": true}]"#)
        .execute(&mut *writer)
        .await
        .expect("cannot change the declarations");

    let (app_for_request, cookie_for_request) = (app.clone(), cookie.clone());
    let request_task = tokio::spawn(async move {
        put_host_values(&app_for_request, &cookie_for_request, agent, json!([])).await
    });
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

/// A change of the host values holds the host row, so a plan save that drops the override waits for
/// it and then finds a plan without a value (FR-PF-11).
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

    // The host change of another transaction takes the host row and removes the value.
    let mut writer = scoped(&db, tenant).await;
    lock_agent(&mut writer, agent).await;
    sqlx::query("DELETE FROM agent_parameter_values WHERE agent_id = $1")
        .bind(agent.as_uuid())
        .execute(&mut *writer)
        .await
        .expect("cannot remove the values");

    let (app_for_request, cookie_for_request, profile_for_request) =
        (app.clone(), cookie.clone(), profile.clone());
    let repository_for_request = repository.clone();
    let plan_for_request = plan.clone();
    let request_task = tokio::spawn(async move {
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

/// Begins a transaction with the tenant scope, as a handler does.
async fn scoped(
    db: &restichub_db::test_util::TestDatabase,
    tenant: restichub_db::TenantId,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
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

/// Locks the profile row for the rest of the transaction, as a profile save does.
async fn lock_profile(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, id: &str) {
    sqlx::query("SELECT id FROM profiles WHERE id = $1 FOR UPDATE")
        .bind(uuid::Uuid::parse_str(id).expect("the identifier"))
        .fetch_one(&mut **tx)
        .await
        .expect("cannot lock the profile");
}

/// Locks the host row for the rest of the transaction, as a change of its values does.
async fn lock_agent(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, id: restichub_db::AgentId) {
    sqlx::query("SELECT id FROM agents WHERE id = $1 FOR UPDATE")
        .bind(id.as_uuid())
        .fetch_one(&mut **tx)
        .await
        .expect("cannot lock the host");
}
