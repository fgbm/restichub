//! The shape of the parameter declarations and their values (FR-PF-09 to FR-PF-11, ADR-0047).

#![allow(clippy::expect_used)]

mod common;
mod parameters_support;

use axum::http::StatusCode;
use serde_json::json;

use common::{app_with_kek, create_tenant_with_key, request, seed_agent, seed_session, send};
use parameters_support::{
    create_plan, create_profile, host_values, plan_view, profile_view, put_host_values,
    repository_fixture, row, update_profile,
};

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

/// FR-PF-09: a name that repeats in one list has its own reason.
#[tokio::test]
async fn fr_pf_09_a_duplicate_name_has_its_own_reason() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;

    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "pg",
                "paths": {"all": ["/data"]},
                "parameters": [
                    {"name": "pg_dir", "kind": "text", "default_value": "/a"},
                    {"name": "pg_dir", "kind": "text", "default_value": "/b"},
                ],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "parameters[1].name");
    assert_eq!(body["reason"], "duplicate");

    let (status, body) = put_host_values(
        &app,
        &cookie,
        agent,
        json!([
            {"name": "pg_dir", "kind": "text", "value": "/a"},
            {"name": "pg_dir", "kind": "text", "value": "/b"},
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "parameters[1].name");
    assert_eq!(body["reason"], "duplicate");

    db.cleanup().await;
}

/// FR-PF-11: two profiles of one host cannot declare one name with two kinds, because the host
/// holds a single value for that name (ADR-0047).
#[tokio::test]
async fn fr_pf_11_two_profiles_of_a_host_agree_on_the_kind() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let path = json!({"all": ["/data"]});

    let first = create_profile(
        &app,
        &cookie,
        json!({
            "name": "first",
            "paths": path,
            "parameters": [{"name": "pg_dir", "kind": "text", "default_value": "/a"}],
        }),
    )
    .await;
    let (status, body) = create_plan(
        &app,
        &cookie,
        "first",
        &first,
        agent,
        &repository,
        json!([]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let second = create_profile(
        &app,
        &cookie,
        json!({
            "name": "second",
            "paths": path,
            "parameters": [{"name": "pg_dir", "kind": "text", "default_value": "/b"}],
        }),
    )
    .await;
    let (status, body) = create_plan(
        &app,
        &cookie,
        "second",
        &second,
        agent,
        &repository,
        json!([]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // A profile save that gives the name another kind is refused, and it names the parameter.
    let (status, body) = update_profile(
        &app,
        &cookie,
        &second,
        json!({
            "paths": path,
            "parameters": [{"name": "pg_dir", "kind": "secret"}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "parameters");
    assert_eq!(body["reason"], "parameter_kind_mismatch");
    assert_eq!(body["parameters"], json!(["pg_dir"]));
    assert_eq!(
        profile_view(&app, &cookie, &second).await["parameters"][0]["kind"],
        "text",
        "the refused save changed nothing"
    );

    // A plan save of a profile with another kind is refused in the same way.
    let third = create_profile(
        &app,
        &cookie,
        json!({
            "name": "third",
            "paths": path,
            "parameters": [{"name": "pg_dir", "kind": "secret"}],
        }),
    )
    .await;
    let (status, body) = create_plan(
        &app,
        &cookie,
        "third",
        &third,
        agent,
        &repository,
        json!([]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "parameter_values");
    assert_eq!(body["reason"], "parameter_kind_mismatch");
    assert_eq!(body["parameters"], json!(["pg_dir"]));

    db.cleanup().await;
}

/// ADR-0047: a kind change of a parameter drops the overrides of the old kind, as a removal does.
#[tokio::test]
async fn adr_0047_a_kind_change_drops_the_overrides() {
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
            "paths": {"all": ["/data"]},
            "parameters": [{"name": "pg_dir", "kind": "text", "default_value": "/srv/pg"}],
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
        json!([{"name": "pg_dir", "kind": "text", "value": "/plan"}]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let plan = body["plan"]["id"]
        .as_str()
        .expect("the identifier")
        .to_owned();
    assert_eq!(
        plan_view(&app, &cookie, &plan).await["parameter_values"][0]["value"],
        "/plan"
    );

    let (status, body) = update_profile(
        &app,
        &cookie,
        &profile,
        json!({
            "paths": {"all": ["/data"]},
            "parameters": [{"name": "pg_dir", "kind": "secret"}],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        plan_view(&app, &cookie, &plan).await["parameter_values"]
            .as_array()
            .expect("a list")
            .len(),
        0,
        "the override of the old kind went with the declaration"
    );

    db.cleanup().await;
}

/// FR-PF-10: a value that breaks the compile of a plan of the host is refused on the value.
#[tokio::test]
async fn fr_pf_10_a_value_that_breaks_a_plan_is_refused() {
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
            "paths": {"all": ["/data"]},
            "tags": ["env-{{param.env}}"],
            "parameters": [{"name": "env", "kind": "text", "default_value": "dev"}],
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

    // Restic reads a comma in a tag as two tags, so the tag of the plan does not compile.
    let (status, body) = put_host_values(
        &app,
        &cookie,
        agent,
        json!([{"name": "env", "kind": "text", "value": "prod, eu"}]),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "parameters[0].value");
    assert_eq!(body["reason"], "invalid_format");
    let table = host_values(&app, &cookie, agent).await;
    assert!(
        table["parameters"]
            .as_array()
            .expect("a list")
            .iter()
            .all(|row| row["value"].is_null()),
        "the refused change kept no value: {table}"
    );

    let (status, body) = put_host_values(
        &app,
        &cookie,
        agent,
        json!([{"name": "env", "kind": "text", "value": "prod"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        row(&host_values(&app, &cookie, agent).await, "env")["value"]["value"],
        "prod"
    );

    db.cleanup().await;
}
