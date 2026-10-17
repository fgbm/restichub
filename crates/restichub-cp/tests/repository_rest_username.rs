//! The user name of a rest-server repository (FR-RP-01, FR-RP-05, AC-RP-01, AC-TN-01).
//!
//! The user name is a backend setting, not a secret: the view shows it and the update changes it.
//! The password stays write-only. The endpoint never holds the credentials.

#![allow(clippy::expect_used)]

mod common;

use axum::Router;
use axum::http::StatusCode;
use serde_json::{Value, json};

use common::{app_with_kek, create_tenant_with_key, request, seed_agent, seed_session, send};

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

#[tokio::test]
async fn ac_rp_01_an_update_without_the_user_name_keeps_it_and_null_clears_it() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let (status, body) = create(
        &app,
        &cookie,
        json!({"kind": "rest", "endpoint": ENDPOINT, "username": "backup"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let path = format!("/api/v1/repositories/{}", id_of(&body));
    let other_endpoint = "https://rest.example.net/other";

    let (status, response) = call(
        &app,
        "PATCH",
        &path,
        &cookie,
        Some(json!({"backend": {"kind": "rest", "endpoint": other_endpoint}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let (_, view) = call(&app, "GET", &path, &cookie, None).await;
    assert_eq!(
        view["backend"],
        json!({"kind": "rest", "endpoint": other_endpoint, "username": "backup"})
    );

    let (status, response) = call(
        &app,
        "PATCH",
        &path,
        &cookie,
        Some(json!({"backend": {"kind": "rest", "endpoint": ENDPOINT, "username": null}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let (_, view) = call(&app, "GET", &path, &cookie, None).await;
    assert_eq!(
        view["backend"],
        json!({"kind": "rest", "endpoint": ENDPOINT})
    );

    let (status, _) = call(
        &app,
        "PATCH",
        &path,
        &cookie,
        Some(json!({"backend": {"kind": "rest", "endpoint": ENDPOINT, "username": "archive"}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(
        &app,
        "PATCH",
        &path,
        &cookie,
        Some(json!({"backend": {"kind": "rest", "endpoint": ENDPOINT, "username": ""}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, view) = call(&app, "GET", &path, &cookie, None).await;
    assert!(view["backend"].get("username").is_none(), "{view}");

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_01_the_repository_audit_holds_the_rest_username_and_no_password() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let (status, body) = create(
        &app,
        &cookie,
        json!({"kind": "rest", "endpoint": ENDPOINT, "username": "backup"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let path = format!("/api/v1/repositories/{}", id_of(&body));
    let (status, response) = call(
        &app,
        "PATCH",
        &path,
        &cookie,
        Some(json!({
            "backend": {"kind": "rest", "endpoint": ENDPOINT, "username": "archive"},
            "rest_password": "rest-new-password",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");

    let (status, page) = call(
        &app,
        "GET",
        "/api/v1/audit?action=repository_update",
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let items = page["items"].as_array().expect("the items");
    assert_eq!(items.len(), 1, "{page}");
    let diff = items[0]["diff"].to_string();
    assert!(diff.contains("archive"), "{diff}");
    let (_, all) = call(&app, "GET", "/api/v1/audit", &cookie, None).await;
    let all = all.to_string();
    assert!(all.contains("backup"), "{all}");
    for password in [
        "repository-password",
        "rest-canary-password",
        "rest-new-password",
    ] {
        assert!(!all.contains(password), "{all}");
    }

    db.cleanup().await;
}

/// The preview shows the real user name, which is not a secret, and only when it is set.
#[tokio::test]
async fn ac_pf_01_the_preview_shows_the_rest_username_and_masks_the_password() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;

    let (status, body) = create(
        &app,
        &cookie,
        json!({"kind": "rest", "endpoint": ENDPOINT, "username": "backup"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let repository_id = id_of(&body);
    let (status, profile) = call(
        &app,
        "POST",
        "/api/v1/profiles",
        &cookie,
        Some(json!({
            "name": "web-daily",
            "repository_id": repository_id,
            "paths": {"all": ["/srv"]},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{profile}");
    let preview_path = format!("/api/v1/profiles/{}/preview", id_of(&profile["profile"]));
    let backup_env = |preview: &Value| -> Vec<(String, String)> {
        let steps = preview["steps"].as_array().expect("the steps");
        let backup = steps
            .iter()
            .find(|step| step["kind"] == "backup")
            .expect("the backup step");
        backup["environment"]
            .as_array()
            .expect("the environment")
            .iter()
            .map(|entry| {
                (
                    entry["name"].as_str().expect("the name").to_owned(),
                    entry["value"].as_str().expect("the value").to_owned(),
                )
            })
            .collect()
    };
    let agent_body = Some(json!({"agent_id": agent.to_string()}));

    let (status, preview) = call(&app, "POST", &preview_path, &cookie, agent_body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let env = backup_env(&preview);
    assert!(env.contains(&("RESTIC_REST_USERNAME".to_owned(), "backup".to_owned())));
    assert!(env.contains(&("RESTIC_REST_PASSWORD".to_owned(), "***".to_owned())));
    assert!(!preview.to_string().contains("rest-canary-password"));

    let (status, _) = call(
        &app,
        "PATCH",
        &format!("/api/v1/repositories/{repository_id}"),
        &cookie,
        Some(json!({"backend": {"kind": "rest", "endpoint": ENDPOINT, "username": null}})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, preview) = call(&app, "POST", &preview_path, &cookie, agent_body).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let env = backup_env(&preview);
    assert!(
        env.iter().all(|(name, _)| name != "RESTIC_REST_USERNAME"),
        "{env:?}"
    );
    assert!(env.contains(&("RESTIC_REST_PASSWORD".to_owned(), "***".to_owned())));

    db.cleanup().await;
}
