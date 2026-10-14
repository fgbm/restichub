//! The plan API: the create, the read, the update, the delete and the next-runs preview
//! (FR-SC-01, FR-SC-07, FR-SC-11).
//!
//! The tests cover the create, the read, the update, the delete, the schedule and reference
//! validation, the next-runs preview, the pagination and the role matrix.

#![allow(clippy::expect_used)]

mod common;

use axum::Router;
use axum::http::StatusCode;
use restichub_db::{Database, PLATFORM_TENANT_ID, TenantId};
use serde_json::{Value, json};

use common::{
    app_with_kek, create_tenant_with_key, request, seed_agent, seed_profile, seed_repository,
    seed_secret, seed_session, send,
};

/// A repository, a profile and an agent of a tenant, for a plan.
async fn plan_fixture(db: &Database, tenant: TenantId) -> (String, String, String) {
    let secret = seed_secret(db, tenant, "plan.password").await;
    let repository = seed_repository(db, tenant, "plan-repo", secret).await;
    let profile = seed_profile(db, tenant, "plan-profile", repository).await;
    let agent = seed_agent(db, tenant, "plan-host").await;
    (
        repository.to_string(),
        profile.to_string(),
        agent.to_string(),
    )
}

fn plan_body(name: &str, profile_id: &str, agent_id: &str) -> Value {
    json!({
        "name": name,
        "profile_id": profile_id,
        "agent_id": agent_id,
        "cron": "0 2 * * *",
        "time_zone": "Europe/Berlin",
        "missed_run_policy": "run_all",
    })
}

