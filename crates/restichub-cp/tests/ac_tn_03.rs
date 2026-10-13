//! AC-TN-03: a suspended tenant (FR-TN-02; partial).
//!
//! The Platform Admin suspends a tenant. Users cannot sign in, the API tokens fail and the existing
//! sessions stop working. The platform audit holds the event. The agents stay connected (epic #1);
//! the dispatcher flag is `tenants::is_dispatch_allowed` (epic #7).

mod common;

use axum::http::StatusCode;
use restichub_cp::password;
use restichub_db::PLATFORM_TENANT_ID;
use serde_json::{Value, json};

use common::{
    app_with_kek, create_tenant, create_user, request, seed_api_token, seed_session, send,
    session_cookie,
};

#[tokio::test]
async fn ac_tn_03_a_suspended_tenant_cannot_sign_in_and_the_platform_audit_has_the_event() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "suspend-ac").await;
    let hash = password::hash_password("correct horse battery").unwrap_or_default();
    let user = create_user(db.database(), tenant, "alice", &hash, "admin").await;

    // An existing session and an API token of the tenant.
    let cookie = session_cookie(db.database(), tenant, user).await;
    let token = seed_api_token(db.database(), tenant, "admin").await;

    let app = app_with_kek(db.database());
    let admin = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;

    // The tenant is active: the session and the token work.
    let (status, _, _) = send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(
        &app,
        common::bearer_request("GET", "/api/v1/users", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, _) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/platform/tenants/{tenant}/suspend"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let body =
        json!({"tenant": "suspend-ac", "username": "alice", "password": "correct horse battery"});
    let (status, _, problem) = send(
        &app,
        request("POST", "/api/v1/auth/sign-in", None, Some(body)),
    )
    .await;
    // The suspended membership is skipped; the account signs in with no organization (ADR-0029).
    assert_eq!(status, StatusCode::OK, "{problem}");
    assert!(
        problem.get("tenant").map(Value::is_null).unwrap_or(false),
        "{problem}"
    );

    let (status, _, _) = send(
        &app,
        common::bearer_request("GET", "/api/v1/users", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _, _) = send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let uri = format!("/api/v1/platform/audit?tenant_id={tenant}&action=tenant_suspend");
    let (status, _, page) = send(&app, request("GET", &uri, Some(&admin), None)).await;
    assert_eq!(status, StatusCode::OK);
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].get("action").and_then(Value::as_str),
        Some("tenant_suspend")
    );

    // The dispatcher refuses jobs for the suspended tenant (epic #7). The agents stay connected
    // (epic #1).
    assert!(!restichub_db::tenants::is_dispatch_allowed(
        restichub_db::TenantStatus::Suspended
    ));
    assert!(restichub_db::tenants::is_dispatch_allowed(
        restichub_db::TenantStatus::Active
    ));

    db.cleanup().await;
}
