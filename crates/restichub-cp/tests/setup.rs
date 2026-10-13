//! Initial setup tests (FR-AU-03).

mod common;

use axum::http::StatusCode;
use restichub_cp::clock::SystemClock;
use restichub_cp::setup::ensure_setup_token;
use restichub_db::{PLATFORM_TENANT_ID, setup};
use serde_json::{Value, json};
use time::OffsetDateTime;

use common::{app, count, create_user, request, send};

/// Inserts a setup token for the platform tenant and returns its value.
async fn insert_setup_token(db: &restichub_db::Database, value: &str, expires_at: OffsetDateTime) {
    let hash = restichub_cp::token::hash_token(value);
    let mut tx = db
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    setup::create(&mut tx, &hash, expires_at)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
}

#[tokio::test]
async fn ensure_setup_token_creates_one_when_there_is_no_user() {
    let Some(db) = common::test_db().await else {
        return;
    };
    ensure_setup_token(db.database(), &SystemClock)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        count(db.database(), PLATFORM_TENANT_ID, "setup_tokens").await,
        1
    );

    // A second start with no user creates another token (the old one may be expired).
    ensure_setup_token(db.database(), &SystemClock)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        count(db.database(), PLATFORM_TENANT_ID, "setup_tokens").await,
        2
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ensure_setup_token_does_not_create_when_a_user_exists() {
    let Some(db) = common::test_db().await else {
        return;
    };
    create_user(
        db.database(),
        PLATFORM_TENANT_ID,
        "root",
        "phc",
        "platform_admin",
    )
    .await;
    ensure_setup_token(db.database(), &SystemClock)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        count(db.database(), PLATFORM_TENANT_ID, "setup_tokens").await,
        0
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_setup_creates_the_first_platform_admin_once() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let value = restichub_cp::token::random_token().unwrap_or_default();
    insert_setup_token(
        db.database(),
        &value,
        OffsetDateTime::now_utc() + time::Duration::hours(24),
    )
    .await;
    let app = app(db.database());

    // The status reports that the setup is required.
    let (status, _, body) = send(&app, request("GET", "/api/v1/setup/status", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("setup_required").and_then(Value::as_bool),
        Some(true)
    );

    let body = json!({
        "setup_token": value,
        "username": "root",
        "password": "a long enough password",
        "display_name": "Root",
    });
    let (status, _, created) = send(&app, request("POST", "/api/v1/setup", None, Some(body))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        created.get("role").and_then(Value::as_str),
        Some("platform_admin")
    );

    let (_, _, body) = send(&app, request("GET", "/api/v1/setup/status", None, None)).await;
    assert_eq!(
        body.get("setup_required").and_then(Value::as_bool),
        Some(false)
    );

    let body = json!({
        "setup_token": value,
        "username": "root2",
        "password": "a long enough password",
        "display_name": "Root Two",
    });
    let (status, _, _) = send(&app, request("POST", "/api/v1/setup", None, Some(body))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    db.cleanup().await;
}

#[tokio::test]
async fn two_parallel_setup_calls_leave_exactly_one_admin() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let value = restichub_cp::token::random_token().unwrap_or_default();
    insert_setup_token(
        db.database(),
        &value,
        OffsetDateTime::now_utc() + time::Duration::hours(24),
    )
    .await;
    let app = app(db.database());
    let body = json!({
        "setup_token": value,
        "username": "root",
        "password": "a long enough password",
        "display_name": "Root",
    });

    let (first, second) = tokio::join!(
        send(
            &app,
            request("POST", "/api/v1/setup", None, Some(body.clone()))
        ),
        send(&app, request("POST", "/api/v1/setup", None, Some(body)))
    );
    let statuses = [first.0, second.0];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "exactly one setup call must succeed: {statuses:?}"
    );
    assert_eq!(count(db.database(), PLATFORM_TENANT_ID, "users").await, 1);

    db.cleanup().await;
}

#[tokio::test]
async fn an_expired_setup_token_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let value = restichub_cp::token::random_token().unwrap_or_default();
    insert_setup_token(
        db.database(),
        &value,
        OffsetDateTime::now_utc() - time::Duration::hours(1),
    )
    .await;
    let app = app(db.database());

    let body = json!({
        "setup_token": value,
        "username": "root",
        "password": "a long enough password",
        "display_name": "Root",
    });
    let (status, _, problem) = send(&app, request("POST", "/api/v1/setup", None, Some(body))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("invalid_credentials")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn an_unknown_setup_token_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app(db.database());
    let body = json!({
        "setup_token": "not-a-real-token",
        "username": "root",
        "password": "a long enough password",
        "display_name": "Root",
    });
    let (status, _, _) = send(&app, request("POST", "/api/v1/setup", None, Some(body))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    db.cleanup().await;
}
