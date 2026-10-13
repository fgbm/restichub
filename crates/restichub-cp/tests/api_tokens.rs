//! API token tests (issue #18, ADR-0023).

mod common;

use axum::http::StatusCode;
use restichub_db::{PLATFORM_TENANT_ID, TenantId, api_tokens};
use serde_json::{Value, json};
use time::OffsetDateTime;

use common::{app, bearer_request, create_tenant, request, seed_api_token, seed_session, send};

/// Creates an Admin session.
async fn admin_cookie(db: &restichub_db::test_util::TestDatabase, tenant: TenantId) -> String {
    seed_session(db.database(), tenant, "admin", "admin").await
}

/// Creates a token with an explicit expiry for a tenant.
async fn token_with_expiry(
    db: &restichub_db::Database,
    tenant: TenantId,
    role: &str,
    expires_at: OffsetDateTime,
) -> String {
    let creator = common::create_user(db, tenant, "creator", "phc", "admin").await;
    let (value, prefix) = restichub_cp::token::random_api_token().unwrap_or_default();
    let hash = restichub_cp::token::hash_token(&value);
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    api_tokens::create(
        &mut tx,
        &api_tokens::NewApiToken {
            name: "expiring",
            token_hash: &hash,
            display_prefix: &prefix,
            role: restichub_db::Role::parse(role).unwrap_or(restichub_db::Role::Viewer),
            created_by: creator,
            expires_at: Some(expires_at),
        },
    )
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
    value
}

#[tokio::test]
async fn an_admin_creates_lists_and_revokes_a_token() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "tokens").await;
    let cookie = admin_cookie(&db, tenant).await;
    let app = app(db.database());

    // Create.
    let body = json!({"name": "ci", "role": "operator"});
    let (status, _, created) = send(
        &app,
        request("POST", "/api/v1/api-tokens", Some(&cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let value = created
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let id = created
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    assert!(value.starts_with("rh_"));
    assert_eq!(
        created.get("display_prefix").and_then(Value::as_str),
        value.get(3..11)
    );

    // The token works for a request.
    let (status, _, me) = send(&app, bearer_request("GET", "/api/v1/auth/me", &value, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me.get("role").and_then(Value::as_str), Some("operator"));

    // List.
    let (status, _, page) = send(
        &app,
        request("GET", "/api/v1/api-tokens", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no items"));
    assert_eq!(items.len(), 1);
    assert!(items.iter().all(|item| item.get("token").is_none()));

    // Revoke.
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/api-tokens/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // The revoked token is refused.
    let (status, _, _) = send(&app, bearer_request("GET", "/api/v1/auth/me", &value, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // A second revoke is idempotent.
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/api-tokens/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    db.cleanup().await;
}

#[tokio::test]
async fn a_token_cannot_create_a_higher_role_token() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "escalate").await;
    let admin_token = seed_api_token(db.database(), tenant, "admin").await;
    let app = app(db.database());

    // An Admin token cannot create a Platform Admin token.
    let body = json!({"name": "high", "role": "platform_admin"});
    let (status, _, _) = send(
        &app,
        bearer_request("POST", "/api/v1/api-tokens", &admin_token, Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // It can create a token with its own role.
    let body = json!({"name": "same", "role": "admin"});
    let (status, _, _) = send(
        &app,
        bearer_request("POST", "/api/v1/api-tokens", &admin_token, Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // A Viewer token cannot create an Operator token.
    let viewer_token = seed_api_token(db.database(), tenant, "viewer").await;
    let body = json!({"name": "higher", "role": "operator"});
    let (status, _, _) = send(
        &app,
        bearer_request("POST", "/api/v1/api-tokens", &viewer_token, Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_02_a_lower_role_token_is_refused_in_the_platform_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let app = app(db.database());
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/api-tokens",
            Some(&cookie),
            Some(json!({"name": "lower", "role": "viewer"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
}

#[tokio::test]
async fn an_expired_token_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "expired").await;
    let value = token_with_expiry(
        db.database(),
        tenant,
        "admin",
        OffsetDateTime::now_utc() - time::Duration::hours(1),
    )
    .await;
    let app = app(db.database());

    let (status, _, _) = send(&app, bearer_request("GET", "/api/v1/auth/me", &value, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    db.cleanup().await;
}
