//! Initial setup tests (FR-AU-03).

#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use restichub_cp::clock::SystemClock;
use restichub_cp::config::TenancyConfig;
use restichub_cp::setup::ensure_setup_token;
use restichub_db::{PLATFORM_TENANT_ID, setup};
use serde_json::{Value, json};
use time::OffsetDateTime;

use common::{app, count, create_tenant_with_key, create_user, request, send};

/// Inserts a setup token for the platform tenant and returns its value.
async fn insert_setup_token(db: &restichub_db::Database, value: &str, expires_at: OffsetDateTime) {
    let hash = restichub_cp::token::hash_token(value);
    let mut tx = db
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .expect("the database call failed");
    setup::create(&mut tx, &hash, expires_at)
        .await
        .expect("the database call failed");
    tx.commit().await.expect("the database call failed");
}

#[tokio::test]
async fn ensure_setup_token_creates_one_when_there_is_no_user() {
    let Some(db) = common::test_db().await else {
        return;
    };
    ensure_setup_token(db.database(), &SystemClock, &TenancyConfig::multi())
        .await
        .expect("the database call failed");
    assert_eq!(
        count(db.database(), PLATFORM_TENANT_ID, "setup_tokens").await,
        1
    );

    // A second start with no user creates another token (the old one may be expired).
    ensure_setup_token(db.database(), &SystemClock, &TenancyConfig::multi())
        .await
        .expect("the database call failed");
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
    ensure_setup_token(db.database(), &SystemClock, &TenancyConfig::multi())
        .await
        .expect("the database call failed");
    assert_eq!(
        count(db.database(), PLATFORM_TENANT_ID, "setup_tokens").await,
        0
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ensure_setup_token_uses_the_default_tenant_in_single_mode() {
    let Some(db) = common::test_db().await else {
        return;
    };
    // The tenant `default` is absent, so the setup is required and the token is issued.
    ensure_setup_token(
        db.database(),
        &SystemClock,
        &TenancyConfig::single("default"),
    )
    .await
    .expect("the database call failed");
    assert_eq!(
        count(db.database(), PLATFORM_TENANT_ID, "setup_tokens").await,
        1
    );

    // With the tenant and a user, the setup is not required and no token is issued.
    let tenant = create_tenant_with_key(db.database(), "default").await;
    create_user(db.database(), tenant, "admin", "phc", "admin").await;
    ensure_setup_token(
        db.database(),
        &SystemClock,
        &TenancyConfig::single("default"),
    )
    .await
    .expect("the database call failed");
    assert_eq!(
        count(db.database(), PLATFORM_TENANT_ID, "setup_tokens").await,
        1
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_setup_creates_the_first_platform_admin_once() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let value = restichub_cp::token::random_token().expect("cannot build the token");
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

    // AC-AU-11: the first account holds the right and no membership. It signs in and reaches the
    // platform screens (ADR-0029).
    assert_eq!(common::platform_admin_count(db.database()).await, 1);
    assert_eq!(
        common::count(db.database(), PLATFORM_TENANT_ID, "memberships").await,
        0
    );
    let sign_in = json!({"username": "root", "password": "a long enough password"});
    let (status, cookie, me) = send(
        &app,
        request("POST", "/api/v1/auth/sign-in", None, Some(sign_in)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(
        me.get("platform_admin").and_then(Value::as_bool),
        Some(true)
    );
    // A Platform Admin with no membership carries the synthetic Platform Admin role (ADR-0029).
    assert_eq!(
        me.get("role").and_then(Value::as_str),
        Some("platform_admin")
    );
    assert!(
        me.get("tenant").map(Value::is_null).unwrap_or(false),
        "{me}"
    );
    let cookie = cookie.expect("a session cookie");
    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/platform/tenants", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

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
    let (status, _, problem) = send(&app, request("POST", "/api/v1/setup", None, Some(body))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("setup_already_done")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn two_parallel_setup_calls_leave_exactly_one_admin() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let value = restichub_cp::token::random_token().expect("cannot build the token");
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
    // The setup creates one account with the Platform Admin right and no membership (ADR-0029).
    assert_eq!(common::platform_admin_count(db.database()).await, 1);

    db.cleanup().await;
}

#[tokio::test]
async fn an_expired_setup_token_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let value = restichub_cp::token::random_token().expect("cannot build the token");
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
        Some("invalid_setup_token")
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
