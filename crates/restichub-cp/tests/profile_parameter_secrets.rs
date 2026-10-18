//! The secrets and the reader of the parameter values (FR-PF-09, FR-PF-10, FR-PF-12, AC-PF-09,
//! AC-PF-10, ADR-0047).

#![allow(clippy::expect_used)]

mod common;
mod parameters_support;

use axum::http::StatusCode;
use serde_json::json;

use common::{app_with_kek, create_tenant_with_key, request, seed_agent, seed_session, send};
use parameters_support::{
    create_plan, create_profile, create_secret, host_values, list_plans, plan_view,
    put_host_values, repository_fixture, row, update_plan,
};

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

    // The retention expands no parameter, because one maintenance step serves every host
    // (ADR-0034, ADR-0047).
    let (status, _, response) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "retention",
                "paths": {"all": ["/data"]},
                "parameters": [{"name": "pg_dir", "kind": "text", "default_value": "/srv/pg"}],
                "retention": {"keep_last": 3, "keep_within": "{{param.pg_dir}}"},
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
    assert_eq!(response["field"], "retention.keep_within");
    assert_eq!(response["reason"], "parameter_not_allowed");

    db.cleanup().await;
}

/// AC-PF-10: a Viewer reads no text value, and the audit holds none.

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

/// FR-PF-10: a secret value of a host blocks the delete of the secret.
#[tokio::test]
async fn fr_pf_10_a_secret_value_of_a_host_blocks_the_delete_of_the_secret() {
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

/// ADR-0047: only an Admin sets, changes or removes a secret value; an Operator keeps the text
/// values (owner decision, threat T35).
#[tokio::test]
async fn adr_0047_an_operator_changes_no_secret_value() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let operator = seed_session(db.database(), tenant, "operator", "operator").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let first = create_secret(&app, &admin, "pg-password", "CANARY-first-1").await;
    let second = create_secret(&app, &admin, "pg-password-2", "CANARY-second-2").await;
    let profile = create_profile(
        &app,
        &admin,
        json!({
            "name": "pg",
            "paths": {"all": ["/data"]},
            "parameters": [
                {"name": "pg_user", "kind": "text", "required": true},
                {"name": "pg_password", "kind": "secret"},
            ],
        }),
    )
    .await;
    let text = json!({"name": "pg_user", "kind": "text", "value": "postgres"});
    let with_first = json!({"name": "pg_password", "kind": "secret", "secret_id": first});
    let with_second = json!({"name": "pg_password", "kind": "secret", "secret_id": second});

    // The Operator sets the text values of the host.
    let (status, body) = put_host_values(&app, &operator, agent, json!([text])).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // A secret value is the choice of an Admin, so the Operator is refused and nothing changes.
    let (status, body) = put_host_values(&app, &operator, agent, json!([text, with_first])).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "forbidden");
    let table = host_values(&app, &operator, agent).await;
    assert!(
        table["parameters"]
            .as_array()
            .expect("a list")
            .iter()
            .all(|row| row["value"]["secret_id"].is_null()),
        "{table}"
    );

    // The Admin sets the secret, and the Operator may keep it while it changes the text value.
    let (status, body) = put_host_values(&app, &admin, agent, json!([text, with_first])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let changed = json!({"name": "pg_user", "kind": "text", "value": "/srv/pg"});
    let (status, body) =
        put_host_values(&app, &operator, agent, json!([changed, with_first])).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Another secret and no secret at all are changes again.
    for values in [json!([changed, with_second]), json!([changed])] {
        let (status, body) = put_host_values(&app, &operator, agent, values).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    }

    // A plan of the Operator holds the text values only.
    let (status, body) = create_plan(
        &app,
        &operator,
        "nightly",
        &profile,
        agent,
        &repository,
        json!([text]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let plan = body["plan"]["id"]
        .as_str()
        .expect("the identifier")
        .to_owned();

    // A secret value in a plan save is the choice of an Admin too.
    let (status, body) = create_plan(
        &app,
        &operator,
        "daily",
        &profile,
        agent,
        &repository,
        json!([text, with_first]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = update_plan(
        &app,
        &admin,
        &plan,
        &profile,
        agent,
        &repository,
        json!([text, with_first]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // An unchanged secret entry of the plan is allowed, and a removed one is not.
    let (status, body) = update_plan(
        &app,
        &operator,
        &plan,
        &profile,
        agent,
        &repository,
        json!([text, with_first]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = update_plan(
        &app,
        &operator,
        &plan,
        &profile,
        agent,
        &repository,
        json!([text]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    db.cleanup().await;
}

/// FR-PF-11: a secret of another organization is not a value of this organization.
#[tokio::test]
async fn fr_pf_11_a_secret_of_another_tenant_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let other = create_tenant_with_key(db.database(), "other").await;
    let app = app_with_kek(db.database());
    let admin = seed_session(db.database(), tenant, "admin-a", "admin").await;
    let other_admin = seed_session(db.database(), other, "admin-b", "admin").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let foreign = create_secret(&app, &other_admin, "foreign", "CANARY-foreign-1").await;
    let profile = create_profile(
        &app,
        &admin,
        json!({
            "name": "pg",
            "paths": {"all": ["/data"]},
            "parameters": [{"name": "pg_password", "kind": "secret"}],
        }),
    )
    .await;
    let value = json!([{"name": "pg_password", "kind": "secret", "secret_id": foreign}]);

    let (status, body) = put_host_values(&app, &admin, agent, value.clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "parameters");
    assert_eq!(body["reason"], "unknown");

    let (status, body) =
        create_plan(&app, &admin, "nightly", &profile, agent, &repository, value).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "parameter_values");
    assert_eq!(body["reason"], "unknown");

    db.cleanup().await;
}

/// AC-PF-10: the plan list hides the text values from a Viewer.
#[tokio::test]
async fn ac_pf_10_a_viewer_reads_no_text_value_in_the_plan_list() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "params").await;
    let app = app_with_kek(db.database());
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let agent = seed_agent(db.database(), tenant, "web-1").await;
    let repository = repository_fixture(db.database(), tenant).await;
    let canary = "CANARY-plan-list-1";
    let profile = create_profile(
        &app,
        &admin,
        json!({
            "name": "pg",
            "paths": {"all": ["/data"]},
            "parameters": [{"name": "pg_dir", "kind": "text", "default_value": "/srv/pg"}],
        }),
    )
    .await;
    let (status, body) = create_plan(
        &app,
        &admin,
        "nightly",
        &profile,
        agent,
        &repository,
        json!([{"name": "pg_dir", "kind": "text", "value": canary}]),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let of_admin = list_plans(&app, &admin).await;
    assert_eq!(
        of_admin["items"][0]["parameter_values"][0]["value"], canary,
        "{of_admin}"
    );
    let of_viewer = list_plans(&app, &viewer).await;
    assert_eq!(
        of_viewer["items"][0]["parameter_values"][0]["name"],
        "pg_dir"
    );
    assert_eq!(of_viewer["items"][0]["parameter_values"][0]["value"], "");
    assert_eq!(of_viewer["items"][0]["parameter_values"][0]["hidden"], true);
    assert!(!of_viewer.to_string().contains(canary));

    db.cleanup().await;
}
