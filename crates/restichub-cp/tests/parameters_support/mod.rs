//! The requests of the parameter tests (FR-PF-09 to FR-PF-12, ADR-0047).
//!
//! The tests of the profile parameters share these calls, so each file covers one topic and the
//! request shapes stay in one place.

#![allow(dead_code)]

use axum::Router;
use axum::http::StatusCode;
use restichub_db::{AgentId, TenantId};
use serde_json::{Value, json};

use crate::common::{request, seed_repository, seed_secret, send};

/// Creates a repository that a plan names and returns its identifier as a string.
pub async fn repository_fixture(db: &restichub_db::Database, tenant: TenantId) -> String {
    let secret = seed_secret(db, tenant, "plan.password").await;
    seed_repository(db, tenant, "repo", secret)
        .await
        .to_string()
}

/// Creates a profile through the API and returns its identifier.
pub async fn create_profile(app: &Router, cookie: &str, body: Value) -> String {
    let (status, _, response) = send(
        app,
        request("POST", "/api/v1/profiles", Some(cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    response["profile"]["id"]
        .as_str()
        .expect("the identifier")
        .to_owned()
}

/// Sends a profile update and returns the status and the body.
pub async fn update_profile(
    app: &Router,
    cookie: &str,
    profile: &str,
    body: Value,
) -> (StatusCode, Value) {
    let (status, _, response) = send(
        app,
        request(
            "PATCH",
            &format!("/api/v1/profiles/{profile}"),
            Some(cookie),
            Some(body),
        ),
    )
    .await;
    (status, response)
}

/// Reads one profile of the tenant.
pub async fn profile_view(app: &Router, cookie: &str, profile: &str) -> Value {
    let (status, _, body) = send(
        app,
        request(
            "GET",
            &format!("/api/v1/profiles/{profile}"),
            Some(cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Sends a plan create and returns the status and the body.
pub async fn create_plan(
    app: &Router,
    cookie: &str,
    name: &str,
    profile: &str,
    agent: AgentId,
    repository: &str,
    values: Value,
) -> (StatusCode, Value) {
    let (status, _, body) = send(
        app,
        request(
            "POST",
            "/api/v1/plans",
            Some(cookie),
            Some(plan_body(name, profile, agent, repository, values)),
        ),
    )
    .await;
    (status, body)
}

/// Sends a plan update and returns the status and the body.
pub async fn update_plan(
    app: &Router,
    cookie: &str,
    plan: &str,
    profile: &str,
    agent: AgentId,
    repository: &str,
    values: Value,
) -> (StatusCode, Value) {
    let (status, _, body) = send(
        app,
        request(
            "PATCH",
            &format!("/api/v1/plans/{plan}"),
            Some(cookie),
            Some(plan_body("nightly", profile, agent, repository, values)),
        ),
    )
    .await;
    (status, body)
}

fn plan_body(name: &str, profile: &str, agent: AgentId, repository: &str, values: Value) -> Value {
    json!({
        "name": name,
        "profile_id": profile,
        "agent_id": agent.to_string(),
        "repository_id": repository,
        "cron": "0 2 * * *",
        "time_zone": "UTC",
        "missed_run_policy": "run_all",
        "parameter_values": values,
    })
}

/// Reads one plan of the tenant.
pub async fn plan_view(app: &Router, cookie: &str, plan: &str) -> Value {
    let (status, _, body) = send(
        app,
        request("GET", &format!("/api/v1/plans/{plan}"), Some(cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Lists the plans of the tenant as the reader sees them.
pub async fn list_plans(app: &Router, cookie: &str) -> Value {
    let (status, _, body) = send(app, request("GET", "/api/v1/plans", Some(cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Replaces the parameter values of a host and returns the status and the body.
pub async fn put_host_values(
    app: &Router,
    cookie: &str,
    agent: AgentId,
    values: Value,
) -> (StatusCode, Value) {
    let (status, _, body) = send(
        app,
        request(
            "PUT",
            &format!("/api/v1/agents/{agent}/parameters"),
            Some(cookie),
            Some(json!({"parameters": values})),
        ),
    )
    .await;
    (status, body)
}

/// Reads the parameter table of a host.
pub async fn host_values(app: &Router, cookie: &str, agent: AgentId) -> Value {
    let (status, _, body) = send(
        app,
        request(
            "GET",
            &format!("/api/v1/agents/{agent}/parameters"),
            Some(cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Creates a secret of the purpose `hook_secret` and returns its identifier.
pub async fn create_secret(app: &Router, cookie: &str, name: &str, value: &str) -> String {
    let (status, _, body) = send(
        app,
        request(
            "POST",
            "/api/v1/secrets",
            Some(cookie),
            Some(json!({"name": name, "purpose": "hook_secret", "value": value})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().expect("the identifier").to_owned()
}

/// Finds the row of one name, and fails the test when the table holds none.
pub fn row<'a>(table: &'a Value, name: &str) -> &'a Value {
    table["parameters"]
        .as_array()
        .expect("the parameter list")
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("the parameter {name} is absent: {table}"))
}

/// The audit records of the tenant, as one value.
pub async fn audit(app: &Router, cookie: &str) -> Value {
    let (status, _, body) = send(app, request("GET", "/api/v1/audit", Some(cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// The diff of the newest audit record of one action on one object.
pub async fn audit_diff(app: &Router, cookie: &str, action: &str, object_id: &str) -> Value {
    let records = audit(app, cookie).await;
    records["items"]
        .as_array()
        .expect("a list of records")
        .iter()
        .find(|record| record["action"] == action && record["object_id"] == object_id)
        .and_then(|record| record.get("diff").cloned())
        .unwrap_or_else(|| panic!("no {action} record of {object_id}: {records}"))
}
