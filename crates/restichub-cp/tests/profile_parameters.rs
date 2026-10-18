//! The profile parameters and their values (FR-PF-09 to FR-PF-12, AC-PF-06 to AC-PF-10).
//!
//! The tests cover the order of the value, the completeness of a plan, the refusals of the saves,
//! the masking of a text value and the audit record.

#![allow(clippy::expect_used)]

mod common;

use axum::Router;
use axum::http::StatusCode;
use restichub_db::{AgentId, TenantId};
use serde_json::{Value, json};

use common::{
    app_with_kek, create_tenant_with_key, request, seed_agent, seed_repository, seed_secret,
    seed_session, send,
};

/// Creates a repository that a plan names and returns its identifier as a string.
async fn repository_fixture(db: &restichub_db::Database, tenant: TenantId) -> String {
    let secret = seed_secret(db, tenant, "plan.password").await;
    seed_repository(db, tenant, "repo", secret)
        .await
        .to_string()
}

/// Creates a profile through the API and returns its identifier.
async fn create_profile(app: &Router, cookie: &str, body: Value) -> String {
    let (status, _, response) = send(
        app,
        request("POST", "/api/v1/profiles", Some(cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    response["profile"]["id"]
        .as_str()
        .expect("the identifier")
        .to_owned()
}

/// Sends a plan create and returns the status and the body.
async fn create_plan(
    app: &Router,
    cookie: &str,
    name: &str,
    profile: &str,
    agent: AgentId,
    repository: &str,
    values: Value,
) -> (StatusCode, Value) {
    let (status, _, body) = send(
        app,
        request(
            "POST",
            "/api/v1/plans",
            Some(cookie),
            Some(json!({
                "name": name,
                "profile_id": profile,
                "agent_id": agent.to_string(),
                "repository_id": repository,
                "cron": "0 2 * * *",
                "time_zone": "UTC",
                "missed_run_policy": "run_all",
                "parameter_values": values,
            })),
        ),
    )
    .await;
    (status, body)
}

/// Replaces the parameter values of a host and returns the status and the body.
async fn put_host_values(
    app: &Router,
    cookie: &str,
    agent: AgentId,
    values: Value,
) -> (StatusCode, Value) {
    let (status, _, body) = send(
        app,
        request(
            "PUT",
            &format!("/api/v1/agents/{agent}/parameters"),
            Some(cookie),
            Some(json!({"parameters": values})),
        ),
    )
    .await;
    (status, body)
}

/// Reads the parameter table of a host.
async fn host_values(app: &Router, cookie: &str, agent: AgentId) -> Value {
    let (status, _, body) = send(
        app,
        request(
            "GET",
            &format!("/api/v1/agents/{agent}/parameters"),
            Some(cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Reads one plan of the tenant.
async fn plan_view(app: &Router, cookie: &str, plan: &str) -> Value {
    let (status, _, body) = send(
        app,
        request("GET", &format!("/api/v1/plans/{plan}"), Some(cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Finds the row of one name, and fails the test when the table holds none.
fn row<'a>(table: &'a Value, name: &str) -> &'a Value {
    table["parameters"]
        .as_array()
        .expect("the parameter list")
        .iter()
        .find(|row| row["name"] == name)
        .unwrap_or_else(|| panic!("the parameter {name} is absent: {table}"))
}

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
                    &format!("/api/v1/profiles/{profile}/preview"),
                    Some(&cookie),
                    Some(json!({"agent_id": agent.to_string(), "repository_id": repository})),
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

/// AC-PF-09: a secret parameter cannot fill a template field.
#[tokio::test]
async fn ac_pf_09_a_secret_parameter_is_refused_in_every_template_field() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let parameter = json!([{"name": "pg_password", "kind": "secret", "required": true}]);

    let cases = [
        (
            json!({"all": ["/data/{{param.pg_password}}"]}),
            "paths.all[0]",
        ),
        (json!({"all": ["/data"]}), "excludes[0]"),
        (json!({"all": ["/data"]}), "exclude_files[0]"),
        (json!({"all": ["/data"]}), "tags[0]"),
    ];
    for (index, (paths, field)) in cases.iter().enumerate() {
        let value = "{{param.pg_password}}";
        let body = match index {
            0 => json!({"name": format!("p{index}"), "paths": paths, "parameters": parameter}),
            1 => {
                json!({"name": format!("p{index}"), "paths": paths, "excludes": [value], "parameters": parameter})
            }
            2 => {
                json!({"name": format!("p{index}"), "paths": paths, "exclude_files": [value], "parameters": parameter})
            }
            _ => {
                json!({"name": format!("p{index}"), "paths": paths, "tags": [value], "parameters": parameter})
            }
        };
        let (status, _, response) = send(
            &app,
            request("POST", "/api/v1/profiles", Some(&cookie), Some(body)),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
        assert_eq!(response["field"], *field, "{response}");
        assert_eq!(
            response["reason"], "parameter_secret_in_template",
            "{response}"
        );
    }

    db.cleanup().await;
}

/// AC-PF-10: a Viewer reads no text value, and the audit holds none.
#[tokio::test]
async fn ac_pf_10_a_viewer_reads_no_text_value_and_the_audit_keeps_none() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;

    let default_canary = "CANARY-default-1";
    let host_canary = "CANARY-host-2";
    let plan_canary = "CANARY-plan-3";
    let profile = create_profile(
        &app,
        &cookie,
        json!({
            "name": "pg",
            "paths": {"all": ["/data/{{param.pg_dir}}"]},
            "parameters": [
                {"name": "pg_dir", "kind": "text", "default_value": default_canary},
            ],
        }),
    )
    .await;
    let (status, body) = put_host_values(
        &app,
        &cookie,
        agent,
        json!([{"name": "pg_dir", "kind": "text", "value": host_canary}]),
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
        json!([{"name": "pg_dir", "kind": "text", "value": plan_canary}]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let plan = body["plan"]["id"]
        .as_str()
        .expect("the identifier")
        .to_owned();

    // An Admin reads every value, including the override of the plan.
    let table = host_values(&app, &cookie, agent).await;
    assert_eq!(row(&table, "pg_dir")["value"]["value"], host_canary);
    assert_eq!(
        plan_view(&app, &cookie, &plan).await["parameter_values"][0]["value"],
        plan_canary
    );

    // A Viewer reads the names, the secret names and no text value (FR-PF-12).
    let table = host_values(&app, &viewer, agent).await;
    let viewer_row = row(&table, "pg_dir");
    assert_eq!(viewer_row["value"]["value"], "");
    assert_eq!(viewer_row["hidden"], true);
    assert_eq!(viewer_row["default_value"], "");
    assert_eq!(viewer_row["needed_by"], json!(["pg"]));
    let plan_of_viewer = plan_view(&app, &viewer, &plan).await;
    assert_eq!(plan_of_viewer["parameter_values"][0]["name"], "pg_dir");
    assert_eq!(plan_of_viewer["parameter_values"][0]["value"], "");
    assert_eq!(plan_of_viewer["parameter_values"][0]["hidden"], true);
    let (_, _, profile_of_viewer) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/profiles/{profile}"),
            Some(&viewer),
            None,
        ),
    )
    .await;
    assert_eq!(profile_of_viewer["parameters"][0]["default_value"], "");
    assert_eq!(profile_of_viewer["parameters"][0]["hidden"], true);

    // The audit records name the parameters and hold no value.
    let (status, _, audit) = send(&app, request("GET", "/api/v1/audit", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{audit}");
    let text = audit.to_string();
    for canary in [default_canary, host_canary, plan_canary] {
        assert!(!text.contains(canary), "the audit must not hold {canary}");
    }
    assert!(text.contains("pg_dir"), "{text}");

    db.cleanup().await;
}

/// ADR-0047: a removed parameter keeps the host values and drops the overrides of the plan.
#[tokio::test]
async fn adr_0047_a_removed_parameter_keeps_the_host_values_and_drops_the_overrides() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let declarations = json!([
        {"name": "pg_dir", "kind": "text", "default_value": "/srv/pg"},
        {"name": "pg_old", "kind": "text", "default_value": "/srv/old"},
    ]);
    let profile = create_profile(
        &app,
        &cookie,
        json!({
            "name": "pg",
            "paths": {"all": ["/data/{{param.pg_dir}}"]},
            "parameters": declarations,
        }),
    )
    .await;
    let (status, body) = put_host_values(
        &app,
        &cookie,
        agent,
        json!([
            {"name": "pg_dir", "kind": "text", "value": "/host"},
            {"name": "pg_old", "kind": "text", "value": "/host-old"},
        ]),
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
        json!([{"name": "pg_old", "kind": "text", "value": "/plan-old"}]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let plan = body["plan"]["id"]
        .as_str()
        .expect("the identifier")
        .to_owned();

    // The update removes pg_old from the declarations of the profile.
    let (status, _, body) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/profiles/{profile}"),
            Some(&cookie),
            Some(json!({
                "paths": {"all": ["/data/{{param.pg_dir}}"]},
                "parameters": [{"name": "pg_dir", "kind": "text", "default_value": "/srv/pg"}],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The override went with the declaration; the value of the host stays and is unused.
    let view = plan_view(&app, &cookie, &plan).await;
    assert_eq!(
        view["parameter_values"].as_array().expect("a list").len(),
        0,
        "{view}"
    );
    let table = host_values(&app, &cookie, agent).await;
    assert_eq!(row(&table, "pg_old")["value"]["value"], "/host-old");
    assert_eq!(row(&table, "pg_old")["unused"], true);
    assert_eq!(row(&table, "pg_dir")["unused"], false);

    db.cleanup().await;
}

/// ADR-0047: an optional parameter without a default cannot fill a path or a tag, and a script
/// variable accepts the empty string.
#[tokio::test]
async fn adr_0047_an_optional_parameter_without_a_default_fills_no_path() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let parameter = json!([{"name": "pg_dir", "kind": "text"}]);

    let (status, _, response) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "path",
                "paths": {"all": ["/data/{{param.pg_dir}}"]},
                "parameters": parameter,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    assert_eq!(response["field"], "paths.all[0]");
    assert_eq!(response["reason"], "parameter_missing");

    let (status, _, response) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "tag",
                "paths": {"all": ["/data"]},
                "tags": ["dir-{{param.pg_dir}}"],
                "parameters": parameter,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    assert_eq!(response["field"], "tags[0]");

    // A script variable takes the empty string, so the profile saves.
    let (status, _, response) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "script",
                "paths": {"all": ["/data"]},
                "parameters": parameter,
                "scripts": [{
                    "name": "dump",
                    "interpreter": "bash",
                    "phase": "before",
                    "body": "pg_dump",
                    "failure_policy": "abort_backup",
                    "env": [{"name": "PGDIR", "kind": "parameter", "value": "pg_dir"}],
                }],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{response}");

    // A parameter that the profile does not declare is refused on the variable.
    let (status, _, response) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "unknown",
                "paths": {"all": ["/data"]},
                "scripts": [{
                    "name": "dump",
                    "interpreter": "bash",
                    "phase": "before",
                    "body": "pg_dump",
                    "failure_policy": "abort_backup",
                    "env": [{"name": "PGDIR", "kind": "parameter", "value": "pg_dir"}],
                }],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    assert_eq!(response["field"], "scripts[0].env[0].value");
    assert_eq!(response["reason"], "unknown");

    db.cleanup().await;
}

/// FR-PF-10: a secret value of a host blocks the delete of the secret.
#[tokio::test]
async fn ac_pf_10_a_secret_value_of_a_host_blocks_the_delete_of_the_secret() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let canary = "CANARY-hook-secret-2";
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
        json!([{"name": "pg_password", "kind": "secret", "secret_id": secret_id}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The table names the secret and never its value.
    let table = host_values(&app, &cookie, agent).await;
    assert_eq!(
        row(&table, "pg_password")["value"]["secret_name"],
        "pg-password"
    );
    assert!(!table.to_string().contains(canary));

    let (status, _, problem) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/secrets/{secret_id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    assert_eq!(problem["code"], "secret_in_use");
    assert_eq!(problem["blockers"]["items"][0]["kind"], "host");

    db.cleanup().await;
}
