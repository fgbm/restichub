//! AC-TN-01: a cross-tenant object identifier returns HTTP 404 and changes nothing (FR-TN-01).
//!
//! The test is table-driven over the served OpenAPI document and the route table of
//! [`ROUTES`]. Every operation with a path parameter that names an object needs one fixture row;
//! an operation without a row stops the test with its method and path. The fixtures live in the
//! registry [`FIXTURES`], keyed by the route path pattern, so a later epic (agents, jobs,
//! repositories) adds one row for a new route.
//!
//! The cross-tenant case needs two tenants, so the harness runs in multi-tenant mode. Single mode
//! serves one tenant and hides the platform routes (AC-TN-06, ADR-0024); the last test records that.

#![allow(clippy::expect_used)]

mod common;

use std::collections::BTreeMap;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use restichub_cp::context::{Access, Operation, minimum_role};
use restichub_cp::routes::ROUTES;
use restichub_db::test_util::TestDatabase;
use restichub_db::{Role, TenantId, audit};
use serde_json::Value;

use common::{
    app_single, app_with_kek, body_json, count, create_tenant_with_key, create_user, list_tokens,
    request, seed_api_token, seed_secret, seed_session, send_response,
};

// The roles of one tenant (requirements section 2). A Platform Admin is absent: it acts in the
// platform tenant only.
const ROLES: [Role; 3] = [Role::Viewer, Role::Operator, Role::Admin];

/// The canary names of the tenant B objects. The response of a tenant A call must hold none.
const CANARY_USER: &str = "canary-bob";
const CANARY_SECRET: &str = "canary-secret";
const CANARY_REPOSITORY: &str = "canary-repo";
const CANARY_PROFILE: &str = "canary-profile";
const CANARY_PLAN: &str = "canary-plan";
const CANARY_CHANNEL: &str = "canary-channel";
const CANARY_SCRIPT_TEMPLATE: &str = "canary-script-template";
const CANARY_JOB_TEMPLATE: &str = "canary-job-template";

/// The kind of tenant B object whose identifier the isolation check puts into the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Object {
    User,
    Secret,
    ApiToken,
    Agent,
    EnrollmentToken,
    Repository,
    Job,
    Profile,
    Plan,
    Channel,
    ScriptTemplate,
    JobTemplate,
}

/// The isolation fixture of one route path pattern from the route table (AC-TN-01).
///
/// The registry is keyed by the path pattern of the route table. The methods of one path share a
/// row, and a new route adds one row only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fixture {
    /// The path holds the identifier of a tenant B object (the `{id}` family).
    Object(Object),
    /// The endpoint returns a collection. The response must hold no tenant B data.
    Collection,
    /// A platform operation (ADR-0023). It is platform-scoped by design, so it is no cross-tenant
    /// call. The test checks that a tenant Admin of tenant A gets HTTP 403 or 404 on it.
    Platform,
    /// The route holds no tenant object: a public probe, authentication or the instance metadata.
    Exempt,
}

