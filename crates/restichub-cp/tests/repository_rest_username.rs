//! The user name of a rest-server repository (FR-RP-01, FR-RP-05, AC-RP-01, AC-TN-01).
//!
//! The user name is a backend setting, not a secret: the view shows it and the update changes it.
//! The password stays write-only. The endpoint never holds the credentials.

#![allow(clippy::expect_used)]

mod common;

use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};

use common::{app_with_kek, create_tenant_with_key, request, seed_session, send};

const ENDPOINT: &str = "https://rest.example.net/main";

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (status, _, response) = send(app, request(method, path, Some(cookie), body)).await;
    (status, response)
}

async fn create(app: &Router, cookie: &str, backend: Value) -> (StatusCode, Value) {
    call(
        app,
        "POST",
        "/api/v1/repositories",
        cookie,
        Some(json!({
            "name": "rest-main",
            "backend": backend,
            "password": "repository-password",
            "rest_password": "rest-canary-password",
        })),
    )
    .await
}

fn id_of(body: &Value) -> String {
    body.get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("the response holds no identifier: {body}"))
}

#[tokio::test]
async fn ac_rp_01_the_rest_username_is_shown_and_changed_and_the_password_stays_hidden() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let (status, body) = create(
        &app,
        &cookie,
        json!({"kind": "rest", "endpoint": ENDPOINT, "username": "  backup  "}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let path = format!("/api/v1/repositories/{}", id_of(&body));

    let (status, view) = call(&app, "GET", &path, &cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(
        view["backend"],
        json!({"kind": "rest", "endpoint": ENDPOINT, "username": "backup"})
    );
    assert_eq!(view["credentials"]["rest_password"], true);
    assert!(!view.to_string().contains("rest-canary-password"));

    let (status, response) = call(
        &app,
        "PATCH",
        &path,
        &cookie,
        Some(json!({"backend": {"kind": "rest", "endpoint": ENDPOINT, "username": "archive"}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let (_, view) = call(&app, "GET", &path, &cookie, None).await;
    assert_eq!(view["backend"]["username"], "archive");

    // An update without the settings keeps the user name.
    let (status, response) = call(
        &app,
        "PATCH",
        &path,
        &cookie,
        Some(json!({"rest_password": "rest-new-password"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let (_, view) = call(&app, "GET", &path, &cookie, None).await;
    assert_eq!(view["backend"]["username"], "archive");

    let (status, response) = call(
        &app,
        "PATCH",
        &path,
        &cookie,
        Some(json!({"backend": {"kind": "rest", "endpoint": ENDPOINT, "username": " "}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let (_, view) = call(&app, "GET", &path, &cookie, None).await;
    assert_eq!(
        view["backend"],
        json!({"kind": "rest", "endpoint": ENDPOINT})
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_rp_01_the_credentials_never_go_into_the_endpoint() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let (status, problem) = create(
        &app,
        &cookie,
        json!({"kind": "rest", "endpoint": "https://backup:secret@rest.example.net/main"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "backend.endpoint");

    let (status, problem) = create(
        &app,
        &cookie,
        json!({"kind": "rest", "endpoint": ENDPOINT, "username": "backup:secret"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "backend.username");
    assert_eq!(problem["reason"], "invalid_character");

    db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_01_another_tenant_cannot_read_or_change_the_rest_username() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant_a = create_tenant_with_key(db.database(), "acme").await;
    let tenant_b = create_tenant_with_key(db.database(), "globex").await;
    let app = app_with_kek(db.database());
    let cookie_a = seed_session(db.database(), tenant_a, "admin-a", "admin").await;
    let cookie_b = seed_session(db.database(), tenant_b, "admin-b", "admin").await;

    let (status, body) = create(
        &app,
        &cookie_b,
        json!({"kind": "rest", "endpoint": ENDPOINT, "username": "globex-backup"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let path = format!("/api/v1/repositories/{}", id_of(&body));

    let (status, _) = call(&app, "GET", &path, &cookie_a, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(
        &app,
        "PATCH",
        &path,
        &cookie_a,
        Some(json!({"backend": {"kind": "rest", "endpoint": ENDPOINT, "username": "taken-over"}})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list) = call(&app, "GET", "/api/v1/repositories", &cookie_a, None).await;
    assert!(!list.to_string().contains("globex-backup"));

    let (_, view) = call(&app, "GET", &path, &cookie_b, None).await;
    assert_eq!(view["backend"]["username"], "globex-backup");

    db.cleanup().await;
}
