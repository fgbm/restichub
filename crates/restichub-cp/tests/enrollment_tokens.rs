//! The enrollment token endpoints (ADR-0026, FR-ID-01, FR-ID-05, FR-TN-03).

mod common;

use axum::http::StatusCode;
use restichub_db::PLATFORM_TENANT_ID;
use restichub_db::enrollment_tokens::{self, ConsumeOutcome};
use serde_json::{Value, json};
use time::OffsetDateTime;

use common::{app, create_tenant, request, seed_session, send};

#[tokio::test]
async fn an_admin_creates_lists_and_revokes_an_unused_token() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app(db.database());

    // The create returns the value once and the metadata.
    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/enrollment-tokens",
            Some(&cookie),
            Some(json!({"labels": ["web"], "ttl_seconds": 900})),
        ),
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
    assert!(value.starts_with("re_"), "the token prefix: {value}");
    assert_eq!(
        created.get("display_prefix").and_then(Value::as_str),
        value.get(3..11)
    );
    assert_eq!(
        created
            .get("labels")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );

    // The list holds the metadata only. No endpoint returns the value.
    let (status, _, page) = send(
        &app,
        request("GET", "/api/v1/enrollment-tokens", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no items"));
    assert_eq!(items.len(), 1);
    assert!(items.iter().all(|item| item.get("token").is_none()));
    assert!(!page.to_string().contains(&value));

    // Revoke the unused token.
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/enrollment-tokens/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // A second revoke finds no token (the row is removed).
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/enrollment-tokens/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The audit log holds the create and the revoke.
    let (status, _, page) = send(
        &app,
        request(
            "GET",
            "/api/v1/audit?action=enrollment_token_create",
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        page.get("items").and_then(Value::as_array).map(Vec::len),
        Some(1)
    );
    let (status, _, page) = send(
        &app,
        request(
            "GET",
            "/api/v1/audit?action=enrollment_token_revoke",
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        page.get("items").and_then(Value::as_array).map(Vec::len),
        Some(1)
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_01_a_used_token_cannot_be_revoked() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-used").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app(db.database());

    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/enrollment-tokens",
            Some(&cookie),
            Some(json!({"labels": ["host"]})),
        ),
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

    // The enrollment exchange spends the token. The test drives the database function directly.
    let hash = restichub_cp::token::hash_token(&value);
    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        enrollment_tokens::consume(&mut tx, &hash, OffsetDateTime::now_utc())
            .await
            .unwrap_or_else(|e| panic!("{e}")),
        ConsumeOutcome::Consumed
    );
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));

    // A used token gives HTTP 409 and changes nothing.
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/enrollment-tokens/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    db.cleanup().await;
}

#[tokio::test]
async fn a_viewer_reads_the_metadata_and_a_bad_ttl_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-roles").await;
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let app = app(db.database());

    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/enrollment-tokens", Some(&viewer), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Refused for a Viewer on both writes.
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/enrollment-tokens",
            Some(&viewer),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!(
                "/api/v1/enrollment-tokens/{}",
                restichub_db::EnrollmentTokenId::new()
            ),
            Some(&viewer),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A time to live outside the accepted range is refused.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/enrollment-tokens",
            Some(&admin),
            Some(json!({"ttl_seconds": 5})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        problem.get("field").and_then(Value::as_str),
        Some("ttl_seconds")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_platform_tenant_gets_403_on_the_enrollment_token_endpoints() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let app = app(db.database());
    let id = restichub_db::EnrollmentTokenId::new();

    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/enrollment-tokens", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/enrollment-tokens",
            Some(&cookie),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/enrollment-tokens/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
}
