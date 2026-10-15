//! The profile API (FR-PF-01, FR-PF-03, FR-PF-04, FR-PF-05).
//!
//! The tests cover the create, the read, the update, the delete, the preview with masked secrets,
//! the compiler validation with a field path, the tenant isolation and the role matrix.

#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use restichub_cp::profiles::{MAX_EXCLUDES, MAX_HOOKS, MAX_PATHS, MAX_TAGS};
use restichub_db::agents::{self as agents_repo, NewAgent};
use restichub_db::{AgentId, PLATFORM_TENANT_ID, TenantId};
use serde_json::{Value, json};
use time::OffsetDateTime;

use common::{
    app_with_kek, create_tenant_with_key, request, seed_agent, seed_plan, seed_repository,
    seed_secret, seed_session, send,
};

/// Inserts an active agent row with explicit facts.
async fn seed_agent_full(
    db: &restichub_db::Database,
    tenant: TenantId,
    hostname: &str,
    os: &str,
    restic_version: &str,
    labels: &[String],
) -> AgentId {
    let mut serial = vec![0_u8; 16];
    getrandom::fill(&mut serial).expect("cannot read random bytes");
    let mut tx = db.begin_tenant(tenant).await.expect("cannot begin");
    let id = agents_repo::create(
        &mut tx,
        &NewAgent {
            id: AgentId::new(),
            hostname,
            os,
            arch: "amd64",
            agent_version: "0.1.0",
            restic_version,
            labels,
            certificate_serial: &serial,
            certificate_expires_at: OffsetDateTime::now_utc() + time::Duration::days(30),
            last_seen_at: None,
        },
    )
    .await
    .expect("cannot create the agent");
    tx.commit().await.expect("cannot commit");
    id
}

/// A local backend row for a test, and its identifier.
async fn repository_fixture(db: &restichub_db::Database, tenant: TenantId) -> String {
    let secret = seed_secret(db, tenant, "profile.password").await;
    seed_repository(db, tenant, "profile-repo", secret)
        .await
        .to_string()
}

/// Builds a create request body with only the required fields.
fn profile_body(name: &str, repository_id: &str, paths_all: Value) -> Value {
    json!({
        "name": name,
        "repository_id": repository_id,
        "paths": {"all": paths_all},
        "tags": ["daily"],
    })
}

