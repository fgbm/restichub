//! The completeness of a plan and the order of the parameter values (FR-PF-10, FR-PF-11,
//! AC-PF-06 to AC-PF-08, ADR-0047).

#![allow(clippy::expect_used)]

mod common;
mod parameters_support;

use axum::http::StatusCode;
use serde_json::json;

use common::{
    app_with_kek, create_tenant_with_key, request, seed_agent, seed_repository, seed_secret,
    seed_session, send,
};
use parameters_support::{
    create_plan, create_profile, host_values, put_host_values, repository_fixture, row,
};

/// AC-PF-06: a plan without a required value is refused and changes nothing.
#[tokio::test]
async fn ac_pf_06_a_plan_without_a_required_parameter_changes_nothing() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let profile = create_profile(
        &app,
        &cookie,
        json!({
            "name": "pg",
            "paths": {"all": ["/data/{{param.pg_dir}}"]},
            "parameters": [
                {"name": "pg_dir", "kind": "text", "required": true},
            ],
        }),
    )
    .await;

    // The host holds no value, so the plan is refused with the names of the parameters.
    let (status, body) = create_plan(
        &app,
        &cookie,
        "nightly",
        &profile,
        agent,
        &repository,
        json!([]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "invalid_argument");
    assert_eq!(body["reason"], "parameter_missing");
    assert_eq!(body["parameters"], json!(["pg_dir"]));

    let (_, _, plans) = send(&app, request("GET", "/api/v1/plans", Some(&cookie), None)).await;
    assert_eq!(plans["items"].as_array().expect("a list").len(), 0);

    // A value on the host completes the plan, so the create succeeds.
    let (status, body) = put_host_values(
        &app,
        &cookie,
        agent,
        json!([{"name": "pg_dir", "kind": "text", "value": "/srv/pg"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = create_plan(
        &app,
        &cookie,
        "nightly",
        &profile,
        agent,
        &repository,
        json!([]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    db.cleanup().await;
}

/// AC-PF-07: a profile change and a host change that leave a plan empty are refused, and a value of
/// AC-PF-07: a profile change and a host change that leave a plan empty are refused, and a value of
/// the wrong kind is its own code.
#[tokio::test]
async fn ac_pf_07_a_change_that_leaves_a_plan_without_a_value_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let profile = create_profile(
        &app,
        &cookie,
        json!({
            "name": "pg",
            "paths": {"all": ["/data/{{param.pg_dir}}"]},
            "parameters": [
                {"name": "pg_dir", "kind": "text", "default_value": "/srv/pg"},
            ],
        }),
    )
    .await;
    let (status, body) = create_plan(
        &app,
        &cookie,
        "nightly",
        &profile,
        agent,
        &repository,
        json!([]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let plan = body["plan"]["id"]
        .as_str()
        .expect("the identifier")
        .to_owned();

    // A new required parameter without a value on the host leaves the plan empty.
    let (status, _, body) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/profiles/{profile}"),
            Some(&cookie),
            Some(json!({
                "paths": {"all": ["/data/{{param.pg_dir}}"]},
                "parameters": [
                    {"name": "pg_dir", "kind": "text", "default_value": "/srv/pg"},
                    {"name": "pg_user", "kind": "text", "required": true},
                ],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["reason"], "parameter_missing_in_plans");
    assert_eq!(body["parameters"], json!(["pg_user"]));
    assert_eq!(body["blockers"]["total"], 1);
    assert_eq!(body["blockers"]["items"][0]["kind"], "plan");
    assert_eq!(body["blockers"]["items"][0]["id"], plan.as_str());

    // The refused update changed nothing: the profile keeps its declarations.
    let (_, _, view) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/profiles/{profile}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(view["parameters"].as_array().expect("a list").len(), 1);

    // The host takes the value and the profile saves.
    let (status, body) = put_host_values(
        &app,
        &cookie,
        agent,
        json!([
            {"name": "pg_dir", "kind": "text", "value": "/srv/pg"},
            {"name": "pg_user", "kind": "text", "value": "postgres"},
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _, body) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/profiles/{profile}"),
            Some(&cookie),
            Some(json!({
                "paths": {"all": ["/data/{{param.pg_dir}}"]},
                "parameters": [
                    {"name": "pg_dir", "kind": "text", "default_value": "/srv/pg"},
                    {"name": "pg_user", "kind": "text", "required": true},
                ],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // A cleared value leaves the plan empty again.
    let (status, body) = put_host_values(&app, &cookie, agent, json!([])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["reason"], "parameter_missing_in_plans");
    // The refused change keeps the values that the host held.
    let table = host_values(&app, &cookie, agent).await;
    assert_eq!(row(&table, "pg_user")["value"]["value"], "postgres");

    // A value whose kind differs from the declaration has its own code.
    let canary = "CANARY-hook-secret-1";
    let (status, _, secret) = send(
        &app,
        request(
            "POST",
            "/api/v1/secrets",
            Some(&cookie),
            Some(json!({"name": "pg-password", "purpose": "hook_secret", "value": canary})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{secret}");
    let secret_id = secret["id"].as_str().expect("the identifier").to_owned();
    let (status, body) = put_host_values(
        &app,
        &cookie,
        agent,
        json!([{"name": "pg_user", "kind": "secret", "secret_id": secret_id}]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["reason"], "parameter_kind_mismatch");
    assert_eq!(body["parameters"], json!(["pg_user"]));

    db.cleanup().await;
}

/// AC-PF-08: the value comes from the plan, then from the host, then from the default.
#[tokio::test]
async fn ac_pf_08_the_value_comes_from_the_plan_then_the_host_then_the_default() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let with_host = repository_fixture(db.database(), tenant).await;
    let with_plan = {
        let secret = seed_secret(db.database(), tenant, "plan.password.2").await;
        seed_repository(db.database(), tenant, "repo-2", secret)
            .await
            .to_string()
    };
    let profile = create_profile(
        &app,
        &cookie,
        json!({
            "name": "pg",
            "paths": {"all": ["/data/{{param.pg_dir}}"]},
            "parameters": [
                {"name": "pg_dir", "kind": "text", "default_value": "/default"},
            ],
        }),
    )
    .await;
    let (status, body) = create_plan(
        &app,
        &cookie,
        "host",
        &profile,
        agent,
        &with_host,
        json!([]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, body) = create_plan(
        &app,
        &cookie,
        "plan",
        &profile,
        agent,
        &with_plan,
        json!([{"name": "pg_dir", "kind": "text", "value": "/plan"}]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let preview = |repository: String| {
        let app = app.clone();
        let cookie = cookie.clone();
        let profile = profile.clone();
        async move {
            let (status, _, body) = send(
                &app,
                request(
                    "POST",
                    "/api/v1/profiles/preview",
                    Some(&cookie),
                    Some(json!({
                        "profile_id": profile,
                        "agent_id": agent.to_string(),
                        "repository_id": repository,
                        "name": "pg",
                        "paths": {"all": ["/data/{{param.pg_dir}}"]},
                        "parameters": [
                            {"name": "pg_dir", "kind": "text", "default_value": "/default"},
                        ],
                    })),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            body["steps"]
                .as_array()
                .expect("the steps")
                .iter()
                .find(|step| step["kind"] == "backup")
                .and_then(|step| step["command"].as_str())
                .expect("the backup command")
                .to_owned()
        }
    };

    // The default fills the path while the host holds no value.
    assert!(preview(with_host.clone()).await.contains("/data//default"));

    let (status, body) = put_host_values(
        &app,
        &cookie,
        agent,
        json!([{"name": "pg_dir", "kind": "text", "value": "/host"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The host value wins over the default, and the plan value wins over the host value.
    assert!(preview(with_host.clone()).await.contains("/data//host"));
    assert!(preview(with_plan.clone()).await.contains("/data//plan"));

    db.cleanup().await;
}