/// The isolation fixture of each route path pattern.
const FIXTURES: &[(&str, Fixture)] = &[
    ("/healthz", Fixture::Exempt),
    ("/readyz", Fixture::Exempt),
    // The scrape answers with text and reads the tenant of the credential only. The metrics tests
    // prove that another tenant sees none of its samples.
    ("/metrics", Fixture::Exempt),
    ("/api/v1/openapi.json", Fixture::Exempt),
    ("/api/v1/auth/sign-in", Fixture::Exempt),
    ("/api/v1/auth/sign-out", Fixture::Exempt),
    ("/api/v1/auth/me", Fixture::Exempt),
    ("/api/v1/auth/password", Fixture::Exempt),
    ("/api/v1/auth/tenants", Fixture::Exempt),
    ("/api/v1/auth/switch", Fixture::Exempt),
    ("/api/v1/setup", Fixture::Exempt),
    ("/api/v1/setup/status", Fixture::Exempt),
    ("/api/v1/instance", Fixture::Exempt),
    ("/agent/v1/enroll", Fixture::Exempt),
    ("/api/v1/audit", Fixture::Collection),
    ("/api/v1/platform/audit", Fixture::Platform),
    (
        "/api/v1/platform/signing-keys/compromise",
        Fixture::Platform,
    ),
    ("/api/v1/platform/tenants", Fixture::Platform),
    ("/api/v1/platform/tenants/{id}", Fixture::Platform),
    ("/api/v1/platform/tenants/{id}/suspend", Fixture::Platform),
    ("/api/v1/platform/tenants/{id}/resume", Fixture::Platform),
    ("/api/v1/platform/users", Fixture::Platform),
    ("/api/v1/platform/users/{id}", Fixture::Platform),
    ("/api/v1/platform/users/{id}/memberships", Fixture::Platform),
    (
        "/api/v1/platform/users/{id}/memberships/{tenant_id}",
        Fixture::Platform,
    ),
    ("/api/v1/users", Fixture::Collection),
    ("/api/v1/users/{id}", Fixture::Object(Object::User)),
    ("/api/v1/users/{id}/password", Fixture::Object(Object::User)),
    ("/api/v1/api-tokens", Fixture::Collection),
    ("/api/v1/api-tokens/{id}", Fixture::Object(Object::ApiToken)),
    ("/api/v1/agents", Fixture::Collection),
    ("/api/v1/agents/{id}", Fixture::Object(Object::Agent)),
    ("/api/v1/agents/{id}/revoke", Fixture::Object(Object::Agent)),
    (
        "/api/v1/agents/{id}/reenroll",
        Fixture::Object(Object::Agent),
    ),
    (
        "/api/v1/agents/{id}/replace",
        Fixture::Object(Object::Agent),
    ),
    ("/api/v1/enrollment-tokens", Fixture::Collection),
    (
        "/api/v1/enrollment-tokens/{id}",
        Fixture::Object(Object::EnrollmentToken),
    ),
    ("/api/v1/secrets", Fixture::Collection),
    ("/api/v1/secrets/{id}", Fixture::Object(Object::Secret)),
    (
        "/api/v1/secrets/{id}/value",
        Fixture::Object(Object::Secret),
    ),
    ("/api/v1/repositories", Fixture::Collection),
    (
        "/api/v1/repositories/{id}",
        Fixture::Object(Object::Repository),
    ),
    (
        "/api/v1/repositories/{id}/init",
        Fixture::Object(Object::Repository),
    ),
    (
        "/api/v1/repositories/{id}/test-access",
        Fixture::Object(Object::Repository),
    ),
    (
        "/api/v1/repositories/{id}/unlock-all",
        Fixture::Object(Object::Repository),
    ),
    (
        "/api/v1/repositories/{id}/maintenance",
        Fixture::Object(Object::Repository),
    ),
    (
        "/api/v1/repositories/{id}/maintenance/run",
        Fixture::Object(Object::Repository),
    ),
    (
        "/api/v1/repositories/{id}/snapshots",
        Fixture::Object(Object::Repository),
    ),
    (
        "/api/v1/repositories/{id}/ls",
        Fixture::Object(Object::Repository),
    ),
    (
        "/api/v1/repositories/{id}/restore",
        Fixture::Object(Object::Repository),
    ),
    ("/api/v1/jobs", Fixture::Collection),
    ("/api/v1/jobs/{id}", Fixture::Object(Object::Job)),
    ("/api/v1/jobs/{id}/cancel", Fixture::Object(Object::Job)),
    ("/api/v1/jobs/{id}/logs", Fixture::Object(Object::Job)),
    ("/api/v1/profiles", Fixture::Collection),
    ("/api/v1/profiles/defaults", Fixture::Collection),
    ("/api/v1/profiles/{id}", Fixture::Object(Object::Profile)),
    (
        "/api/v1/profiles/{id}/preview",
        Fixture::Object(Object::Profile),
    ),
    ("/api/v1/plans", Fixture::Collection),
    ("/api/v1/plans/{id}", Fixture::Object(Object::Plan)),
    ("/api/v1/plans/{id}/run", Fixture::Object(Object::Plan)),
    (
        "/api/v1/plans/{id}/next-runs",
        Fixture::Object(Object::Plan),
    ),
    // The preview computes the fire times of a schedule in the request body. It names no tenant
    // object.
    ("/api/v1/plans/preview", Fixture::Exempt),
    ("/api/v1/dashboard", Fixture::Collection),
    ("/api/v1/alerts", Fixture::Collection),
    ("/api/v1/alert-rules", Fixture::Collection),
    ("/api/v1/notification-channels", Fixture::Collection),
    (
        "/api/v1/notification-channels/{id}",
        Fixture::Object(Object::Channel),
    ),
    (
        "/api/v1/notification-channels/{id}/test",
        Fixture::Object(Object::Channel),
    ),
    ("/api/v1/script-templates", Fixture::Collection),
    (
        "/api/v1/script-templates/{id}",
        Fixture::Object(Object::ScriptTemplate),
    ),
    ("/api/v1/job-templates", Fixture::Collection),
    (
        "/api/v1/job-templates/{id}",
        Fixture::Object(Object::JobTemplate),
    ),
];