async fn create_plan(app: &Router, cookie: &str, body: Value) -> Value {
    let (status, _, response) = send(
        app,
        request("POST", "/api/v1/plans", Some(cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    response
        .get("plan")
        .cloned()
        .unwrap_or_else(|| panic!("the response holds no plan: {response}"))
}

#[tokio::test]
async fn the_plan_crud_writes_the_audit_and_keeps_the_view() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "plan-crud").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "operator", "operator").await;
    let (_repository, profile_id, agent_id) = plan_fixture(db.database(), tenant).await;

    let audit_before = common::count(db.database(), tenant, "audit_log").await;

    let plan = create_plan(&app, &cookie, plan_body("nightly", &profile_id, &agent_id)).await;
    let plan_id = plan["id"]
        .as_str()
        .expect("the plan holds an id")
        .to_owned();
    assert_eq!(plan["name"], "nightly");
    assert_eq!(plan["profile_id"], profile_id.as_str());
    assert_eq!(plan["agent_hostname"], "plan-host");
    assert_eq!(plan["time_zone"], "Europe/Berlin");
    assert_eq!(plan["missed_run_policy"], "run_all");
    assert_eq!(plan["enabled"], true);
    // A new plan gets its first fire time from the schedule.
    assert!(
        plan["next_run_at"].is_string(),
        "the created plan holds a next run time"
    );
    assert_eq!(
        common::count(db.database(), tenant, "audit_log").await,
        audit_before + 1,
        "the create writes one audit record"
    );

    // A read of the plan returns the same view as the create.
    let (status, _, read) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/plans/{plan_id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(read["id"], plan_id.as_str());

    // The update changes the schedule and the policy.
    let (status, _, updated) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/plans/{plan_id}"),
            Some(&cookie),
            Some(json!({
                "profile_id": profile_id,
                "agent_id": agent_id,
                "cron": "30 3 * * *",
                "time_zone": "UTC",
                "missed_run_policy": "latest_only",
                "enabled": false,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(
        updated["plan"]["name"], "nightly",
        "an absent name keeps the name"
    );
    assert_eq!(updated["plan"]["cron"], "30 3 * * *");
    assert_eq!(updated["plan"]["missed_run_policy"], "latest_only");
    assert_eq!(updated["plan"]["enabled"], false);
    // A disabled plan keeps no next fire time, so the scheduler skips it.
    assert!(updated["plan"]["next_run_at"].is_null());

    // A delete removes the plan, and a later read is not found.
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/plans/{plan_id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/plans/{plan_id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The create, the update and the delete each write one audit record.
    assert_eq!(
        common::count(db.database(), tenant, "audit_log").await,
        audit_before + 3
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_plan_create_validates_the_schedule_and_the_references() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "plan-valid").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "operator", "operator").await;
    let (repository_id, profile_id, agent_id) = plan_fixture(db.database(), tenant).await;
    let unknown = uuid::Uuid::now_v7().to_string();

    let cases = [
        (
            "a wrong cron expression",
            json!({
                "name": "bad-cron",
                "profile_id": profile_id,
                "agent_id": agent_id,
                "cron": "* * * *",
                "time_zone": "UTC",
                "missed_run_policy": "run_all",
            }),
            "cron",
        ),
        (
            "an unknown time zone",
            json!({
                "name": "bad-zone",
                "profile_id": profile_id,
                "agent_id": agent_id,
                "cron": "0 2 * * *",
                "time_zone": "Mars/Olympus",
                "missed_run_policy": "run_all",
            }),
            "time_zone",
        ),
        (
            "a profile of another tenant",
            json!({
                "name": "bad-profile",
                "profile_id": unknown,
                "agent_id": agent_id,
                "cron": "0 2 * * *",
                "time_zone": "UTC",
                "missed_run_policy": "run_all",
            }),
            "profile_id",
        ),
        (
            "an agent of another tenant",
            json!({
                "name": "bad-agent",
                "profile_id": profile_id,
                "agent_id": unknown,
                "cron": "0 2 * * *",
                "time_zone": "UTC",
                "missed_run_policy": "run_all",
            }),
            "agent_id",
        ),
    ];

    for (case, body, field) in cases {
        let (status, _, problem) = send(
            &app,
            request("POST", "/api/v1/plans", Some(&cookie), Some(body)),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{case}: {problem}");
        assert_eq!(
            problem["field"], field,
            "{case} must name the field: {problem}"
        );
    }

    // A profile whose paths exist only for Windows does not compile for a Linux agent.
    let (status, _, profile) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "windows-only",
                "repository_id": repository_id,
                "paths": {"windows": ["C:\\Data"]},
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{profile}");
    let windows_profile = profile["profile"]["id"]
        .as_str()
        .expect("the profile holds an id")
        .to_owned();
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/plans",
            Some(&cookie),
            Some(json!({
                "name": "windows-plan",
                "profile_id": windows_profile,
                "agent_id": agent_id,
                "cron": "0 2 * * *",
                "time_zone": "UTC",
                "missed_run_policy": "run_all",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "paths", "{problem}");

    db.cleanup().await;
}

#[tokio::test]
async fn the_next_runs_preview_returns_the_fire_times() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "plan-preview").await;
    let app = app_with_kek(db.database());
    let operator = seed_session(db.database(), tenant, "operator", "operator").await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let (_repository, profile_id, agent_id) = plan_fixture(db.database(), tenant).await;
    let plan = create_plan(
        &app,
        &operator,
        plan_body("preview", &profile_id, &agent_id),
    )
    .await;
    let plan_id = plan["id"]
        .as_str()
        .expect("the plan holds an id")
        .to_owned();

    // A Viewer may read the next-runs preview of a plan.
    let (status, _, preview) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/plans/{plan_id}/next-runs?count=5"),
            Some(&viewer),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["time_zone"], "Europe/Berlin");
    let runs = preview["runs"]
        .as_array()
        .expect("the preview holds runs")
        .clone();
    assert_eq!(runs.len(), 5);
    // Each run is an RFC 3339 instant, strictly after the one before.
    let parsed: Vec<time::OffsetDateTime> = runs
        .iter()
        .map(|run| {
            time::OffsetDateTime::parse(
                run.as_str().expect("a run is a string"),
                &time::format_description::well_known::Rfc3339,
            )
            .expect("a run parses")
        })
        .collect();
    assert!(parsed.windows(2).all(|pair| pair[0] < pair[1]));

    // A count of zero or above the maximum is refused.
    for count in ["0", "21"] {
        let (status, _, _) = send(
            &app,
            request(
                "GET",
                &format!("/api/v1/plans/{plan_id}/next-runs?count={count}"),
                Some(&viewer),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "count={count}");
    }

    db.cleanup().await;
}

#[tokio::test]
async fn the_plan_list_is_paginated() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "plan-page").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "operator", "operator").await;
    let (_repository, profile_id, agent_id) = plan_fixture(db.database(), tenant).await;

    for name in ["plan-a", "plan-b", "plan-c"] {
        create_plan(&app, &cookie, plan_body(name, &profile_id, &agent_id)).await;
    }

    let (status, _, first) = send(
        &app,
        request("GET", "/api/v1/plans?limit=2", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["items"].as_array().map(Vec::len), Some(2));
    let cursor = first["next_cursor"]
        .as_str()
        .expect("the first page holds a cursor")
        .to_owned();

    let (status, _, second) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/plans?limit=2&cursor={cursor}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["items"].as_array().map(Vec::len), Some(1));
    assert!(second["next_cursor"].is_null());

    db.cleanup().await;
}

#[tokio::test]
async fn the_plan_role_matrix_and_the_platform_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "plan-roles").await;
    let app = app_with_kek(db.database());
    let (_repository, profile_id, agent_id) = plan_fixture(db.database(), tenant).await;

    // A Viewer may read the list of the plans of the tenant.
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let (status, _, _) = send(&app, request("GET", "/api/v1/plans", Some(&viewer), None)).await;
    assert_eq!(status, StatusCode::OK);
    // A Viewer cannot create a plan (requirements section 2).
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/plans",
            Some(&viewer),
            Some(plan_body("viewer-plan", &profile_id, &agent_id)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // An Operator may create a plan (requirements section 2).
    let operator = seed_session(db.database(), tenant, "operator", "operator").await;
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/plans",
            Some(&operator),
            Some(plan_body("operator-plan", &profile_id, &agent_id)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // The platform tenant refuses a plan operation with HTTP 403 (ADR-0024).
    let admin = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let (status, _, _) = send(&app, request("GET", "/api/v1/plans", Some(&admin), None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
}
