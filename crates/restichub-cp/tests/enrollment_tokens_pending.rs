//! The pending filter of the enrollment token list and the read of one token (FR-ID-01).

mod common;

use axum::http::StatusCode;
use restichub_db::enrollment_tokens::{self, ConsumeOutcome};
use serde_json::{Value, json};
use time::OffsetDateTime;

use common::{app, create_tenant, request, seed_session, send};

fn ids(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .unwrap_or_else(|| panic!("no items: {page}"))
        .iter()
        .map(|item| item["id"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[tokio::test]
async fn fr_id_01_the_pending_list_holds_only_tokens_that_can_still_enroll() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-pending").await;
    let admin = common::create_user(db.database(), tenant, "admin", "phc", "admin").await;
    let cookie = common::session_cookie(db.database(), tenant, admin).await;
    let app = app(db.database());
    let create = || {
        request(
            "POST",
            "/api/v1/enrollment-tokens",
            Some(&cookie),
            Some(json!({"ttl_seconds": 900})),
        )
    };

    let (_, _, pending) = send(&app, create()).await;
    let (_, _, used) = send(&app, create()).await;
    let (_, _, revoked) = send(&app, create()).await;
    let id = |token: &Value| token["id"].as_str().unwrap_or_default().to_owned();

    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let hash = restichub_cp::token::hash_token(used["token"].as_str().unwrap_or_default());
    assert_eq!(
        enrollment_tokens::consume(&mut tx, &hash, OffsetDateTime::now_utc())
            .await
            .unwrap_or_else(|e| panic!("{e}")),
        ConsumeOutcome::Consumed
    );
    enrollment_tokens::create(
        &mut tx,
        &enrollment_tokens::NewEnrollmentToken {
            token_hash: &[9u8; 32],
            display_prefix: "expired1",
            labels: &[],
            expires_at: OffsetDateTime::now_utc() - time::Duration::hours(1),
            created_by: admin,
            replaces_agent_id: None,
        },
    )
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/enrollment-tokens/{}", id(&revoked)),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, page) = send(
        &app,
        request(
            "GET",
            "/api/v1/enrollment-tokens?pending=true",
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(ids(&page), vec![id(&pending)]);
    let (_, _, page) = send(
        &app,
        request("GET", "/api/v1/enrollment-tokens", Some(&cookie), None),
    )
    .await;
    assert_eq!(ids(&page).len(), 4);

    // The read of one token shows a used token, which the pending list leaves out.
    let (status, _, token) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/enrollment-tokens/{}", id(&used)),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{token}");
    assert_eq!(token["id"], used["id"]);
    assert!(token["used_at"].is_string(), "{token}");
    assert!(token.get("token").is_none());

    db.cleanup().await;
}

#[tokio::test]
async fn fr_id_01_the_read_of_a_token_stays_in_its_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "enroll-read").await;
    let other = create_tenant(db.database(), "enroll-read-other").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let stranger = seed_session(db.database(), other, "viewer", "viewer").await;
    let app = app(db.database());

    let (_, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/enrollment-tokens",
            Some(&cookie),
            Some(json!({"ttl_seconds": 900})),
        ),
    )
    .await;
    let uri = format!(
        "/api/v1/enrollment-tokens/{}",
        created["id"].as_str().unwrap_or_default()
    );
    let (status, _, _) = send(&app, request("GET", &uri, Some(&stranger), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, problem) = send(
        &app,
        request(
            "GET",
            "/api/v1/enrollment-tokens/not-an-id",
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "id");

    db.cleanup().await;
}
