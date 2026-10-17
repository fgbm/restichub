//! The taken username or email names its request field, and a blank email is no email.
//!
//! AC-AU-07: the taken username of the organization create. AC-AU-10: the taken email of the
//! platform create and update.

#![allow(clippy::expect_used)]

mod common;

use axum::Router;
use axum::http::StatusCode;
use restichub_db::PLATFORM_TENANT_ID;
use serde_json::{Value, json};

use common::{app_with_kek, create_tenant, request, seed_session, send};

async fn post(app: &Router, cookie: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let (status, _, response) = send(app, request("POST", path, Some(cookie), Some(body))).await;
    (status, response)
}

fn account(username: &str, email: Option<&str>) -> Value {
    json!({
        "username": username,
        "display_name": username,
        "password": "a long enough password",
        "email": email,
    })
}

fn code_and_field(problem: &Value) -> (Option<&str>, Option<&str>) {
    (
        problem.get("code").and_then(Value::as_str),
        problem.get("field").and_then(Value::as_str),
    )
}

async fn email_of(app: &Router, cookie: &str, username: &str) -> Value {
    let (status, _, list) = send(
        app,
        request("GET", "/api/v1/platform/users", Some(cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    list.get("items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("username").and_then(Value::as_str) == Some(username))
        })
        .and_then(|item| item.get("email"))
        .cloned()
        .expect("the account is in the list")
}

#[tokio::test]
async fn ac_au_10_a_taken_username_with_an_empty_email_names_the_username_only() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let app = app_with_kek(db.database());
    let path = "/api/v1/platform/users";

    let (status, body) = post(
        &app,
        &cookie,
        path,
        account("alice", Some("alice@example.net")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    for email in ["", "   "] {
        let (status, problem) = post(&app, &cookie, path, account("alice", Some(email))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");
        assert_eq!(
            code_and_field(&problem),
            (Some("username_taken"), Some("username"))
        );
    }

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_10_a_taken_email_with_a_new_username_names_the_email_only() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let app = app_with_kek(db.database());
    let path = "/api/v1/platform/users";

    let (status, body) = post(
        &app,
        &cookie,
        path,
        account("alice", Some("alice@example.net")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, problem) = post(
        &app,
        &cookie,
        path,
        account("sdfsdfasd", Some("Alice@Example.net")),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    assert_eq!(
        code_and_field(&problem),
        (Some("email_taken"), Some("email"))
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_10_accounts_without_an_email_do_not_collide() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let app = app_with_kek(db.database());
    let path = "/api/v1/platform/users";

    for (username, email) in [
        ("blank", Some("")),
        ("spaces", Some("  ")),
        ("absent", None),
    ] {
        let (status, body) = post(&app, &cookie, path, account(username, email)).await;
        assert_eq!(status, StatusCode::CREATED, "{username}: {body}");
        assert_eq!(body.get("email"), Some(&Value::Null), "{username}: {body}");
        assert_eq!(email_of(&app, &cookie, username).await, Value::Null);
    }

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_10_the_email_update_names_the_field_and_a_blank_value_removes_it() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let app = app_with_kek(db.database());
    let path = "/api/v1/platform/users";

    let (status, body) = post(
        &app,
        &cookie,
        path,
        account("alice", Some("alice@example.net")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, bob) = post(&app, &cookie, path, account("bob", Some("bob@example.net"))).await;
    assert_eq!(status, StatusCode::CREATED, "{bob}");
    let bob_path = format!(
        "/api/v1/platform/users/{}",
        bob.get("id")
            .and_then(Value::as_str)
            .expect("the account id")
    );

    let (status, _, problem) = send(
        &app,
        request(
            "PATCH",
            &bob_path,
            Some(&cookie),
            Some(json!({"email": "ALICE@example.net"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    assert_eq!(
        code_and_field(&problem),
        (Some("email_taken"), Some("email"))
    );

    let alice_path = format!(
        "/api/v1/platform/users/{}",
        body.get("id")
            .and_then(Value::as_str)
            .expect("the account id")
    );
    // The second clear must not collide with the first one in the unique email index.
    for (username, account_path) in [("alice", &alice_path), ("bob", &bob_path)] {
        let (status, _, response) = send(
            &app,
            request(
                "PATCH",
                account_path,
                Some(&cookie),
                Some(json!({"email": " "})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{response}");
        assert_eq!(email_of(&app, &cookie, username).await, Value::Null);
    }

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_07_the_organization_create_names_the_taken_username() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let root = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let app = app_with_kek(db.database());
    let tenant = create_tenant(db.database(), "conflict-org").await;
    let admin = seed_session(db.database(), tenant, "org-admin", "admin").await;

    let (status, body) = post(
        &app,
        &root,
        "/api/v1/platform/users",
        account("alice", None),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, problem) = post(
        &app,
        &admin,
        "/api/v1/users",
        json!({
            "username": "ALICE",
            "display_name": "Alice",
            "password": "a long enough password",
            "role": "viewer",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    assert_eq!(
        code_and_field(&problem),
        (Some("username_taken"), Some("username"))
    );

    db.cleanup().await;
}
