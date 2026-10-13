//! The agent registry endpoints (ADR-0026, FR-ID-04, FR-TN-01).

mod common;

use axum::http::StatusCode;
use restichub_db::{AgentStatus, PLATFORM_TENANT_ID};
use serde_json::{Value, json};

use common::{app, create_tenant, request, seed_agent, seed_session, send};

#[tokio::test]
async fn ac_id_04_an_admin_revokes_an_agent_and_writes_the_audit() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "agents").await;
    let agent = seed_agent(db.database(), tenant, "host-1").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app(db.database());

    // The revoke sets the status and writes the audit record in one transaction.
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/agents/{agent}/revoke"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, body) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/agents/{agent}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.get("status").and_then(Value::as_str), Some("revoked"));
    assert!(body.get("revoked_at").and_then(Value::as_str).is_some());

    let (status, _, page) = send(
        &app,
        request(
            "GET",
            "/api/v1/audit?action=agent_revoked",
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no items"));
    assert!(
        items
            .iter()
            .any(|item| item.get("object_id").and_then(Value::as_str)
                == Some(agent.to_string().as_str())),
        "the audit log must hold the revoke"
    );

    // A second revoke changes nothing (idempotent).
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/agents/{agent}/revoke"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    db.cleanup().await;
}

#[tokio::test]
async fn a_viewer_lists_and_reads_the_agents_and_the_status_filter_works() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "agents-list").await;
    let first = seed_agent(db.database(), tenant, "host-1").await;
    let second = seed_agent(db.database(), tenant, "host-2").await;
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    // Read the list as a Viewer.
    let (status, _, page) = send(&app, request("GET", "/api/v1/agents", Some(&viewer), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        page.get("items").and_then(Value::as_array).map(Vec::len),
        Some(2)
    );

    // Revoke one agent, then the status filter narrows the result.
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/agents/{first}/revoke"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, page) = send(
        &app,
        request("GET", "/api/v1/agents?status=active", Some(&viewer), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no items"));
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].get("id").and_then(Value::as_str),
        Some(second.to_string().as_str())
    );

    // A Viewer cannot rename or revoke (Admin only).
    let (status, _, _) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/agents/{second}"),
            Some(&viewer),
            Some(json!({"hostname": "renamed"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/agents/{second}/revoke"),
            Some(&viewer),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Rename as an Admin.
    let (status, _, body) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/agents/{second}"),
            Some(&admin),
            Some(json!({"hostname": "renamed"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("hostname").and_then(Value::as_str),
        Some("renamed")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_01_an_agent_of_another_tenant_is_not_found() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant_a = create_tenant(db.database(), "agents-a").await;
    let tenant_b = create_tenant(db.database(), "agents-b").await;
    let agent_b = seed_agent(db.database(), tenant_b, "host-b").await;
    let cookie = seed_session(db.database(), tenant_a, "admin", "admin").await;
    let app = app(db.database());

    for (method, path) in [
        ("GET", format!("/api/v1/agents/{agent_b}")),
        ("PATCH", format!("/api/v1/agents/{agent_b}")),
        ("POST", format!("/api/v1/agents/{agent_b}/revoke")),
    ] {
        let body = (method == "PATCH").then(|| json!({"hostname": "renamed"}));
        let (status, _, _) = send(&app, request(method, &path, Some(&cookie), body)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
    }

    // Tenant B keeps its agent active.
    let (status, _, body) = send(&app, request("GET", "/api/v1/agents", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("items").and_then(Value::as_array).map(Vec::len),
        Some(0)
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_platform_tenant_gets_403_on_the_agent_endpoints() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let agent = restichub_db::AgentId::new().to_string();
    let cookie = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let app = app(db.database());

    let cases = [
        ("GET", "/api/v1/agents".to_owned(), None),
        ("GET", format!("/api/v1/agents/{agent}"), None),
        (
            "PATCH",
            format!("/api/v1/agents/{agent}"),
            Some(json!({"hostname": "renamed"})),
        ),
        ("POST", format!("/api/v1/agents/{agent}/revoke"), None),
    ];
    for (method, path, body) in cases {
        let (status, _, _) = send(&app, request(method, &path, Some(&cookie), body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
    }

    assert_eq!(AgentStatus::parse("active"), Some(AgentStatus::Active));
    assert_eq!(AgentStatus::parse("nope"), None);

    db.cleanup().await;
}
