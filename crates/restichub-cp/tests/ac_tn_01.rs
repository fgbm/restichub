//! AC-TN-01: a cross-tenant object identifier returns 404.
//!
//! The test is table-driven. It reads [`ROUTES`] and covers every route with an `{id}`
//! placeholder. A new `{id}` route without a fixture stops the test.

mod common;

use axum::http::StatusCode;
use restichub_cp::routes::ROUTES;
use serde_json::json;

use common::{
    app, create_tenant, create_tenant_with_key, create_user, list_tokens, list_users, request,
    seed_api_token, seed_secret, seed_session, send, token_revoked, user_disabled,
};

/// The fixture of one `{id}` route: the body of an unsafe method.
///
/// The key is `(method, path)`. The test fails when a `{id}` route has no entry.
const FIXTURES: &[(&str, &str)] = &[
    ("get", "/api/v1/platform/tenants/{id}"),
    ("post", "/api/v1/platform/tenants/{id}/suspend"),
    ("post", "/api/v1/platform/tenants/{id}/resume"),
    ("delete", "/api/v1/platform/tenants/{id}"),
    ("get", "/api/v1/users/{id}"),
    ("patch", "/api/v1/users/{id}"),
    ("delete", "/api/v1/users/{id}"),
    ("post", "/api/v1/users/{id}/password"),
    ("delete", "/api/v1/api-tokens/{id}"),
    ("get", "/api/v1/secrets/{id}"),
    ("put", "/api/v1/secrets/{id}/value"),
    ("delete", "/api/v1/secrets/{id}"),
];

fn body_for(method: &str, path: &str) -> Option<serde_json::Value> {
    match (method, path) {
        ("patch", _) => Some(json!({"display_name": "changed"})),
        ("put", _) => Some(json!({"value": "a new value"})),
        ("post", path) if path.ends_with("/password") => {
            Some(json!({"new_password": "a long enough password"}))
        }
        _ => None,
    }
}

#[tokio::test]
async fn ac_tn_01_api_cross_tenant_ids_return_404() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant_a = create_tenant(db.database(), "cross-a").await;
    let tenant_b = create_tenant_with_key(db.database(), "cross-b").await;

    // Tenant A has an Admin session.
    let cookie_a = seed_session(db.database(), tenant_a, "admin-a", "admin").await;
    // Tenant B has a user, an API token and a secret.
    let user_b = create_user(db.database(), tenant_b, "bob", "phc", "viewer").await;
    seed_api_token(db.database(), tenant_b, "viewer").await;
    let token_b = list_tokens(db.database(), tenant_b)
        .await
        .first()
        .map(|token| token.id)
        .unwrap_or_default();
    let secret_b = seed_secret(db.database(), tenant_b, "cross-secret").await;

    let app = app(db.database());

    // Every route with an `{id}` placeholder has a fixture, and the other way round.
    let id_routes: Vec<(String, String)> = ROUTES
        .iter()
        .filter(|route| route.path.contains("{id}"))
        .map(|route| (route.method.to_owned(), route.path.to_owned()))
        .collect();
    let fixtures: Vec<(String, String)> = FIXTURES
        .iter()
        .map(|(method, path)| ((*method).to_owned(), (*path).to_owned()))
        .collect();
    assert_eq!(
        id_routes, fixtures,
        "a route with an {{id}} placeholder has no fixture"
    );

    // Each fixture uses an identifier of tenant B. A tenant route returns 404 for tenant A. A
    // platform route returns 403 for tenant A, because tenant A is not the platform tenant.
    for (method, path) in FIXTURES {
        let id = if path.contains("api-tokens") {
            token_b.to_string()
        } else if path.contains("secrets") {
            secret_b.to_string()
        } else {
            user_b.to_string()
        };
        let uri = path.replace("{id}", &id);
        let http_method = method.to_ascii_uppercase();
        let body = body_for(method, path);
        let (status, _, _) = send(&app, request(&http_method, &uri, Some(&cookie_a), body)).await;
        let expected = if path.starts_with("/api/v1/platform/") {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::NOT_FOUND
        };
        assert_eq!(
            status, expected,
            "{http_method} {path} with an identifier of tenant B must return {expected}"
        );
    }

    assert!(
        !user_disabled(db.database(), tenant_b, user_b).await,
        "the user of tenant B must not be disabled"
    );
    assert!(
        !token_revoked(db.database(), tenant_b, token_b).await,
        "the token of tenant B must not be revoked"
    );
    assert!(
        list_users(db.database(), tenant_b)
            .await
            .iter()
            .any(|user| user.id == user_b),
        "the user of tenant B must exist"
    );
    let secret_exists = {
        let mut tx = db
            .database()
            .begin_tenant(tenant_b)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let secret = restichub_db::secrets::get(&mut tx, secret_b)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        tx.commit().await.unwrap_or_else(|error| panic!("{error}"));
        secret.is_some()
    };
    assert!(secret_exists, "the secret of tenant B must exist");

    db.cleanup().await;
}
