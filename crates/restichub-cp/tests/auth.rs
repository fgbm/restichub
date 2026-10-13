//! Sign-in, session and self-password tests (AC-AU-02).

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use restichub_cp::clock::ManualClock;
use restichub_cp::config::{AuthConfig, TenancyConfig};
use restichub_cp::password;
use restichub_cp::rate_limit::RateLimiter;
use restichub_cp::server::AppState;
use restichub_cp::server::AuthState;
use restichub_cp::server::router;
use restichub_db::{PLATFORM_TENANT_ID, TenantId};
use serde_json::{Value, json};
use time::OffsetDateTime;

use common::{
    app, app_with_auth, body_json, create_tenant, create_user, request, seed_session, send,
    send_response, session_cookie, set_tenant_status,
};

/// Builds a user whose password hash is real.
async fn user_with_password(
    db: &restichub_db::Database,
    tenant: TenantId,
    username: &str,
    password_value: &str,
    role: &str,
) -> restichub_db::UserId {
    let hash = password::hash_password(password_value).unwrap_or_default();
    create_user(db, tenant, username, &hash, role).await
}

/// Returns the result of a sign-in through the HTTP API.
async fn sign_in(
    app: &Router,
    tenant: &str,
    username: &str,
    password_value: &str,
) -> (StatusCode, Option<String>, Value) {
    let body = json!({"tenant": tenant, "username": username, "password": password_value});
    send(
        app,
        request("POST", "/api/v1/auth/sign-in", None, Some(body)),
    )
    .await
}

/// Signs in and returns the cookie pair. The caller has a valid account.
async fn sign_in_ok(app: &Router, tenant: &str, username: &str, password_value: &str) -> String {
    let (status, cookie, _) = sign_in(app, tenant, username, password_value).await;
    assert_eq!(status, StatusCode::OK);
    cookie.unwrap_or_default()
}

#[tokio::test]
async fn sign_in_sets_a_cookie_and_returns_the_actor() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "sign-in").await;
    user_with_password(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let app = app(db.database());

    let (status, cookie, body) = sign_in(&app, "sign-in", "alice", "correct horse battery").await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.unwrap_or_default();
    assert!(cookie.starts_with("__Host-restichub_session="));
    assert_eq!(body.get("role").and_then(Value::as_str), Some("admin"));
    assert_eq!(
        body.pointer("/user/username").and_then(Value::as_str),
        Some("alice")
    );
    assert_eq!(
        body.pointer("/tenant/name").and_then(Value::as_str),
        Some("sign-in")
    );

    let (status, _, me) = send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me.get("role").and_then(Value::as_str), Some("admin"));

    db.cleanup().await;
}

#[tokio::test]
async fn sign_in_failures_are_identical() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "failures").await;
    let user = user_with_password(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    // An account with no membership in any active tenant.
    let orphan_hash = password::hash_password("correct horse battery").unwrap_or_default();
    {
        let mut tx = db
            .database()
            .begin_tenant(tenant)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        restichub_db::users::create(
            &mut tx,
            &restichub_db::users::NewUser {
                username: "orphan",
                password_hash: &orphan_hash,
                display_name: "Orphan",
            },
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
    }
    let app = app(db.database());

    // An unknown account, a wrong password and an account with no active membership.
    let (s1, _, b1) = sign_in(&app, "failures", "nobody", "correct horse battery").await;
    let (s2, _, b2) = sign_in(&app, "failures", "alice", "wrong password value").await;
    let (s3, _, b3) = sign_in(&app, "failures", "orphan", "correct horse battery").await;

    // Disabled account.
    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    restichub_db::users::set_disabled(&mut tx, user, true)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
    let (s4, _, b4) = sign_in(&app, "failures", "alice", "correct horse battery").await;

    // Suspended tenant.
    set_tenant_status(db.database(), tenant, "suspended").await;
    let (s5, _, b5) = sign_in(&app, "failures", "alice", "correct horse battery").await;

    for (status, body) in [(s1, &b1), (s2, &b2), (s3, &b3), (s4, &b4), (s5, &b5)] {
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            body.get("code").and_then(Value::as_str),
            Some("invalid_credentials")
        );
        assert_eq!(body.get("status").and_then(Value::as_u64), Some(401));
        assert_eq!(
            body.get("title").and_then(Value::as_str),
            Some("Unauthorized")
        );
    }

    db.cleanup().await;
}