async fn create_profile(app: &axum::Router, cookie: &str, body: Value) -> Value {
    let (status, _, response) = send(
        app,
        request("POST", "/api/v1/profiles", Some(cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    // The response holds the profile and the save-time warnings.
    response
        .get("profile")
        .cloned()
        .unwrap_or_else(|| panic!("the response holds no profile: {response}"))
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
            "repository_id": repository_id,
            "paths": {"all": ["/srv"], "linux": ["/etc"]},
            "excludes": ["*.tmp"],
            "tags": ["daily"],
            "options": {"one_file_system": true},
        }),
    )
    .await;
    let profile_id = profile["id"].as_str().expect("the identifier").to_owned();

    let (status, _, preview) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/profiles/{profile_id}/preview"),
            Some(&cookie),
            Some(json!({"agent_id": agent.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preview}");

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

/// AC-PF-02: the preview refuses VSS for a Linux agent, a missing hook and an old restic.
#[tokio::test]
async fn ac_pf_02_the_preview_points_to_the_field() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let repository_id = repository_fixture(db.database(), tenant).await;

    let linux_agent =
        seed_agent_full(db.database(), tenant, "linux-1", "linux", "0.19.0", &[]).await;
    let old_agent = seed_agent_full(db.database(), tenant, "old-1", "linux", "0.17.0", &[]).await;

    // VSS on a Linux agent.
    let vss = create_profile(
        &app,
        &cookie,
        json!({
            "name": "vss-profile",
            "repository_id": repository_id,
            "paths": {"all": ["/srv"]},
            "options": {"use_fs_snapshot": true},
        }),
    )
    .await;
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            &format!(
                "/api/v1/profiles/{}/preview",
                vss["id"].as_str().expect("the identifier")
            ),
            Some(&cookie),
            Some(json!({"agent_id": linux_agent.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "options.use_fs_snapshot");
    assert_eq!(problem["reason"], "unsupported");

    // A hook that the agent allow-list does not hold (FR-HK-04). Another agent reports it, so the
    // profile is saved and only the preview for this agent refuses it.
    let holder = seed_agent_full(db.database(), tenant, "holder-1", "linux", "0.19.0", &[]).await;
    common::seed_agent_hooks(
        db.database(),
        tenant,
        holder,
        &std::collections::BTreeMap::from([(
            "pg_dump".to_owned(),
            restichub_compiler::AllowListedHook::default(),
        )]),
        restichub_db::HookAllowListState::Loaded,
    )
    .await;
    let hook = create_profile(
        &app,
        &cookie,
        json!({
            "name": "hook-profile",
            "repository_id": repository_id,
            "paths": {"all": ["/srv"]},
            "hooks": [{"hook_id": "pg_dump", "phase": "before", "parameters": []}],
        }),
    )
    .await;
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            &format!(
                "/api/v1/profiles/{}/preview",
                hook["id"].as_str().expect("the identifier")
            ),
            Some(&cookie),
            Some(json!({"agent_id": linux_agent.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "hooks[0].hook_id");
    assert_eq!(problem["reason"], "unknown");

    // An agent with an old restic (FR-AG-11, NFR-CMP-03).
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            &format!(
                "/api/v1/profiles/{}/preview",
                vss["id"].as_str().expect("the identifier")
            ),
            Some(&cookie),
            Some(json!({"agent_id": old_agent.to_string()})),
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
                "repository_id": repository_id,
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
                "repository_id": repository_id,
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
                "repository_id": repository_id,
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

/// AC-PF-02: the save-time validation refuses a field and points to it.
#[tokio::test]
async fn ac_pf_02_the_save_validation_points_to_the_field() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let repository_id = repository_fixture(db.database(), tenant).await;

    // Upper-case letters are refused in a name.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(profile_body("Upper", &repository_id, json!(["/srv"]))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "name");

    // Relative paths are refused.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(profile_body(
                "relative",
                &repository_id,
                json!(["relative/path"]),
            )),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "paths.all");

    // Unknown repository identifiers are refused.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(profile_body(
                "no-repo",
                &uuid::Uuid::now_v7().to_string(),
                json!(["/srv"]),
            )),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "repository_id");

    // A duplicate tag is not refused, but an empty tag is.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "bad-tag",
                "repository_id": repository_id,
                "paths": {"all": ["/srv"]},
                "tags": [""],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "tags");

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
    let repository_id = repository_fixture(db.database(), tenant).await;

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

    let profile = create_profile(
        &app,
        &cookie,
        json!({
            "name": "templated",
            "repository_id": repository_id,
            "paths": {"all": ["/srv/{{hostname}}/{{agent.label.env}}"]},
            "tags": ["{{os}}"],
        }),
    )
    .await;
    let profile_id = profile["id"].as_str().expect("the identifier").to_owned();

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
            &format!("/api/v1/profiles/{profile_id}/preview"),
            Some(&cookie),
            Some(json!({"agent_id": prod.to_string()})),
        ),
    )
    .await;
    let (_, _, dev_preview) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/profiles/{profile_id}/preview"),
            Some(&cookie),
            Some(json!({"agent_id": dev.to_string()})),
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

/// The profile of another tenant gives HTTP 404 (AC-TN-01).
#[tokio::test]
async fn ac_tn_01_a_profile_of_another_tenant_gives_404() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant_a = create_tenant_with_key(db.database(), "acme").await;
    let tenant_b = create_tenant_with_key(db.database(), "globex").await;
    let app = app_with_kek(db.database());
    let cookie_a = seed_session(db.database(), tenant_a, "admin-a", "admin").await;
    let cookie_b = seed_session(db.database(), tenant_b, "admin-b", "admin").await;

    let repository_b = repository_fixture(db.database(), tenant_b).await;
    let profile_b = create_profile(
        &app,
        &cookie_b,
        profile_body("b-profile", &repository_b, json!(["/srv"])),
    )
    .await;
    let id_b = profile_b["id"].as_str().expect("the identifier");

    let (status, _, _) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/profiles/{id_b}"),
            Some(&cookie_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Tenant A holds no profile, so its list is empty.
    let (status, _, body) = send(
        &app,
        request("GET", "/api/v1/profiles", Some(&cookie_a), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().map(Vec::len), Some(0));

    db.cleanup().await;
}

/// The role matrix: a Viewer reads, an Operator writes, the platform tenant is refused.
#[tokio::test]
async fn the_role_matrix_and_the_platform_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let repository_id = repository_fixture(db.database(), tenant).await;

    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let operator = seed_session(db.database(), tenant, "operator", "operator").await;

    // Viewers cannot write a profile.
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&viewer),
            Some(profile_body(
                "viewer-profile",
                &repository_id,
                json!(["/srv"]),
            )),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Operators can write and read a profile.
    let created = create_profile(
        &app,
        &operator,
        profile_body("operator-profile", &repository_id, json!(["/srv"])),
    )
    .await;
    assert_eq!(created["name"], "operator-profile");
    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/profiles", Some(&viewer), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The platform tenant holds no profile: the operation is tenant-only.
    let platform = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/profiles", Some(&platform), None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
}

