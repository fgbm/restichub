//! The delete of a plan that holds a secret parameter value (ADR-0047).
//!
//! The delete removes the secret values of the plan, and only an Admin sets, changes or removes a
//! secret value, because it chooses the credential that a script receives.

#![allow(clippy::expect_used)]

mod common;
mod parameters_support;

use axum::http::StatusCode;
use serde_json::json;

use common::{app_with_kek, create_tenant_with_key, request, seed_agent, seed_session, send};
use parameters_support::{
    create_plan, create_profile, create_secret, list_plans, plan_view, repository_fixture,
};

/// ADR-0047: an Operator deletes no plan that holds a secret value, and the plan stays.
#[tokio::test]
async fn adr_0047_an_operator_deletes_no_plan_with_a_secret_value() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "plan-delete").await;
    let app = app_with_kek(db.database());
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let operator = seed_session(db.database(), tenant, "operator", "operator").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let secret = create_secret(&app, &admin, "pg-password", "CANARY-delete-1").await;
    let profile = create_profile(
        &app,
        &admin,
        json!({
            "name": "pg",
            "paths": {"all": ["/data"]},
            "parameters": [{"name": "pg_password", "kind": "secret"}],
        }),
    )
    .await;
    let (status, body) = create_plan(
        &app,
        &admin,
        "nightly",
        &profile,
        agent,
        &repository,
        json!([{"name": "pg_password", "kind": "secret", "secret_id": secret}]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let plan = body["plan"]["id"]
        .as_str()
        .expect("the identifier")
        .to_owned();

    let (status, _, problem) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/plans/{plan}"),
            Some(&operator),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(problem["code"], "forbidden");
    assert_eq!(plan_view(&app, &admin, &plan).await["id"], json!(plan));

    // The Admin removes the plan with the secret value.
    let (status, _, problem) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/plans/{plan}"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{problem}");
    assert_eq!(
        list_plans(&app, &admin).await["items"]
            .as_array()
            .expect("a list")
            .len(),
        0
    );

    db.cleanup().await;
}

/// ADR-0047: an Operator deletes a plan that holds no secret value.
#[tokio::test]
async fn adr_0047_an_operator_deletes_a_plan_with_text_values_only() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "plan-delete").await;
    let app = app_with_kek(db.database());
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let operator = seed_session(db.database(), tenant, "operator", "operator").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let profile = create_profile(
        &app,
        &admin,
        json!({
            "name": "pg",
            "paths": {"all": ["/data"]},
            "parameters": [{"name": "pg_dir", "kind": "text"}],
        }),
    )
    .await;
    let (status, body) = create_plan(
        &app,
        &admin,
        "nightly",
        &profile,
        agent,
        &repository,
        json!([{"name": "pg_dir", "kind": "text", "value": "/srv/pg"}]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let plan = body["plan"]["id"]
        .as_str()
        .expect("the identifier")
        .to_owned();

    let (status, _, problem) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/plans/{plan}"),
            Some(&operator),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{problem}");

    db.cleanup().await;
}