fn fixture_for(method: &str, path: &str) -> Fixture {
    FIXTURES
        .iter()
        .find(|(pattern, _)| *pattern == path)
        .map(|(_, fixture)| *fixture)
        .unwrap_or_else(|| panic!("the operation {method} {path} has no isolation fixture"))
}

/// The `(method, path)` pairs of the served OpenAPI document.
fn document_operations(document: &Value) -> Vec<(String, String)> {
    let paths = document
        .get("paths")
        .and_then(Value::as_object)
        .expect("the OpenAPI document holds no paths");
    let mut operations = Vec::new();
    for (path, item) in paths {
        let methods = item
            .as_object()
            .unwrap_or_else(|| panic!("the path {path} is not an object"));
        for method in methods.keys() {
            operations.push((method.clone(), path.clone()));
        }
    }
    operations.sort();
    operations
}

/// Sends a request and returns the status, the content type and the body.
async fn call(app: &Router, request: Request<Body>) -> (StatusCode, Option<String>, Value) {
    let response = send_response(app, request).await;
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = body_json(response).await;
    (status, content_type, body)
}

/// Asserts the result of a tenant A call on an object identifier of tenant B.
///
/// The authorization layer refuses a role below the minimum role of the operation before the
/// object lookup, so the status is HTTP 403 for that role and HTTP 404 for an allowed role.
fn assert_cross_tenant(
    role: Role,
    operation: Operation,
    method: &str,
    path: &str,
    status: StatusCode,
    content_type: Option<String>,
    problem: &Value,
) {
    if role.allows(minimum_role(operation)) {
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "{method} {path} as {} with an identifier of tenant B",
            role.as_str()
        );
        assert_eq!(
            content_type.as_deref(),
            Some("application/problem+json"),
            "{method} {path} must answer with a problem document"
        );
        assert_eq!(
            problem.get("code").and_then(Value::as_str),
            Some("not_found"),
            "{method} {path} must answer with the code not_found"
        );
    } else {
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {path} as {} must return HTTP 403",
            role.as_str()
        );
    }
}

/// Asserts that a tenant A Admin cannot reach a platform operation.
fn assert_platform_refused(method: &str, uri: &str, status: StatusCode) {
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "the platform operation {method} {uri} as a tenant Admin gave {status}"
    );
}

