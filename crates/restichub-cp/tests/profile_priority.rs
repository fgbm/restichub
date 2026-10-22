//! The process and I/O priority of a profile (FR-AG-15, AC-AG-14).

#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};

use common::{app_with_kek, create_tenant_with_key, request, seed_session, send};

fn body(name: &str, options: Value) -> Value {
    json!({
        "name": name,
        "paths": {"all": ["/srv"]},
        "options": options,
    })
}

/// FR-AG-15: a profile without a priority runs at the normal priority, and the low priority is
/// stored, read back and kept by an update that names no priority.
#[tokio::test]
async fn ac_ag_14_a_profile_stores_its_priority() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(body("defaults", json!({}))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["profile"]["options"]["priority"], "normal");

    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(body("quiet", json!({"priority": "low"}))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["profile"]["id"].as_str().expect("an id");
    let (status, _, read) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/profiles/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["options"]["priority"], "low");

    let (status, _, updated) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/profiles/{id}"),
            Some(&cookie),
            Some(body("quiet", json!({"priority": "normal"}))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["profile"]["options"]["priority"], "normal");

    db.cleanup().await;
}

/// FR-AG-15: a priority outside the closed set is refused.
#[tokio::test]
async fn ac_ag_14_the_save_refuses_an_unknown_priority() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(body("bad", json!({"priority": "realtime"}))),
        ),
    )
    .await;
    assert!(status.is_client_error(), "{status} {problem}");

    db.cleanup().await;
}