#[tokio::test]
async fn sign_in_requires_the_csrf_header() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "csrf").await;
    user_with_password(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let app = app(db.database());

    let body = json!({"tenant": "csrf", "username": "alice", "password": "correct horse battery"});
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/sign-in")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap_or_else(|e| panic!("{e}"));
    request.extensions_mut().insert(common::peer());
    let response = send_response(&app, request).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let problem = body_json(response).await;
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("csrf_required")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_sign_in_rate_limit_refuses_with_retry_after() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "rate-limit").await;
    user_with_password(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let config = AuthConfig {
        sign_in_user_rate: 2,
        sign_in_ip_rate: 100,
        ..AuthConfig::default()
    };
    let app = app_with_auth(
        db.database(),
        config,
        Arc::new(ManualClock::new(OffsetDateTime::now_utc())),
    );

    for _ in 0..2 {
        let (status, _, _) = sign_in(&app, "rate-limit", "alice", "wrong password value").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let body =
        json!({"tenant": "rate-limit", "username": "alice", "password": "wrong password value"});
    let response = send_response(
        &app,
        request("POST", "/api/v1/auth/sign-in", None, Some(body)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(common::retry_after(&response).is_some());
    let problem = body_json(response).await;
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("rate_limited")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_session_expires_when_it_is_idle() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "idle").await;
    user_with_password(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let clock = Arc::new(ManualClock::new(OffsetDateTime::now_utc()));
    let config = AuthConfig {
        session_idle: Duration::from_secs(60),
        session_absolute: Duration::from_secs(3600),
        ..AuthConfig::default()
    };
    let app = app_with_auth(db.database(), config, clock.clone());
    let cookie = sign_in_ok(&app, "idle", "alice", "correct horse battery").await;

    let (status, _, _) = send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::OK);

    clock.advance(120);
    let (status, _, problem) =
        send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("unauthenticated")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_session_expires_at_the_absolute_lifetime() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "absolute").await;
    user_with_password(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let clock = Arc::new(ManualClock::new(OffsetDateTime::now_utc()));
    let config = AuthConfig {
        session_idle: Duration::from_secs(3600),
        session_absolute: Duration::from_secs(120),
        ..AuthConfig::default()
    };
    let app = app_with_auth(db.database(), config, clock.clone());
    let cookie = sign_in_ok(&app, "absolute", "alice", "correct horse battery").await;

    clock.advance(180);
    let (status, _, _) = send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    db.cleanup().await;
}

#[tokio::test]
async fn sign_out_removes_the_session() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "sign-out").await;
    user_with_password(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let app = app(db.database());
    let cookie = sign_in_ok(&app, "sign-out", "alice", "correct horse battery").await;

    let (status, _, _) = send(
        &app,
        request("POST", "/api/v1/auth/sign-out", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    db.cleanup().await;
}

#[tokio::test]
async fn a_password_change_keeps_the_current_session_and_kills_the_others() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "password").await;
    user_with_password(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let app = app(db.database());
    let first = sign_in_ok(&app, "password", "alice", "correct horse battery").await;
    let second = sign_in_ok(&app, "password", "alice", "correct horse battery").await;

    let body =
        json!({"current_password": "correct horse battery", "new_password": "a new long password"});
    let (status, _, _) = send(
        &app,
        request("POST", "/api/v1/auth/password", Some(&first), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = send(&app, request("GET", "/api/v1/auth/me", Some(&first), None)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(&app, request("GET", "/api/v1/auth/me", Some(&second), None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let _ = sign_in_ok(&app, "password", "alice", "a new long password").await;

    db.cleanup().await;
}

#[tokio::test]
async fn a_disabled_user_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "disabled").await;
    let user = user_with_password(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let app = app(db.database());
    let cookie = sign_in_ok(&app, "disabled", "alice", "correct horse battery").await;

    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    restichub_db::users::set_disabled(&mut tx, user, true)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));

    let (status, _, _) = send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_03_a_suspended_tenant_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "suspended").await;
    user_with_password(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let app = app(db.database());
    let cookie = sign_in_ok(&app, "suspended", "alice", "correct horse battery").await;

    set_tenant_status(db.database(), tenant, "suspended").await;
    let (status, _, _) = send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    db.cleanup().await;
}

#[tokio::test]
async fn a_cookie_and_a_bearer_token_are_ambiguous() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "ambiguous").await;
    let user = create_user(db.database(), tenant, "alice", "phc", "admin").await;
    let cookie = session_cookie(db.database(), tenant, user).await;
    let app = app(db.database());

    let request = Request::builder()
        .method("GET")
        .uri("/api/v1/auth/me")
        .header("cookie", &cookie)
        .header("authorization", "Bearer rh_unknown")
        .body(Body::empty())
        .unwrap_or_else(|e| panic!("{e}"));
    let mut request = request;
    request.extensions_mut().insert(common::peer());
    let response = send_response(&app, request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let problem = body_json(response).await;
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("ambiguous_credentials")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_cookie_unsafe_request_needs_the_csrf_header() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "cookie-csrf").await;
    let user = create_user(db.database(), tenant, "alice", "phc", "admin").await;
    let cookie = session_cookie(db.database(), tenant, user).await;
    let app = app(db.database());

    let body = json!({"current_password": "x", "new_password": "a new long password"});
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/password")
        .header("cookie", &cookie)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap_or_else(|e| panic!("{e}"));
    let mut request = request;
    request.extensions_mut().insert(common::peer());
    let response = send_response(&app, request).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let problem = body_json(response).await;
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("csrf_required")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_bearer_token_does_not_need_the_csrf_header() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "bearer-csrf").await;
    let token = common::seed_api_token(db.database(), tenant, "admin").await;
    let app = app(db.database());

    // A POST with a bearer token and no CSRF header is not refused with csrf_required.
    let body = json!({"name": "new-token", "role": "viewer"});
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/api-tokens")
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap_or_else(|e| panic!("{e}"));
    let mut request = request;
    request.extensions_mut().insert(common::peer());
    let response = send_response(&app, request).await;
    assert_eq!(response.status(), StatusCode::CREATED);

    db.cleanup().await;
}

#[tokio::test]
async fn the_platform_setup_session_works() {
    let Some(db) = common::test_db().await else {
        return;
    };
    // A Platform Admin of the platform tenant can sign in and read the platform audit.
    let cookie = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let app = router(AppState::new(db.database().clone()).with_tenancy(TenancyConfig::multi()));
    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/platform/audit", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    db.cleanup().await;
}

#[tokio::test]
async fn the_sign_in_rate_limit_ignores_the_case() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "case").await;
    let _ = tenant;
    let config = AuthConfig {
        sign_in_user_rate: 10,
        sign_in_ip_rate: 1000,
        ..AuthConfig::default()
    };
    // Use a long refill window, so a slow or a loaded machine cannot refill a token during the
    // test. The bucket then holds exactly the capacity.
    let window = Duration::from_secs(3600);
    let mut auth = AuthState::new(config.clone());
    auth.user_limiter = Arc::new(RateLimiter::new(
        config.sign_in_user_rate,
        window,
        restichub_cp::rate_limit::MAX_ENTRIES,
    ));
    auth.ip_limiter = Arc::new(RateLimiter::new(
        config.sign_in_ip_rate,
        window,
        restichub_cp::rate_limit::MAX_ENTRIES,
    ));
    let app = router(
        AppState::with_auth(db.database().clone(), auth).with_tenancy(TenancyConfig::multi()),
    );

    // The same unknown user with alternating case uses one bucket.
    for index in 0..10 {
        let username = if index % 2 == 0 { "Nobody" } else { "nobody" };
        let (status, _, _) = sign_in(&app, "case", username, "wrong password value").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _, problem) = sign_in(&app, "case", "NOBODY", "wrong password value").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("rate_limited")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_sign_in_body_over_the_limit_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app(db.database());
    let body = json!({
        "tenant": "big",
        "username": "alice",
        "password": "a".repeat(20_000),
    });
    let response = send_response(
        &app,
        request("POST", "/api/v1/auth/sign-in", None, Some(body)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

    db.cleanup().await;
}

#[tokio::test]
async fn a_full_synthetic_map_does_not_block_a_known_user() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "bucket-cap").await;
    user_with_password(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let config = AuthConfig {
        sign_in_user_rate: 100,
        sign_in_ip_rate: 1_000,
        ..AuthConfig::default()
    };
    let auth = AuthState::new(config).with_bucket_cap(4);
    let app = router(
        AppState::with_auth(db.database().clone(), auth).with_tenancy(TenancyConfig::multi()),
    );

    // Fill the synthetic map with distinct unknown users. The map evicts instead of growing, and
    // each attempt gives the generic error.
    for index in 0..20 {
        let (status, _, _) = sign_in(
            &app,
            "bucket-cap",
            &format!("nobody{index}"),
            "wrong password value",
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    // A known user still signs in. The resolved map is separate from the synthetic map.
    let (status, cookie, _) = sign_in(&app, "bucket-cap", "alice", "correct horse battery").await;
    assert_eq!(status, StatusCode::OK);
    assert!(cookie.is_some());

    db.cleanup().await;
}

#[tokio::test]
async fn a_saturated_semaphore_does_not_block_other_requests() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "pool").await;
    user_with_password(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let config = AuthConfig {
        max_concurrent_hashes: 1,
        ..AuthConfig::default()
    };
    let auth = AuthState::new(config);
    let semaphore = auth.hash_semaphore.clone();
    let database = db
        .database_with_max(3)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let app = router(AppState::with_auth(database, auth).with_tenancy(TenancyConfig::multi()));
    let cookie = sign_in_ok(&app, "pool", "alice", "correct horse battery").await;

    // Hold the only hash permit. A sign-in then waits for it without a database transaction.
    let _permit = semaphore.acquire().await.unwrap_or_else(|e| panic!("{e}"));

    let (first, second, third, me) = tokio::join!(
        sign_in(&app, "pool", "alice", "wrong password value"),
        sign_in(&app, "pool", "alice", "wrong password value"),
        sign_in(&app, "pool", "alice", "wrong password value"),
        send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)),
    );
    assert_eq!(
        me.0,
        StatusCode::OK,
        "a hashing request must not hold a connection"
    );
    for status in [first.0, second.0, third.0] {
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    db.cleanup().await;
}