#[tokio::test]
async fn ac_tn_01_a_cross_tenant_identifier_returns_404_and_changes_nothing() {
    let Some(db) = common::test_db().await else {
        return;
    };

    // Tenant A and tenant B are real tenants. The secret fixtures need a data encryption key.
    let tenant_a = create_tenant_with_key(db.database(), "cross-a").await;
    let tenant_b = create_tenant_with_key(db.database(), "cross-b").await;

    // The tenant B objects. Their identifiers go into the path of a tenant A call.
    let user_b = create_user(db.database(), tenant_b, CANARY_USER, "phc", "viewer").await;
    let secret_b = seed_secret(db.database(), tenant_b, CANARY_SECRET).await;
    let repository_b =
        common::seed_repository(db.database(), tenant_b, CANARY_REPOSITORY, secret_b).await;
    let profile_b =
        common::seed_profile(db.database(), tenant_b, CANARY_PROFILE, repository_b).await;
    let agent_b = common::seed_agent(db.database(), tenant_b, "canary-host").await;
    let enrollment_token_b = common::seed_enrollment_token(db.database(), tenant_b, user_b).await;
    let job_b = common::seed_job(db.database(), tenant_b, agent_b, repository_b).await;
    let plan_b = common::seed_plan(db.database(), tenant_b, CANARY_PLAN, profile_b, agent_b).await;
    let channel_b = seed_channel(&db, tenant_b, CANARY_CHANNEL).await;
    let (script_template_b, job_template_b) = seed_templates(&db, tenant_b).await;
    seed_api_token(db.database(), tenant_b, "viewer").await;
    let token_b = list_tokens(db.database(), tenant_b)
        .await
        .first()
        .map(|token| token.id)
        .expect("tenant B holds an API token");
    // One audit record of tenant B, so a spurious write from tenant A shows up below.
    let mut tx = db
        .database()
        .begin_tenant(tenant_b)
        .await
        .expect("cannot begin the transaction");
    let mut event = audit::AuditEvent::new(audit::AuditAction::ApiTokenCreate, "api_token");
    event.object_id = Some(token_b.as_uuid());
    audit::insert(&mut tx, &event)
        .await
        .expect("cannot insert the audit record");
    tx.commit().await.expect("cannot commit the audit record");

    // Tenant A holds one session and one API token for each role.
    let mut sessions: Vec<(Role, String)> = Vec::new();
    let mut bearer: Vec<(Role, String)> = Vec::new();
    for role in ROLES {
        let username = format!("session-{}", role.as_str());
        let cookie = seed_session(db.database(), tenant_a, &username, role.as_str()).await;
        sessions.push((role, cookie));
        let token = seed_api_token(db.database(), tenant_a, role.as_str()).await;
        bearer.push((role, token));
    }
    let admin_cookie = sessions
        .iter()
        .find(|(role, _)| *role == Role::Admin)
        .map(|(_, cookie)| cookie.clone())
        .expect("tenant A holds an Admin session");
    let admin_token = bearer
        .iter()
        .find(|(role, _)| *role == Role::Admin)
        .map(|(_, token)| token.clone())
        .expect("tenant A holds an Admin token");

    let app = app_with_kek(db.database());

    // A repository of tenant A, so the profile collection create succeeds. The profile references
    // it and the composite foreign key is satisfied. The plan collection create needs a profile and
    // an agent of tenant A too.
    let secret_a = seed_secret(db.database(), tenant_a, "cross-a-secret").await;
    let repository_a =
        common::seed_repository(db.database(), tenant_a, "cross-a-repo", secret_a).await;
    let profile_a =
        common::seed_profile(db.database(), tenant_a, "cross-a-profile", repository_a).await;
    let agent_a = common::seed_agent(db.database(), tenant_a, "cross-a-host").await;

    let (status, _, document) =
        call(&app, request("GET", "/api/v1/openapi.json", None, None)).await;
    assert_eq!(status, StatusCode::OK);

    let b_ids = [
        tenant_b.to_string(),
        user_b.to_string(),
        secret_b.to_string(),
        token_b.to_string(),
        agent_b.to_string(),
        enrollment_token_b.to_string(),
        repository_b.to_string(),
        job_b.to_string(),
        profile_b.to_string(),
        plan_b.to_string(),
        channel_b.to_string(),
        script_template_b.to_string(),
        job_template_b.to_string(),
    ];
    let before = snapshot_tenant(&db, tenant_b).await;
    let audit_before = count(db.database(), tenant_b, "audit_log").await;

    let mut index = 0usize;
    for (method, path) in document_operations(&document) {
        index += 1;
        let route = ROUTES
            .iter()
            .find(|route| route.method == method && route.path == path)
            .unwrap_or_else(|| {
                panic!("the OpenAPI operation {method} {path} is not in the route table")
            });
        let http_method = method.to_ascii_uppercase();
        let body = common::write_body(
            &http_method,
            &path,
            index,
            false,
            &tenant_b.to_string(),
            &repository_a.to_string(),
            &profile_a.to_string(),
            &agent_a.to_string(),
        );
        match fixture_for(&method, &path) {
            Fixture::Exempt => {}
            Fixture::Platform => {
                // A platform operation is platform-scoped by design. A tenant Admin of tenant A
                // must not reach it.
                let uri = path.replace("{id}", &tenant_b.to_string());
                let response = call(
                    &app,
                    request(
                        &http_method,
                        &uri,
                        Some(admin_cookie.as_str()),
                        body.clone(),
                    ),
                )
                .await;
                assert_platform_refused(&http_method, &uri, response.0);
                let response = call(
                    &app,
                    common::bearer_request(&http_method, &uri, admin_token.as_str(), body.clone()),
                )
                .await;
                assert_platform_refused(&http_method, &uri, response.0);
            }
            Fixture::Object(object) => {
                let id = match object {
                    Object::User => user_b.to_string(),
                    Object::Secret => secret_b.to_string(),
                    Object::ApiToken => token_b.to_string(),
                    Object::Agent => agent_b.to_string(),
                    Object::EnrollmentToken => enrollment_token_b.to_string(),
                    Object::Repository => repository_b.to_string(),
                    Object::Job => job_b.to_string(),
                    Object::Profile => profile_b.to_string(),
                    Object::Plan => plan_b.to_string(),
                    Object::Channel => channel_b.to_string(),
                    Object::ScriptTemplate => script_template_b.to_string(),
                    Object::JobTemplate => job_template_b.to_string(),
                };
                let uri = path.replace("{id}", &id);
                let Access::Protected(operation) = route.access else {
                    panic!("the object route {http_method} {path} is public");
                };
                for (role, cookie) in &sessions {
                    let (status, content_type, problem) = call(
                        &app,
                        request(&http_method, &uri, Some(cookie.as_str()), body.clone()),
                    )
                    .await;
                    assert_cross_tenant(
                        *role,
                        operation,
                        &http_method,
                        &path,
                        status,
                        content_type,
                        &problem,
                    );
                }
                for (role, token) in &bearer {
                    let (status, content_type, problem) = call(
                        &app,
                        common::bearer_request(&http_method, &uri, token.as_str(), body.clone()),
                    )
                    .await;
                    assert_cross_tenant(
                        *role,
                        operation,
                        &http_method,
                        &path,
                        status,
                        content_type,
                        &problem,
                    );
                }
            }
            Fixture::Collection => {
                // A collection endpoint holds no object identifier. The response of tenant A must
                // hold no tenant B data.
                let (status, _, response) = call(
                    &app,
                    request(&http_method, &path, Some(&admin_cookie), body),
                )
                .await;
                let expected = if http_method == "POST" {
                    StatusCode::CREATED
                } else {
                    StatusCode::OK
                };
                assert_eq!(
                    status, expected,
                    "{http_method} {path} as tenant A Admin must return {expected}"
                );
                let text = response.to_string();
                for needle in [
                    tenant_b.to_string(),
                    user_b.to_string(),
                    secret_b.to_string(),
                    token_b.to_string(),
                    agent_b.to_string(),
                    enrollment_token_b.to_string(),
                    CANARY_USER.to_owned(),
                    CANARY_SECRET.to_owned(),
                    CANARY_REPOSITORY.to_owned(),
                    CANARY_PROFILE.to_owned(),
                    CANARY_PLAN.to_owned(),
                    CANARY_CHANNEL.to_owned(),
                    CANARY_SCRIPT_TEMPLATE.to_owned(),
                    CANARY_JOB_TEMPLATE.to_owned(),
                    channel_b.to_string(),
                    script_template_b.to_string(),
                    job_template_b.to_string(),
                ] {
                    assert!(
                        !text.contains(&needle),
                        "{http_method} {path} of tenant A holds tenant B data: {needle}"
                    );
                }
            }
        }
    }

    // No cross-tenant call changes a row of tenant B.
    let after = snapshot_tenant(&db, tenant_b).await;
    assert_eq!(
        before, after,
        "a cross-tenant call changed the rows of tenant B"
    );
    assert_eq!(
        count(db.database(), tenant_b, "audit_log").await,
        audit_before,
        "a cross-tenant call wrote an audit record for tenant B"
    );

    // No audit record of tenant A names a tenant B object.
    for (object_id, diff) in audit_rows(&db, tenant_a).await {
        for id in &b_ids {
            assert_ne!(
                &object_id, id,
                "an audit record of tenant A names the tenant B object {id}"
            );
            assert!(
                !diff.contains(id),
                "the diff of an audit record of tenant A names the tenant B object {id}"
            );
        }
        assert!(
            !diff.contains(CANARY_USER)
                && !diff.contains(CANARY_SECRET)
                && !diff.contains(CANARY_REPOSITORY),
            "the diff of an audit record of tenant A names a tenant B object"
        );
    }

    db.cleanup().await;
}

