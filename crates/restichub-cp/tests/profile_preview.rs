//! The draft preview of a profile (FR-PF-04, AC-PF-01, AC-PF-02, FR-HK-09, FR-HK-12).

#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use restichub_db::test_util::TestDatabase;
use restichub_db::{AgentId, TenantId};
use serde_json::{Value, json};

use common::{
    app_with_kek, count, create_tenant_with_key, exec, request, seed_agent_full, seed_repository,
    seed_secret, seed_session, send,
};

/// A local backend row for a test, and its identifier.
async fn repository_fixture(db: &restichub_db::Database, tenant: TenantId) -> String {
    let secret = seed_secret(db, tenant, "preview.password").await;
    seed_repository(db, tenant, "preview-repo", secret)
        .await
        .to_string()
}

async fn create_profile(app: &axum::Router, cookie: &str, body: Value) -> Value {
    let (status, _, response) = send(
        app,
        request("POST", "/api/v1/profiles", Some(cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    response
        .get("profile")
        .cloned()
        .unwrap_or_else(|| panic!("the response holds no profile: {response}"))
}

/// Turns the trust mode of a host on, so a preview of a script compiles (FR-HK-08).
async fn trust_scripts(db: &TestDatabase, tenant: TenantId, agent: AgentId) {
    exec(
        db,
        tenant,
        "UPDATE agents SET server_scripts = 'enabled', script_interpreters = ARRAY['bash'] \
         WHERE id = $1",
        agent.as_uuid(),
    )
    .await;
}

/// A before entry of a profile: the interpreter, the body and the environment.
fn script(body: &str, env: Value) -> Value {
    json!({
        "name": "dump",
        "interpreter": "bash",
        "phase": "before",
        "body": body,
        "timeout_seconds": 60,
        "failure_policy": "abort_backup",
        "env": env,
    })
}

/// AC-PF-01: the preview shows the steps and the command line with masked secrets.
#[tokio::test]
async fn ac_pf_01_the_preview_masks_the_secrets() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    // An S3 repository created through the API, so the backend credentials are set.
    let canary = "RESTICHUB-CANARY-profile-1a2b";
    let (status, _, repository) = send(
        &app,
        request(
            "POST",
            "/api/v1/repositories",
            Some(&cookie),
            Some(json!({
                "name": "s3-prod",
                "backend": {
                    "kind": "s3",
                    "endpoint": "https://s3.example.net",
                    "bucket": "backups-prod",
                    "region": "eu-central-1"
                },
                "password": canary,
                "s3_access_key": canary,
                "s3_secret_key": canary,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{repository}");
    let repository_id = repository["id"]
        .as_str()
        .expect("the identifier")
        .to_owned();

    let agent = seed_agent_full(db.database(), tenant, "web-1", "linux", "0.19.0", &[]).await;

    let profile = create_profile(
        &app,
        &cookie,
        json!({
            "name": "web-daily",
            "paths": {"all": ["/srv"], "linux": ["/etc"]},
            "excludes": ["*.tmp"],
            "tags": ["daily"],
            "options": {"one_file_system": true},
        }),
    )
    .await;
    let profile_id = profile["id"].as_str().expect("the identifier").to_owned();

    // The preview takes the repository of the plan of the profile on this agent (ADR-0047).
    let (status, _, plan) = send(
        &app,
        request(
            "POST",
            "/api/v1/plans",
            Some(&cookie),
            Some(json!({
                "name": "web-daily",
                "profile_id": profile_id,
                "agent_id": agent.to_string(),
                "repository_id": repository_id,
                "cron": "0 2 * * *",
                "time_zone": "UTC",
                "missed_run_policy": "run_all",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{plan}");

    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(json!({
                "agent_id": agent.to_string(),
                "profile_id": profile_id,
                "name": "web-daily",
                "paths": {"all": ["/srv"], "linux": ["/etc"]},
                "excludes": ["*.tmp"],
                "tags": ["daily"],
                "options": {"one_file_system": true},
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["repository_id"], repository_id.as_str());
    // The commands of a stored profile filter the retention on its own tag.
    assert!(
        preview
            .to_string()
            .contains(&format!("restichub-profile-{profile_id}")),
        "{preview}"
    );

    let steps = preview["steps"].as_array().expect("the steps");
    assert!(!steps.is_empty());
    let backup = steps
        .iter()
        .find(|step| step["kind"] == "backup")
        .expect("the backup step");
    let command = backup["command"].as_str().expect("the command");
    assert!(command.contains("backup"), "{command}");
    assert!(command.contains("--one-file-system"), "{command}");
    assert!(command.contains("--exclude=*.tmp"), "{command}");
    assert!(command.contains("--tag=daily"), "{command}");
    // Positional paths follow the `--` marker, so a path never becomes an option.
    assert!(command.contains("-- /srv /etc"), "{command}");

    // The secrets are masked: the response holds no canary and the environment values are `***`.
    assert!(
        !preview.to_string().contains(canary),
        "the preview must not hold the secret"
    );
    let environment = backup["environment"].as_array().expect("the environment");
    let env_value = |name: &str| {
        environment
            .iter()
            .find(|entry| entry["name"] == name)
            .and_then(|entry| entry["value"].as_str())
            .map(str::to_owned)
    };
    assert_eq!(env_value("RESTIC_PASSWORD").as_deref(), Some("***"));
    assert_eq!(env_value("AWS_SECRET_ACCESS_KEY").as_deref(), Some("***"));
    // The region is not a secret, so the preview shows it in clear.
    assert_eq!(
        env_value("AWS_DEFAULT_REGION").as_deref(),
        Some("eu-central-1")
    );

    db.cleanup().await;
}

/// FR-HK-12: a caller without the profile write right gets the commands of the stored profile, and
/// the draft fields of the request change nothing.
#[tokio::test]
async fn fr_hk_12_a_reader_previews_the_stored_profile_only() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let agent = seed_agent_full(db.database(), tenant, "reader-host", "linux", "0.19.0", &[]).await;
    trust_scripts(&db, tenant, agent).await;

    let body_canary = "body-canary-of-the-stored-script";
    let value_canary = "value-canary-of-the-stored-script";
    let profile = create_profile(
        &app,
        &admin,
        json!({
            "name": "with-script",
            "paths": {"all": ["/srv/stored"]},
            "scripts": [script(body_canary, json!([
                {"name": "PGDATABASE", "kind": "string", "value": value_canary}
            ]))],
        }),
    )
    .await;
    let profile_id = profile["id"].as_str().expect("the identifier").to_owned();

    // The Viewer sends the identifiers only. The script step shows its name and its variables, not
    // the body and not a value.
    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&viewer),
            Some(json!({
                "agent_id": agent.to_string(),
                "profile_id": profile_id,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let steps = preview["steps"].as_array().expect("the steps");
    assert!(
        steps.iter().any(|step| step["kind"]
            .as_str()
            .is_some_and(|kind| kind.starts_with("script"))),
        "{preview}"
    );
    let text = preview.to_string();
    assert!(!text.contains(body_canary), "{preview}");
    assert!(!text.contains(value_canary), "{preview}");

    // A draft of a Viewer is ignored: the crafted path in the body appears in no command.
    let crafted = "CRAFTED-PATH-CANARY";
    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&viewer),
            Some(json!({
                "agent_id": agent.to_string(),
                "profile_id": profile_id,
                "name": "crafted",
                "paths": {"all": [format!("/srv/{crafted}/{{{{hostname}}}}")]},
                "tags": ["crafted"],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let text = preview.to_string();
    assert!(!text.contains(crafted), "{preview}");
    assert!(text.contains("/srv/stored"), "{preview}");
    assert!(!text.contains("--tag=crafted"), "{preview}");

    // The Viewer names no stored profile, so the server cannot compile anything for it.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&viewer),
            Some(json!({
                "agent_id": agent.to_string(),
                "name": "new",
                "paths": {"all": ["/srv"]},
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "profile_id");
    assert_eq!(problem["reason"], "empty");

    // A stored profile of the tenant that does not exist is not found.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&viewer),
            Some(json!({
                "agent_id": agent.to_string(),
                "profile_id": restichub_db::ProfileId::new().to_string(),
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{problem}");

    db.cleanup().await;
}

/// FR-HK-12: an Operator previews the draft of a stored profile, and the stored scripts stay as
/// they are stored.
#[tokio::test]
async fn fr_hk_12_an_operator_previews_the_stored_scripts() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let operator = seed_session(db.database(), tenant, "operator", "operator").await;
    let agent = seed_agent_full(
        db.database(),
        tenant,
        "operator-host",
        "linux",
        "0.19.0",
        &[],
    )
    .await;
    trust_scripts(&db, tenant, agent).await;

    let body_canary = "body-canary-of-the-stored-script";
    let value_canary = "value-canary-of-the-stored-script";
    let profile = create_profile(
        &app,
        &admin,
        json!({
            "name": "with-script",
            "paths": {"all": ["/srv/stored"]},
            "scripts": [script(body_canary, json!([
                {"name": "PGDATABASE", "kind": "string", "value": value_canary}
            ]))],
        }),
    )
    .await;
    let profile_id = profile["id"].as_str().expect("the identifier").to_owned();

    // The draft of an Operator carries the stored scripts with empty bodies: only an Admin changes
    // the code that runs on a host (FR-HK-12).
    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&operator),
            Some(json!({
                "agent_id": agent.to_string(),
                "profile_id": profile_id,
                "paths": {"all": ["/srv/edited"]},
                "scripts": [script("", json!([
                    {"name": "PGDATABASE", "kind": "string", "value": ""}
                ]))],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");

    let steps = preview["steps"].as_array().expect("the steps");
    let backup = steps
        .iter()
        .find(|step| step["kind"] == "backup")
        .expect("the backup step");
    let command = backup["command"].as_str().expect("the command");
    // The rest of the draft compiles as the person edited it.
    assert!(command.contains("-- /srv/edited"), "{command}");
    assert!(!command.contains("/srv/stored"), "{command}");
    // The stored script keeps its place, and the preview shows no body and no value.
    let script_step = steps
        .iter()
        .find(|step| {
            step["kind"]
                .as_str()
                .is_some_and(|kind| kind.starts_with("script"))
        })
        .expect("the script step");
    let script_command = script_step["command"].as_str().expect("the command");
    assert!(script_command.contains("script=dump"), "{script_command}");
    assert!(script_command.contains("phase=before"), "{script_command}");
    let text = preview.to_string();
    assert!(!text.contains(body_canary), "{preview}");
    assert!(!text.contains(value_canary), "{preview}");

    db.cleanup().await;
}

/// FR-HK-12: a script that an Operator puts into the draft changes nothing.
#[tokio::test]
async fn fr_hk_12_a_script_of_an_operator_draft_is_ignored() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let operator = seed_session(db.database(), tenant, "operator", "operator").await;
    let agent = seed_agent_full(
        db.database(),
        tenant,
        "crafted-host",
        "linux",
        "0.19.0",
        &[],
    )
    .await;
    trust_scripts(&db, tenant, agent).await;

    let profile = create_profile(
        &app,
        &admin,
        json!({
            "name": "stored-script",
            "paths": {"all": ["/srv"]},
            "scripts": [script("pg_dump --all", json!([]))],
        }),
    )
    .await;
    let profile_id = profile["id"].as_str().expect("the identifier").to_owned();

    let crafted = "crafted-script-of-the-draft";
    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&operator),
            Some(json!({
                "agent_id": agent.to_string(),
                "profile_id": profile_id,
                "paths": {"all": ["/srv"]},
                "scripts": [{
                    "name": crafted,
                    "interpreter": "bash",
                    "phase": "after",
                    "body": "echo crafted",
                    "timeout_seconds": 60,
                    "failure_policy": "abort_backup",
                    "env": [],
                }],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let text = preview.to_string();
    assert!(!text.contains(crafted), "{preview}");
    assert!(text.contains("script=dump"), "{preview}");
    assert!(text.contains("phase=before"), "{preview}");

    db.cleanup().await;
}

/// AC-PF-10: a Viewer draft fills no template of the stored profile, so the host value of a parameter
/// that only the draft reads appears in no command; a value that the stored profile puts into a path
/// is part of the command, and the reader sees it (FR-PF-12, ADR-0047).
#[tokio::test]
async fn ac_pf_10_a_reader_draft_shows_no_parameter_value() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let agent = seed_agent_full(db.database(), tenant, "params-host", "linux", "0.19.0", &[]).await;
    let canary = "CANARY-host-value-1";
    let stored = create_profile(
        &app,
        &admin,
        json!({
            "name": "plain",
            "paths": {"all": ["/srv/stored"]},
            "parameters": [{"name": "pg_dir", "kind": "text"}],
        }),
    )
    .await;
    let stored_id = stored["id"].as_str().expect("the identifier").to_owned();
    let (status, _, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/v1/agents/{agent}/parameters"),
            Some(&admin),
            Some(json!({"parameters": [
                {"name": "pg_dir", "kind": "text", "value": canary},
            ]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The draft of the Viewer puts the parameter into a path, and the stored profile does not.
    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&viewer),
            Some(json!({
                "agent_id": agent.to_string(),
                "profile_id": stored_id,
                "paths": {"all": ["/srv/{{param.pg_dir}}"]},
                "parameters": [{"name": "pg_dir", "kind": "text"}],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let text = preview.to_string();
    assert!(text.contains("/srv/stored"), "{preview}");
    assert!(
        !text.contains(canary),
        "the draft read the host value: {preview}"
    );
    assert!(!text.contains("{{param"), "{preview}");

    // The stored profile puts the value into the path of its own accord, so the command holds it.
    let planned = create_profile(
        &app,
        &admin,
        json!({
            "name": "planned",
            "paths": {"all": ["/srv/{{param.pg_dir}}"]},
            "parameters": [{"name": "pg_dir", "kind": "text", "default_value": "/default"}],
        }),
    )
    .await;
    let planned_id = planned["id"].as_str().expect("the identifier").to_owned();
    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&viewer),
            Some(json!({
                "agent_id": agent.to_string(),
                "profile_id": planned_id,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert!(
        preview.to_string().contains(canary),
        "the stored path is part of the command: {preview}"
    );

    db.cleanup().await;
}

/// AC-PF-01: the preview takes a body that is as large as a profile create, and refuses a body that
/// is larger (FR-PF-05, FR-HK-09).
#[tokio::test]
async fn ac_pf_01_the_preview_takes_the_body_limit_of_a_profile_create() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent_full(db.database(), tenant, "big-host", "linux", "0.19.0", &[]).await;

    // 200 excludes of 100 characters pass the 16 KiB limit of the other routes.
    let excludes: Vec<String> = (0..200).map(|index| format!("{index:0>100}")).collect();
    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(json!({
                "agent_id": agent.to_string(),
                "paths": {"all": ["/srv"]},
                "excludes": excludes,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");

    // A body above the profile create limit is refused before the validation.
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(json!({
                "agent_id": agent.to_string(),
                "paths": {"all": ["/srv"]},
                "scripts": [script(&"x".repeat(3 * 1024 * 1024), json!([]))],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    db.cleanup().await;
}

/// ADR-0047: the preview takes the repository of the body, and a draft without a stored profile
/// shows no repository (FR-PF-04, AC-PF-01).
#[tokio::test]
async fn ac_pf_01_the_preview_takes_the_repository_of_the_body_or_none() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent_full(
        db.database(),
        tenant,
        "preview-host",
        "linux",
        "0.19.0",
        &[],
    )
    .await;
    let repository = repository_fixture(db.database(), tenant).await;
    let preview_body = |repository: Option<String>| {
        let mut body = json!({
            "agent_id": agent.to_string(),
            "name": "preview-repo",
            "paths": {"all": ["/srv"]},
        });
        if let Some(repository) = repository {
            body["repository_id"] = json!(repository);
        }
        body
    };

    // A new draft has no plan, so its steps carry no repository environment.
    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(preview_body(None)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert!(preview["repository_id"].is_null(), "{preview}");
    assert!(
        !preview.to_string().contains("RESTIC_REPOSITORY"),
        "{preview}"
    );
    assert!(
        !preview.to_string().contains("RESTIC_PASSWORD"),
        "{preview}"
    );

    // The body names the repository, and the space around the identifier is ignored.
    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(preview_body(Some(format!("  {repository}  ")))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["repository_id"], repository.as_str());
    assert!(
        preview.to_string().contains("RESTIC_REPOSITORY"),
        "{preview}"
    );

    db.cleanup().await;
}

/// AC-PF-01: the preview accepts a draft that has no name yet, and keeps the other checks.
#[tokio::test]
async fn ac_pf_01_the_preview_accepts_a_draft_without_a_name() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent_full(db.database(), tenant, "draft-host", "linux", "0.19.0", &[]).await;

    let draft = |name: Value| {
        json!({
            "agent_id": agent.to_string(),
            "name": name,
            "paths": {"all": ["/srv"]},
        })
    };

    // A new profile shows its commands before the first save, when it has no name.
    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(draft(json!(""))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert!(
        preview["steps"]
            .as_array()
            .is_some_and(|steps| !steps.is_empty()),
        "{preview}"
    );

    // Every other check of the draft stays.
    let mut no_name = draft(json!(""));
    no_name["excludes"] = json!(["/var/lib/\u{0007}"]);
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(no_name),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "excludes");

    // A name that the person typed is still checked.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(draft(json!("x".repeat(101)))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "name");

    db.cleanup().await;
}

/// FR-PF-04: the preview of an unsaved profile shows the placeholder tag, and a stored profile
/// shows its own tag (FR-RP-08).
#[tokio::test]
async fn fr_pf_04_the_preview_of_a_new_profile_shows_the_placeholder_tag() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent_full(db.database(), tenant, "tag-host", "linux", "0.19.0", &[]).await;

    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(json!({
                "agent_id": agent.to_string(),
                "paths": {"all": ["/srv"]},
                "retention": {"keep_daily": 7},
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert!(
        preview.to_string().contains("restichub-profile-new"),
        "{preview}"
    );
    assert!(
        !preview.to_string().contains("profile-validation"),
        "{preview}"
    );

    db.cleanup().await;
}

/// AC-PF-02: the preview refuses VSS for a Linux agent and an old restic.
#[tokio::test]
async fn ac_pf_02_the_preview_points_to_the_field() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let linux_agent =
        seed_agent_full(db.database(), tenant, "linux-1", "linux", "0.19.0", &[]).await;
    let old_agent = seed_agent_full(db.database(), tenant, "old-1", "linux", "0.17.0", &[]).await;

    // VSS on a Linux agent.
    let vss_draft = |agent: &str| {
        json!({
            "agent_id": agent,
            "name": "vss-profile",
            "paths": {"all": ["/srv"]},
            "options": {"use_fs_snapshot": true},
        })
    };
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(vss_draft(&linux_agent.to_string())),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "options.use_fs_snapshot");
    assert_eq!(problem["reason"], "unsupported");

    // An agent with an old restic (FR-AG-11, NFR-CMP-03).
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(vss_draft(&old_agent.to_string())),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "agent_id");
    assert_eq!(problem["reason"], "incompatible");

    // An unknown template variable is refused at save time (FR-PF-03, FR-PF-05).
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "template-profile",
                "paths": {"all": ["/srv/{{nope}}"]},
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "paths.all[0]");
    assert_eq!(problem["reason"], "invalid_format");

    // An unclosed template is refused at save time.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "unclosed-template",
                "paths": {"all": ["/srv/{{hostname"]},
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "paths.all[0]");
    assert_eq!(problem["reason"], "invalid_format");

    // VSS for a Linux target is a warning at save time, not a refusal: the profile may target
    // several operating systems (FR-PF-05).
    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "vss-warning",
                "paths": {"all": ["/srv"]},
                "options": {"use_fs_snapshot": true},
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let warnings = created["warnings"].as_array().expect("the warnings");
    assert!(
        warnings
            .iter()
            .any(|warning| warning["field"] == "options.use_fs_snapshot"
                && warning["reason"] == "unsupported"),
        "{created}"
    );

    db.cleanup().await;
}

/// AC-PF-03: each agent gets its own variable values in the preview.
#[tokio::test]
async fn ac_pf_03_each_agent_gets_its_own_values() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let prod = seed_agent_full(
        db.database(),
        tenant,
        "web-prod",
        "linux",
        "0.19.0",
        &["env=prod".to_owned()],
    )
    .await;
    let dev = seed_agent_full(
        db.database(),
        tenant,
        "web-dev",
        "linux",
        "0.19.0",
        &["env=dev".to_owned()],
    )
    .await;

    let draft = json!({
        "name": "templated",
        "paths": {"all": ["/srv/{{hostname}}/{{agent.label.env}}"]},
        "tags": ["{{os}}"],
    });
    let preview_for = |agent: &str| {
        let mut body = draft.clone();
        body["agent_id"] = json!(agent);
        body
    };

    let backup_command = |preview: &Value| -> String {
        preview["steps"]
            .as_array()
            .expect("the steps")
            .iter()
            .find(|step| step["kind"] == "backup")
            .and_then(|step| step["command"].as_str())
            .map(str::to_owned)
            .expect("the backup command")
    };

    let (_, _, prod_preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(preview_for(&prod.to_string())),
        ),
    )
    .await;
    let (_, _, dev_preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(preview_for(&dev.to_string())),
        ),
    )
    .await;

    let prod_command = backup_command(&prod_preview);
    let dev_command = backup_command(&dev_preview);
    assert!(
        prod_command.contains("/srv/web-prod/prod"),
        "{prod_command}"
    );
    assert!(prod_command.contains("--tag=linux"), "{prod_command}");
    assert!(dev_command.contains("/srv/web-dev/dev"), "{dev_command}");
    assert_ne!(prod_command, dev_command);

    db.cleanup().await;
}

/// AC-PF-02: the draft preview validates the draft as a create request and points to the field; it
/// stores nothing and writes no audit record (FR-PF-05).
#[tokio::test]
async fn ac_pf_02_the_draft_preview_points_to_the_field() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent_full(db.database(), tenant, "draft-1", "linux", "0.19.0", &[]).await;
    let audit_before = count(db.database(), tenant, "audit_log").await;

    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(json!({
                "agent_id": agent.to_string(),
                "name": "relative",
                "paths": {"all": ["relative/path"]},
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "paths.all");
    assert_eq!(problem["reason"], "invalid_format");

    // The draft keeps the validation of a create request (FR-PF-05).
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(json!({
                "agent_id": agent.to_string(),
                "name": "",
                "paths": {"all": ["/srv"]},
                "tags": ["daily,weekly"],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "tags");

    // The preview stores nothing, so the list is still empty.
    let (status, _, body) = send(
        &app,
        request("GET", "/api/v1/profiles", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().map(Vec::len), Some(0), "{body}");
    assert_eq!(
        count(db.database(), tenant, "audit_log").await,
        audit_before,
        "the preview wrote an audit record"
    );

    db.cleanup().await;
}

/// AC-PF-02: a bad stored profile identifier names the field of the body (FR-PF-04).
#[tokio::test]
async fn ac_pf_02_a_bad_profile_id_of_the_preview_names_the_field() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent_full(db.database(), tenant, "bad-id", "linux", "0.19.0", &[]).await;

    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(json!({
                "agent_id": agent.to_string(),
                "profile_id": "not-a-uuid",
                "paths": {"all": ["/srv"]},
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "profile_id");
    assert_eq!(problem["reason"], "invalid_format");

    db.cleanup().await;
}

/// FR-HK-09: the preview refuses a secret that a script may not use, and the refusal is the same
/// for any such identifier (ADR-0017).
#[tokio::test]
async fn fr_hk_09_the_preview_refuses_every_secret_which_is_no_hook_secret() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let other = create_tenant_with_key(db.database(), "globex").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent_full(db.database(), tenant, "secret-host", "linux", "0.19.0", &[]).await;

    // A secret of the tenant with the wrong purpose, a secret of another tenant and an identifier
    // that exists nowhere.
    let wrong_purpose = seed_secret(db.database(), tenant, "repository-password").await;
    let foreign = seed_secret(db.database(), other, "foreign-hook-secret").await;
    let absent = restichub_db::SecretId::new();

    let mut bodies = Vec::new();
    for secret in [wrong_purpose, foreign, absent] {
        let (status, _, problem) = send(
            &app,
            request(
                "POST",
                "/api/v1/profiles/preview",
                Some(&cookie),
                Some(json!({
                    "agent_id": agent.to_string(),
                    "paths": {"all": ["/srv"]},
                    "scripts": [script("pg_dump", json!([
                        {"name": "PGPASSWORD", "kind": "secret_ref", "value": secret.to_string()}
                    ]))],
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
        assert_eq!(problem["field"], "scripts[0].env[0].value");
        assert_eq!(problem["reason"], "unknown");
        // The request identifier is the only part that differs between the three refusals.
        let mut problem = problem;
        problem
            .as_object_mut()
            .expect("a problem object")
            .remove("request_id");
        bodies.push(problem.to_string());
    }
    assert_eq!(bodies[0], bodies[1], "{bodies:?}");
    assert_eq!(bodies[1], bodies[2], "{bodies:?}");

    db.cleanup().await;
}

/// The profile of another tenant gives HTTP 404 (AC-TN-01).
#[tokio::test]
async fn ac_tn_01_a_foreign_tenant_agent_and_repository_are_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant_a = create_tenant_with_key(db.database(), "acme").await;
    let tenant_b = create_tenant_with_key(db.database(), "globex").await;
    let app = app_with_kek(db.database());
    let cookie_a = seed_session(db.database(), tenant_a, "admin-a", "admin").await;
    let repository_b = repository_fixture(db.database(), tenant_b).await;
    let agent_a = seed_agent_full(db.database(), tenant_a, "a-host", "linux", "0.19.0", &[]).await;

    // A plan that names the repository of tenant B is refused.
    let profile = create_profile(
        &app,
        &cookie_a,
        json!({"name": "foreign-repo", "paths": {"all": ["/srv"]}, "tags": ["daily"]}),
    )
    .await;
    let profile_id = profile["id"].as_str().expect("the identifier");
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/plans",
            Some(&cookie_a),
            Some(json!({
                "name": "foreign-repo",
                "profile_id": profile_id,
                "agent_id": agent_a.to_string(),
                "repository_id": repository_b,
                "cron": "0 2 * * *",
                "time_zone": "UTC",
                "missed_run_policy": "run_all",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "repository_id");

    // The draft preview with the agent of tenant B is refused.
    let agent_b = seed_agent_full(db.database(), tenant_b, "b-host", "linux", "0.19.0", &[]).await;
    let draft = |extra: Value| {
        let mut body = json!({
            "agent_id": agent_a.to_string(),
            "name": "foreign-repo",
            "paths": {"all": ["/srv"]},
        });
        for (key, value) in extra.as_object().expect("an object") {
            body[key] = value.clone();
        }
        body
    };
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie_a),
            Some(draft(json!({"agent_id": agent_b.to_string()}))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "agent_id");

    // A draft preview that names the repository of tenant B is refused.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie_a),
            Some(draft(json!({"repository_id": repository_b}))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "repository_id");
    assert_eq!(problem["reason"], "unknown");

    // A draft preview of the stored profile of tenant B is not found.
    let cookie_b = seed_session(db.database(), tenant_b, "admin-b", "admin").await;
    let profile_b = create_profile(
        &app,
        &cookie_b,
        json!({"name": "b-profile", "paths": {"all": ["/srv"]}, "tags": ["daily"]}),
    )
    .await;
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie_a),
            Some(draft(json!({
                "profile_id": profile_b["id"].as_str().expect("the identifier"),
            }))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{problem}");
    assert_eq!(problem["code"], "not_found");

    db.cleanup().await;
}

/// An agent that reports no parsable restic version cannot be checked (AC-AG-09).
#[tokio::test]
async fn an_unparsable_restic_version_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let agent = seed_agent_full(db.database(), tenant, "no-version", "linux", "", &[]).await;

    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&cookie),
            Some(json!({
                "agent_id": agent.to_string(),
                "name": "no-version",
                "paths": {"all": ["/srv"]},
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "agent_id");
    assert_eq!(problem["reason"], "incompatible");

    db.cleanup().await;
}

/// AC-PF-10: the preview of a stored profile shows the name of a script variable that names a
/// parameter and masks the value of that parameter (FR-PF-12, ADR-0047).
#[tokio::test]
async fn ac_pf_10_the_preview_masks_the_value_of_a_parameter_variable() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let agent = seed_agent_full(db.database(), tenant, "script-host", "linux", "0.19.0", &[]).await;
    trust_scripts(&db, tenant, agent).await;
    let canary = "CANARY-script-value-1";
    let profile = create_profile(
        &app,
        &admin,
        json!({
            "name": "with-parameter",
            "paths": {"all": ["/srv"]},
            "parameters": [{"name": "pg_dir", "kind": "text"}],
            "scripts": [script("echo dump", json!([
                {"name": "PGDIR", "kind": "parameter", "value": "pg_dir"}
            ]))],
        }),
    )
    .await;
    let profile_id = profile["id"].as_str().expect("the identifier").to_owned();
    let (status, _, body) = send(
        &app,
        request(
            "PUT",
            &format!("/api/v1/agents/{agent}/parameters"),
            Some(&admin),
            Some(json!({"parameters": [
                {"name": "pg_dir", "kind": "text", "value": canary},
            ]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles/preview",
            Some(&viewer),
            Some(json!({
                "agent_id": agent.to_string(),
                "profile_id": profile_id,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    let steps = preview["steps"].as_array().expect("the steps");
    assert!(
        steps.iter().any(|step| step["kind"]
            .as_str()
            .is_some_and(|kind| kind.starts_with("script"))),
        "{preview}"
    );
    let text = preview.to_string();
    assert!(text.contains("PGDIR"), "{preview}");
    assert!(
        !text.contains(canary),
        "the preview shows the value: {preview}"
    );

    db.cleanup().await;
}
