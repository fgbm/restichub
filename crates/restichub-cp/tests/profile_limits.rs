//! The job limits of a profile (ADR-0039, FR-PF-01).

#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use restichub_db::TenantId;
use serde_json::{Value, json};

use common::{
    app_with_kek, create_tenant_with_key, request, seed_repository, seed_secret, seed_session, send,
};

async fn repository_fixture(db: &restichub_db::Database, tenant: TenantId) -> String {
    let secret = seed_secret(db, tenant, "profile.password").await;
    seed_repository(db, tenant, "profile-repo", secret)
        .await
        .to_string()
}

fn body(name: &str, repository_id: &str, options: Value) -> Value {
    json!({
        "name": name,
        "repository_id": repository_id,
        "paths": {"all": ["/srv"]},
        "options": options,
    })
}

/// ADR-0039: a profile without limits gets the defaults, and the settings are stored.
#[tokio::test]
async fn ac_ag_12_a_profile_stores_its_job_limits() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let repository_id = repository_fixture(db.database(), tenant).await;

    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(body("defaults", &repository_id, json!({}))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let options = &created["profile"]["options"];
    assert_eq!(options["stall_timeout_minutes"], 60);
    assert_eq!(options["max_duration_minutes"], 2880);

    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(body(
                "limited",
                &repository_id,
                json!({
                    "stall_timeout_minutes": 120,
                    "max_duration_minutes": 1440,
                }),
            )),
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
    let options = &read["options"];
    assert_eq!(options["stall_timeout_minutes"], 120);
    assert_eq!(options["max_duration_minutes"], 1440);

    db.cleanup().await;
}

/// ADR-0039: the save refuses a stall timeout or a maximum duration out of range.
#[tokio::test]
async fn ac_ag_12_the_save_refuses_a_job_limit_out_of_range() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let repository_id = repository_fixture(db.database(), tenant).await;

    for (options, field) in [
        (
            json!({"stall_timeout_minutes": 1}),
            "options.stall_timeout_minutes",
        ),
        (
            json!({"stall_timeout_minutes": 0}),
            "options.stall_timeout_minutes",
        ),
        (
            json!({"stall_timeout_minutes": 100_000}),
            "options.stall_timeout_minutes",
        ),
        (
            json!({"max_duration_minutes": 100_000}),
            "options.max_duration_minutes",
        ),
    ] {
        let (status, _, problem) = send(
            &app,
            request(
                "POST",
                "/api/v1/profiles",
                Some(&cookie),
                Some(body("bad-limit", &repository_id, options)),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
        assert_eq!(problem["field"], field);
    }

    db.cleanup().await;
}

/// ADR-0039: an update that names no limit keeps the stored limits.
#[tokio::test]
async fn ac_ag_12_an_update_without_limit_fields_keeps_the_stored_limits() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let repository_id = repository_fixture(db.database(), tenant).await;

    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(body(
                "kept",
                &repository_id,
                json!({"stall_timeout_minutes": 120, "max_duration_minutes": 0}),
            )),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["profile"]["id"].as_str().expect("an id");

    let (status, _, updated) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/profiles/{id}"),
            Some(&cookie),
            Some(body(
                "kept",
                &repository_id,
                json!({"exclude_caches": true}),
            )),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    let options = &updated["profile"]["options"];
    assert_eq!(options["exclude_caches"], true);
    assert_eq!(options["stall_timeout_minutes"], 120);
    assert_eq!(options["max_duration_minutes"], 0);

    db.cleanup().await;
}

/// ADR-0039: the API gives the defaults that a new profile gets.
#[tokio::test]
async fn ac_ag_12_the_api_gives_the_default_job_limits() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let (status, _, defaults) = send(
        &app,
        request("GET", "/api/v1/profiles/defaults", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{defaults}");
    assert_eq!(defaults["stall_timeout_minutes"], 60);
    assert_eq!(defaults["max_duration_minutes"], 2880);

    db.cleanup().await;
}