/// The cross-tenant case needs two tenants, so the harness runs in multi mode. Single mode serves
/// one tenant and hides the platform routes (AC-TN-06). The test records the mode boundary.
#[tokio::test]
async fn ac_tn_01_single_mode_has_one_tenant_and_hides_the_platform_routes() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app_single(db.database());
    for route in ROUTES {
        if !route.access.is_platform() {
            continue;
        }
        let uri = route
            .path
            .replace("{id}", &restichub_db::TenantId::new().to_string());
        let http_method = route.method.to_ascii_uppercase();
        let (status, _, problem) = call(&app, request(&http_method, &uri, None, None)).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "single mode must hide the platform route {http_method} {uri}"
        );
        assert_eq!(
            problem.get("code").and_then(Value::as_str),
            Some("not_found")
        );
    }
    db.cleanup().await;
}

/// The rows of a tenant in every table that holds `tenant_id`, read through the owner connection.
///
/// The owner login is subject to `FORCE ROW LEVEL SECURITY` (ADR-0023). The function sets the
/// tenant setting, so Row-Level Security shows the rows of the tenant only.
async fn seed_channel(db: &TestDatabase, tenant: TenantId, name: &str) -> restichub_db::ChannelId {
    let id = restichub_db::ChannelId::new();
    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    restichub_db::channels::create(
        &mut tx,
        &restichub_db::channels::NewChannel {
            id,
            name,
            kind: restichub_db::channels::ChannelKind::Email,
            enabled: true,
            recipients: &["canary@example.com".to_owned()],
            url_origin: None,
            secret_id: None,
            url_secret_id: None,
            created_by: None,
        },
    )
    .await
    .expect("cannot create the channel");
    tx.commit().await.expect("cannot commit the channel");
    id
}

