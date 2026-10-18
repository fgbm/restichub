//! The audit diff of the parameter values (FR-PF-12, ADR-0047).
//!
//! The diff names a parameter and hides the text of its value. A change of a text value shows in the
//! diff, and so does a change of the reference of a secret value.

#![allow(clippy::expect_used)]

mod common;
mod parameters_support;

use axum::http::StatusCode;
use serde_json::json;

use common::{app_with_kek, create_tenant_with_key, request, seed_agent, seed_session, send};
use parameters_support::{
    audit_diff, create_plan, create_profile, create_secret, put_host_values, repository_fixture,
    update_plan,
};

/// FR-PF-12: a change of a text value of a host shows its name and the marker of the change.
#[tokio::test]
async fn fr_pf_12_the_audit_diff_of_a_host_names_the_changed_value() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let profile = create_profile(
        &app,
        &cookie,
        json!({
            "name": "pg",
            "paths": {"all": ["/data"]},
            "parameters": [{"name": "pg_dir", "kind": "text", "default_value": "/srv/pg"}],
        }),
    )
    .await;
    assert!(!profile.is_empty());

    let (status, body) = put_host_values(
        &app,
        &cookie,
        agent,
        json!([{"name": "pg_dir", "kind": "text", "value": "/srv/pg"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = put_host_values(
        &app,
        &cookie,
        agent,
        json!([{"name": "pg_dir", "kind": "text", "value": "/srv/pg2"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let diff = audit_diff(&app, &cookie, "agent_parameters_update", &agent.to_string()).await;
    let parameters = &diff["parameters"];
    assert_eq!(parameters["old"][0]["name"], "pg_dir", "{diff}");
    assert_eq!(parameters["old"][0]["value"], "<set>", "{diff}");
    assert_eq!(parameters["new"][0]["value"], "<changed>", "{diff}");
    let text = diff.to_string();
    assert!(
        !text.contains("/srv/pg"),
        "the audit holds no value: {text}"
    );
    assert!(
        !text.contains("/srv/pg2"),
        "the audit holds no value: {text}"
    );

    db.cleanup().await;
}

/// FR-PF-12: a plan update that changes the reference of a secret keeps both identifiers and no
/// text of a value.
#[tokio::test]
async fn fr_pf_12_the_audit_diff_of_a_plan_keeps_both_secret_identifiers() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let first = create_secret(&app, &cookie, "pg-password", "CANARY-first-1").await;
    let second = create_secret(&app, &cookie, "pg-password-2", "CANARY-second-2").await;
    let profile = create_profile(
        &app,
        &cookie,
        json!({
            "name": "pg",
            "paths": {"all": ["/data"]},
            "parameters": [{"name": "pg_password", "kind": "secret"}],
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
        json!([{"name": "pg_password", "kind": "secret", "secret_id": first}]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let plan = body["plan"]["id"]
        .as_str()
        .expect("the identifier")
        .to_owned();

    let (status, body) = update_plan(
        &app,
        &cookie,
        &plan,
        &profile,
        agent,
        &repository,
        json!([{"name": "pg_password", "kind": "secret", "secret_id": second}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let diff = audit_diff(&app, &cookie, "plan_update", &plan).await;
    let values = &diff["parameter_values"];
    assert_eq!(values["old"][0]["name"], "pg_password", "{diff}");
    assert_eq!(values["old"][0]["reference"], first.as_str(), "{diff}");
    assert_eq!(values["new"][0]["reference"], second.as_str(), "{diff}");
    let text = diff.to_string();
    assert!(
        !text.contains("CANARY"),
        "the audit holds no secret: {text}"
    );

    db.cleanup().await;
}

/// FR-PF-12: the audit record of a deleted plan lists its values.
#[tokio::test]
async fn fr_pf_12_the_audit_diff_of_a_deleted_plan_lists_the_values() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let secret_id = create_secret(&app, &cookie, "pg-password", "CANARY-delete-1").await;
    let profile = create_profile(
        &app,
        &cookie,
        json!({
            "name": "pg",
            "paths": {"all": ["/data"]},
            "parameters": [
                {"name": "pg_dir", "kind": "text", "default_value": "/srv/pg"},
                {"name": "pg_password", "kind": "secret"},
            ],
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
        json!([
            {"name": "pg_dir", "kind": "text", "value": "CANARY-text-2"},
            {"name": "pg_password", "kind": "secret", "secret_id": secret_id},
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let plan = body["plan"]["id"]
        .as_str()
        .expect("the identifier")
        .to_owned();

    let (status, _, body) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/plans/{plan}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let diff = audit_diff(&app, &cookie, "plan_delete", &plan).await;
    let values = diff["old"]["parameter_values"]
        .as_array()
        .expect("the values of the deleted plan");
    assert_eq!(values.len(), 2, "{diff}");
    let named = |name: &str| {
        values
            .iter()
            .find(|value| value["name"] == name)
            .unwrap_or_else(|| panic!("the value {name} is absent: {diff}"))
    };
    assert_eq!(named("pg_dir")["value"], "<set>");
    assert_eq!(named("pg_password")["reference"], secret_id.as_str());
    let text = diff.to_string();
    assert!(!text.contains("CANARY"), "the audit holds no text: {text}");

    db.cleanup().await;
}