/// The update replaces the settings and the delete removes the row.
#[tokio::test]
async fn an_update_replaces_the_settings_and_a_delete_removes_the_row() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let repository_id = repository_fixture(db.database(), tenant).await;

    let created = create_profile(
        &app,
        &cookie,
        profile_body("editable", &repository_id, json!(["/srv"])),
    )
    .await;
    let id = created["id"].as_str().expect("the identifier").to_owned();

    let (status, _, updated) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/profiles/{id}"),
            Some(&cookie),
            Some(json!({
                "name": "edited",
                "repository_id": repository_id,
                "paths": {"all": ["/data"], "linux": ["/etc"]},
                "tags": ["weekly"],
                "retention": {"keep_daily": 7},
                "check": {"read_data_subset": "10%"},
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    let updated = &updated["profile"];
    assert_eq!(updated["name"], "edited");
    assert_eq!(updated["paths"]["linux"][0], "/etc");
    assert_eq!(updated["retention"]["keep_daily"], 7);
    assert_eq!(updated["check"]["read_data_subset"], "10%");

    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/profiles/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    db.cleanup().await;
}

/// A profile that a plan uses cannot be removed until the plan is gone (FR-SC-11).
#[tokio::test]
async fn a_profile_in_use_by_a_plan_cannot_be_deleted() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let repository_id = repository_fixture(db.database(), tenant).await;

    let profile = create_profile(
        &app,
        &cookie,
        profile_body("plan-bound", &repository_id, json!(["/srv"])),
    )
    .await;
    let profile_id = profile["id"].as_str().expect("the identifier").to_owned();
    let agent = seed_agent(db.database(), tenant, "plan-host").await;
    let plan = seed_plan(
        db.database(),
        tenant,
        "plan",
        restichub_db::ProfileId::from_uuid(
            uuid::Uuid::parse_str(&profile_id).expect("a profile id"),
        ),
        agent,
    )
    .await;

    // The plan binds the profile, so the delete is refused with a conflict.
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/profiles/{profile_id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // After the plan is gone the profile can be removed.
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/plans/{plan}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/profiles/{profile_id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    db.cleanup().await;
}

/// A profile update revalidates the plans that use it against the facts of their agent and reports
/// the failing ones as warnings; it does not refuse the change (FR-SC-11).
#[tokio::test]
async fn a_profile_update_reports_the_plans_that_no_longer_compile() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let repository_id = repository_fixture(db.database(), tenant).await;
    let agent = seed_agent_full(db.database(), tenant, "linux-host", "linux", "0.19.0", &[]).await;

    let profile = create_profile(
        &app,
        &cookie,
        profile_body("linux-bound", &repository_id, json!(["/srv"])),
    )
    .await;
    let profile_id = profile["id"].as_str().expect("the identifier").to_owned();
    seed_plan(
        db.database(),
        tenant,
        "bound",
        restichub_db::ProfileId::from_uuid(
            uuid::Uuid::parse_str(&profile_id).expect("a profile id"),
        ),
        agent,
    )
    .await;

    // The update makes the profile Windows-only, so it no longer compiles for the Linux agent.
    // Updating the profile keeps the plan and reports it.
    let (status, _, updated) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/profiles/{profile_id}"),
            Some(&cookie),
            Some(json!({
                "name": "linux-bound",
                "repository_id": repository_id,
                "paths": {"windows": ["C:\\Data"]},
                "tags": ["daily"],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    let plan_warnings = updated["plan_warnings"]
        .as_array()
        .expect("the plan warnings");
    assert_eq!(plan_warnings.len(), 1);
    assert_eq!(plan_warnings[0]["name"], "bound");
    assert_eq!(plan_warnings[0]["field"], "paths");

    db.cleanup().await;
}

/// A repository that a profile uses cannot be removed until the profile is gone.
#[tokio::test]
async fn a_repository_in_use_by_a_profile_cannot_be_deleted() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let repository_id = repository_fixture(db.database(), tenant).await;

    let profile = create_profile(
        &app,
        &cookie,
        profile_body("in-use", &repository_id, json!(["/srv"])),
    )
    .await;
    let profile_id = profile["id"].as_str().expect("the identifier");

    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/repositories/{repository_id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // After the profile is gone the repository can be removed.
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/profiles/{profile_id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/repositories/{repository_id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    db.cleanup().await;
}

/// The item limits, the root paths and the trailing separator (FR-PF-05).
#[tokio::test]
async fn the_item_limits_and_the_root_paths_are_enforced() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let repository_id = repository_fixture(db.database(), tenant).await;

    // Root paths are allowed.
    let root = create_profile(
        &app,
        &cookie,
        json!({
            "name": "root-path",
            "repository_id": repository_id,
            "paths": {"all": ["/"], "windows": ["C:\\"]},
        }),
    )
    .await;
    assert_eq!(root["paths"]["all"][0], "/");
    assert_eq!(root["paths"]["windows"][0], "C:\\");

    // One trailing separator is dropped.
    let trailing = create_profile(
        &app,
        &cookie,
        json!({
            "name": "trailing",
            "repository_id": repository_id,
            "paths": {"all": ["/srv/"], "windows": ["C:\\Data\\"]},
        }),
    )
    .await;
    assert_eq!(trailing["paths"]["all"][0], "/srv");
    assert_eq!(trailing["paths"]["windows"][0], "C:\\Data");

    // Too many paths, across every operating system.
    let paths: Vec<String> = (0..=MAX_PATHS).map(|index| format!("/p{index}")).collect();
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "many-paths",
                "repository_id": repository_id,
                "paths": {"all": paths},
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "paths");
    assert_eq!(problem["reason"], "too_long");

    // Too many excludes.
    let excludes: Vec<String> = (0..=MAX_EXCLUDES)
        .map(|index| format!("e{index}"))
        .collect();
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "many-excludes",
                "repository_id": repository_id,
                "paths": {"all": ["/srv"]},
                "excludes": excludes,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "excludes");
    assert_eq!(problem["reason"], "too_long");

    // Too many tags.
    let tags: Vec<String> = (0..=MAX_TAGS).map(|index| format!("t{index}")).collect();
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "many-tags",
                "repository_id": repository_id,
                "paths": {"all": ["/srv"]},
                "tags": tags,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "tags");
    assert_eq!(problem["reason"], "too_long");

    // Too many hooks.
    let hooks: Vec<Value> = (0..=MAX_HOOKS)
        .map(|index| json!({"hook_id": format!("h{index}"), "phase": "before"}))
        .collect();
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie),
            Some(json!({
                "name": "many-hooks",
                "repository_id": repository_id,
                "paths": {"all": ["/srv"]},
                "hooks": hooks,
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "hooks");
    assert_eq!(problem["reason"], "too_long");

    db.cleanup().await;
}

/// A foreign tenant identifier is refused for the agent and for the repository (AC-TN-01).
#[tokio::test]
async fn a_foreign_tenant_agent_and_repository_are_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant_a = create_tenant_with_key(db.database(), "acme").await;
    let tenant_b = create_tenant_with_key(db.database(), "globex").await;
    let app = app_with_kek(db.database());
    let cookie_a = seed_session(db.database(), tenant_a, "admin-a", "admin").await;
    let repository_a = repository_fixture(db.database(), tenant_a).await;
    let repository_b = repository_fixture(db.database(), tenant_b).await;

    // A profile that names the repository of tenant B is refused.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/profiles",
            Some(&cookie_a),
            Some(profile_body("foreign-repo", &repository_b, json!(["/srv"]))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "repository_id");

    // A preview with the agent of tenant B is refused.
    let agent_b = seed_agent_full(db.database(), tenant_b, "b-host", "linux", "0.19.0", &[]).await;
    let profile = create_profile(
        &app,
        &cookie_a,
        profile_body("local", &repository_a, json!(["/srv"])),
    )
    .await;
    let profile_id = profile["id"].as_str().expect("the identifier");
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/profiles/{profile_id}/preview"),
            Some(&cookie_a),
            Some(json!({"agent_id": agent_b.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "agent_id");

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
    let repository_id = repository_fixture(db.database(), tenant).await;
    let agent = seed_agent_full(db.database(), tenant, "no-version", "linux", "", &[]).await;

    let profile = create_profile(
        &app,
        &cookie,
        profile_body("no-version", &repository_id, json!(["/srv"])),
    )
    .await;
    let profile_id = profile["id"].as_str().expect("the identifier");
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/profiles/{profile_id}/preview"),
            Some(&cookie),
            Some(json!({"agent_id": agent.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "agent_id");
    assert_eq!(problem["reason"], "incompatible");

    db.cleanup().await;
}