async fn seed_templates(
    db: &TestDatabase,
    tenant: TenantId,
) -> (restichub_db::ScriptTemplateId, restichub_db::JobTemplateId) {
    use restichub_db::script_templates::{NewJobTemplate, NewScriptTemplate};
    let script_id = restichub_db::ScriptTemplateId::new();
    let job_id = restichub_db::JobTemplateId::new();
    let parameters = serde_json::json!([]);
    let scripts = serde_json::json!([]);
    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    restichub_db::script_templates::create(
        &mut tx,
        &NewScriptTemplate {
            id: script_id,
            name: CANARY_SCRIPT_TEMPLATE,
            description: "",
            interpreter: "bash",
            phase: "before",
            body: "true",
            timeout_seconds: 60,
            failure_policy: "abort_backup",
            parameters: &parameters,
            created_by: None,
        },
    )
    .await
    .expect("cannot create the script template");
    restichub_db::script_templates::create_job_template(
        &mut tx,
        &NewJobTemplate {
            id: job_id,
            name: CANARY_JOB_TEMPLATE,
            description: "",
            scripts: &scripts,
            created_by: None,
        },
    )
    .await
    .expect("cannot create the job template");
    tx.commit().await.expect("cannot commit the templates");
    (script_id, job_id)
}

async fn snapshot_tenant(db: &TestDatabase, tenant: TenantId) -> BTreeMap<String, String> {
    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT table_name FROM information_schema.columns \
         WHERE table_schema = 'public' AND column_name = 'tenant_id' ORDER BY table_name",
    )
    .fetch_all(db.owner_pool())
    .await
    .expect("cannot list the tenant tables");
    let mut tx = db
        .owner_pool()
        .begin()
        .await
        .expect("cannot begin the owner transaction");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    let mut snapshot = BTreeMap::new();
    for (table,) in tables {
        let query = format!(
            "SELECT coalesce(string_agg(row_to_json(t)::text, E'\\n' \
             ORDER BY row_to_json(t)::text), '') FROM \"{table}\" t"
        );
        let (dump,): (String,) = sqlx::query_as(sqlx::AssertSqlSafe(query))
            .fetch_one(&mut *tx)
            .await
            .expect("cannot dump the tenant rows");
        snapshot.insert(table, dump);
    }
    tx.commit().await.expect("cannot commit the snapshot");
    snapshot
}

/// The `(object_id, diff)` of each audit record of a tenant.
async fn audit_rows(db: &TestDatabase, tenant: TenantId) -> Vec<(String, String)> {
    let mut tx = db
        .owner_pool()
        .begin()
        .await
        .expect("cannot begin the owner transaction");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT coalesce(object_id::text, ''), coalesce(diff::text, '') FROM audit_log",
    )
    .fetch_all(&mut *tx)
    .await
    .expect("cannot read the audit records");
    tx.commit().await.expect("cannot commit the audit read");
    rows
}
