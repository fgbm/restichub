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

    // A second revoke finds the revoked row and gives HTTP 409 with its own code.
    let (status, _, problem) = send(
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
    assert_eq!(problem["code"], "token_already_revoked");

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

    // A used token gives HTTP 409 with the code `token_used` and changes nothing.
    let (status, _, problem) = send(
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
    assert_eq!(problem["code"], "token_used");

    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_01_an_expired_token_gives_the_token_expired_code_on_revoke() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-expired").await;
    let admin = common::create_user(db.database(), tenant, "admin", "phc", "admin").await;
    let cookie = common::session_cookie(db.database(), tenant, admin).await;
    let app = app(db.database());

    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let id = enrollment_tokens::create(
        &mut tx,
        &enrollment_tokens::NewEnrollmentToken {
            token_hash: &[7u8; 32],
            display_prefix: "expired0",
            labels: &[],
            expires_at: OffsetDateTime::now_utc() - time::Duration::hours(1),
            created_by: admin,
            replaces_agent_id: None,
        },
    )
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));

    let (status, _, problem) = send(
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
    assert_eq!(problem["code"], "token_expired");

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
async fn ac_id_01_a_revoked_token_is_refused_by_the_exchange() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-revoke").await;
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

    // An Admin revokes the unused token (a soft change).
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

    // The metadata keeps the row and holds the revoke.
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
    assert!(items[0].get("revoked_at").and_then(Value::as_str).is_some());

    // Refuse the revoked token at the exchange.
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
        ConsumeOutcome::Revoked
    );
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));

    // The audit log holds the revoke with the revoking user.
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
async fn a_default_token_expires_in_one_hour() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-ttl").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app(db.database());

    // No ttl_seconds: the default is one hour (ADR-0032).
    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/enrollment-tokens",
            Some(&cookie),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let expires_at = created
        .get("expires_at")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let created_at = created
        .get("created_at")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let expires_at =
        OffsetDateTime::parse(&expires_at, &time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|e| panic!("{e}"));
    let created_at =
        OffsetDateTime::parse(&created_at, &time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        (expires_at - created_at).whole_seconds(),
        3600,
        "the default time to live is one hour"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_labels_are_deduplicated_and_a_bad_label_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-labels").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app(db.database());

    // Duplicates are removed.
    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/enrollment-tokens",
            Some(&cookie),
            Some(json!({"labels": ["web", "web", "prod"]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let labels = created
        .get("labels")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no labels"));
    assert_eq!(labels.len(), 2);

    // A label outside `a-z`, `0-9` and `-` is refused.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/enrollment-tokens",
            Some(&cookie),
            Some(json!({"labels": ["Web"]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(problem.get("field").and_then(Value::as_str), Some("labels"));

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
