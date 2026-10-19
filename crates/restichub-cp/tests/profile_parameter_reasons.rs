//! The save-time reasons of a parameter reference (FR-PF-05, FR-PF-10, ADR-0047).
//!
//! An undeclared reference and an optional parameter without a default have their own reasons, and
//! the refusal names the parameter, so a person corrects the declaration instead of a host value.

#![allow(clippy::expect_used)]

mod common;
mod parameters_support;

use axum::http::StatusCode;
use serde_json::{Value, json};

use common::{app_with_kek, create_tenant_with_key, request, seed_session, send};

/// The profile bodies that hold an undeclared `{{param.pg_dir}}` in one template field, with the
/// field that the refusal names.
fn undeclared_fields() -> Vec<(&'static str, Value)> {
    vec![
        (
            "paths.all[0]",
            json!({"paths": {"all": ["/data/{{param.pg_dir}}"]}}),
        ),
        (
            "excludes[0]",
            json!({"paths": {"all": ["/data"]}, "excludes": ["{{param.pg_dir}}/*.tmp"]}),
        ),
        (
            "exclude_files[0]",
            json!({"paths": {"all": ["/data"]}, "exclude_files": ["/etc/{{param.pg_dir}}.list"]}),
        ),
        (
            "tags[0]",
            json!({"paths": {"all": ["/data"]}, "tags": ["dir-{{param.pg_dir}}"]}),
        ),
    ]
}

/// FR-PF-05, FR-PF-10: an undeclared reference has its own reason in each template field.
#[tokio::test]
async fn fr_pf_05_an_undeclared_reference_has_its_own_reason_in_every_template_field() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "reasons").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    for (index, (field, body)) in undeclared_fields().into_iter().enumerate() {
        let mut body = body;
        body["name"] = json!(format!("undeclared-{index}"));
        let (status, _, problem) = send(
            &app,
            request("POST", "/api/v1/profiles", Some(&cookie), Some(body)),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {problem}");
        assert_eq!(problem["field"], field);
        assert_eq!(problem["reason"], "parameter_not_declared");
        assert_eq!(problem["parameters"], json!(["pg_dir"]));
    }

    db.cleanup().await;
}

/// FR-PF-05, FR-PF-10: an optional parameter without a default has its own reason, and the same
/// field saves with a default or with the required flag.
#[tokio::test]
async fn fr_pf_05_an_optional_parameter_without_a_default_has_its_own_reason() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "reasons").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    for (index, (field, body)) in undeclared_fields().into_iter().enumerate() {
        let mut body = body;
        body["name"] = json!(format!("optional-{index}"));
        body["parameters"] = json!([{"name": "pg_dir", "kind": "text"}]);
        let (status, _, problem) = send(
            &app,
            request(
                "POST",
                "/api/v1/profiles",
                Some(&cookie),
                Some(body.clone()),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {problem}");
        assert_eq!(problem["field"], field);
        assert_eq!(problem["reason"], "parameter_needs_default");
        assert_eq!(problem["parameters"], json!(["pg_dir"]));

        body["parameters"] = json!([{"name": "pg_dir", "kind": "text", "required": true}]);
        let (status, _, response) = send(
            &app,
            request("POST", "/api/v1/profiles", Some(&cookie), Some(body)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{field}: {response}");
    }

    db.cleanup().await;
}

/// FR-PF-05: a script variable of the kind parameter that names an undeclared parameter has the
/// same reason as a template field.
#[tokio::test]
async fn fr_pf_05_a_script_variable_names_no_undeclared_parameter() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "reasons").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "script",
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
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "scripts[0].env[0].value");
    assert_eq!(problem["reason"], "parameter_not_declared");
    assert_eq!(problem["parameters"], json!(["pg_dir"]));

    db.cleanup().await;
}
