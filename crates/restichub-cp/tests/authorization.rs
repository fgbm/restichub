//! The role and operation matrix (AC-AU-02). The cross-tenant API token isolation (AC-TN-04) is in
//! `ac_tn_04.rs`.

mod common;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use restichub_cp::about::about;
use restichub_cp::context::{Access, Operation, RequestContext, minimum_role};
use restichub_cp::environment::is_safe_fact;
use restichub_cp::routes::ROUTES;
use restichub_cp::server::AppState;
use restichub_db::audit::Actor;
use restichub_db::{PLATFORM_TENANT_ID, Role, api_tokens};
use serde_json::Value;

use common::{
    app, create_tenant_with_key, create_user, request, seed_session, send, send_response,
};

/// The four roles in the order of the requirements section 2.
const ROLES: [Role; 4] = [
    Role::PlatformAdmin,
    Role::Admin,
    Role::Operator,
    Role::Viewer,
];

/// Builds a username that is not in use for a fresh session.
fn unique(role: Role, index: usize) -> String {
    format!("user-{}-{index}", role.as_str())
}

#[tokio::test]
async fn ac_au_02_the_router_route_list_matches_the_openapi_paths() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app(db.database());
    let (status, _, document) =
        send(&app, request("GET", "/api/v1/openapi.json", None, None)).await;
    assert_eq!(status, StatusCode::OK);

    let paths = document
        .get("paths")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("the document holds no paths"));

    let schemes = document
        .pointer("/components/securitySchemes")
        .unwrap_or_else(|| panic!("the document holds no security schemes"));
    assert!(
        schemes.get("cookie").is_some(),
        "the cookie scheme is missing"
    );
    assert!(
        schemes.get("bearer").is_some(),
        "the bearer scheme is missing"
    );

    // Every OpenAPI operation has exactly one entry in the table, and the other way round.
    let mut document_operations = Vec::new();
    for (path, item) in paths {
        let methods = item
            .as_object()
            .unwrap_or_else(|| panic!("the path {path} is not an object"));
        for method in methods.keys() {
            document_operations.push((method.clone(), path.clone()));
        }
    }
    let mut table_operations: Vec<(String, String)> = ROUTES
        .iter()
        .map(|route| (route.method.to_owned(), route.path.to_owned()))
        .collect();

    document_operations.sort();
    table_operations.sort();
    assert_eq!(
        document_operations, table_operations,
        "the OpenAPI operations and the route table differ"
    );

    // Each pair appears exactly once.
    for route in ROUTES {
        let count = ROUTES
            .iter()
            .filter(|other| other.method == route.method && other.path == route.path)
            .count();
        assert_eq!(count, 1, "the route {route:?} is not unique");
    }

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_02_every_protected_route_documents_security() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app(db.database());
    let (status, _, document) =
        send(&app, request("GET", "/api/v1/openapi.json", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    let paths = document
        .get("paths")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("the document holds no paths"));

    // The OpenAPI `security` of an operation is present exactly when the route is not public.
    for route in ROUTES {
        let operation = paths
            .get(route.path)
            .and_then(|item| item.get(route.method))
            .unwrap_or_else(|| panic!("the document holds no {} {}", route.method, route.path));
        let has_security = operation.get("security").is_some();
        let protected = !matches!(route.access, Access::Public);
        assert_eq!(
            has_security, protected,
            "{} {}: the security document does not match the access rule",
            route.method, route.path
        );
    }

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_02_role_operation_matrix() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "matrix").await;
    // The matrix runs in multi mode: the helper sets it. Single mode covers the platform routes
    // in ac_tn_06 (ADR-0024). The key compromise route needs the loaded job signing key set.
    let app = common::app_with_pki(db.database()).await;
    let id = uuid::Uuid::now_v7().to_string();

    let mut index = 0;
    for route in ROUTES {
        let Access::Protected(operation) = route.access else {
            continue;
        };
        let minimum = minimum_role(operation);
        for role in ROLES {
            index += 1;
            let session_tenant = if role == Role::PlatformAdmin {
                PLATFORM_TENANT_ID
            } else {
                tenant
            };
            let cookie = seed_session(
                db.database(),
                session_tenant,
                &unique(role, index),
                role.as_str(),
            )
            .await;
            let path = route.path.replace("{id}", &id);
            let method = route.method.to_ascii_uppercase();
            let body = common::write_body(
                &method,
                route.path,
                index,
                role == Role::PlatformAdmin,
                &session_tenant.to_string(),
                &id,
                &id,
                &id,
            );
            // The status is enough, and the event stream never ends, so the body stays unread.
            let status = send_response(&app, request(&method, &path, Some(&cookie), body))
                .await
                .status();

            let allowed_by_role = role.rank() >= minimum.rank();
            // A platform operation needs the Platform Admin right (ADR-0029). A tenant data
            // operation is refused in the platform tenant, which holds no organization.
            // The platform tenant is an allow-list: only the account operations pass there, next to
            // the platform operations that need the right (ADR-0029).
            let tenant_ok = if operation.is_platform() {
                role == Role::PlatformAdmin
            } else if session_tenant == PLATFORM_TENANT_ID {
                operation.is_auth()
            } else {
                true
            };
            let expected_forbidden = !(allowed_by_role && tenant_ok);
            assert_eq!(
                status == StatusCode::FORBIDDEN,
                expected_forbidden,
                "{method} {path} as {} gave {status}",
                role.as_str()
            );
            assert_ne!(
                status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "{method} {path} as {} gave an internal error",
                role.as_str()
            );
        }
    }

    // A Viewer gets 403 for every write operation.
    for operation in [
        Operation::UsersWrite,
        Operation::ApiTokensWrite,
        Operation::SecretsWrite,
        Operation::RepositoriesWrite,
    ] {
        assert_eq!(minimum_role(operation), Role::Admin);
        assert!(!Role::Viewer.allows(minimum_role(operation)));
    }
    // An Operator cannot manage users. A Viewer reads the metadata of the secrets.
    assert!(!Role::Operator.allows(minimum_role(Operation::UsersRead)));
    assert!(!Role::Operator.allows(minimum_role(Operation::UsersWrite)));
    assert_eq!(minimum_role(Operation::SecretsRead), Role::Viewer);
    assert!(Role::Viewer.allows(minimum_role(Operation::SecretsRead)));
    assert!(!Role::Operator.allows(minimum_role(Operation::SecretsWrite)));
    assert!(Operation::SecretsRead.is_tenant_only());
    assert!(Operation::SecretsWrite.is_tenant_only());
    assert_eq!(minimum_role(Operation::RepositoriesRead), Role::Viewer);
    assert!(Role::Viewer.allows(minimum_role(Operation::RepositoriesRead)));
    assert!(Operation::RepositoriesRead.is_tenant_only());
    assert!(Operation::RepositoriesWrite.is_tenant_only());
    assert!(!Operation::UsersRead.is_tenant_only());
    // An Operator manages the plans too; a Viewer reads them (requirements section 2).
    assert_eq!(minimum_role(Operation::PlansRead), Role::Viewer);
    assert_eq!(minimum_role(Operation::PlansWrite), Role::Operator);
    assert!(Role::Viewer.allows(minimum_role(Operation::PlansRead)));
    assert!(!Role::Viewer.allows(minimum_role(Operation::PlansWrite)));
    assert!(Role::Operator.allows(minimum_role(Operation::PlansWrite)));
    assert!(Operation::PlansRead.is_tenant_only());
    assert!(Operation::PlansWrite.is_tenant_only());
    // The account operations are the allow-list of the platform tenant.
    assert!(Operation::AuthMe.is_auth());
    assert!(Operation::AuthPasswordChange.is_auth());
    assert!(!Operation::UsersRead.is_auth());
    assert!(!Operation::AgentsRead.is_auth());

    // Only the Platform Admin manages the tenants, and only in the platform tenant.
    for operation in [
        Operation::PlatformTenantsRead,
        Operation::PlatformTenantsWrite,
    ] {
        assert_eq!(minimum_role(operation), Role::PlatformAdmin);
        assert!(operation.is_platform());
        assert!(!Role::Admin.allows(minimum_role(operation)));
    }

    db.cleanup().await;
}

/// The About ResticHub data is for every role, and the schema level is for the Platform Admin only
/// (AC-AU-02, ADR-0054).
///
/// The version goes to support, so a Viewer reads it. The field set is exactly the visible one, and
/// the environment holds no host fact (ADR-0055).
#[tokio::test]
async fn adr_0054_about_is_readable_by_a_viewer_and_hides_the_schema_level() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "about").await;
    let app = app(db.database());

    let viewer = seed_session(db.database(), tenant, "about-viewer", "viewer").await;
    let (status, _, body) = send(&app, request("GET", "/api/v1/about", Some(&viewer), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut fields: Vec<&str> = body
        .as_object()
        .unwrap_or_else(|| panic!("the body is no object: {body}"))
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        [
            "agent_protocol",
            "environment",
            "prerelease",
            "protocol",
            "revision",
            "version"
        ],
        "{body}"
    );
    assert_ne!(body["version"].as_str(), Some(""));
    assert_eq!(
        body["protocol"]["minor"].as_u64(),
        Some(u64::from(restichub_proto::PROTOCOL_VERSION_MINOR))
    );
    assert_eq!(
        body["agent_protocol"]["minimum_minor"].as_u64(),
        Some(u64::from(
            restichub_proto::version::Version::CURRENT.minimum_minor()
        ))
    );

    let operator = seed_session(db.database(), tenant, "about-operator", "operator").await;
    let (status, _, body) =
        send(&app, request("GET", "/api/v1/about", Some(&operator), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("version").is_some(), "{body}");
    assert!(body.get("schema_level").is_none(), "{body}");

    db.cleanup().await;
}

/// The About answer reports a broken installation, so it arrives without the database version while
/// the database answers no query (ADR-0055).
///
/// A request authenticates against the database, so the test calls the route handler with the
/// context of a Viewer. The handler is the part of the request that runs after the authentication.
#[tokio::test]
async fn the_about_answer_survives_a_database_that_takes_no_connection() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "about-without-database").await;
    let viewer = create_user(
        db.database(),
        tenant,
        "about-without-database-user",
        "phc",
        "viewer",
    )
    .await;
    let state = AppState::new(db.database().clone());
    let context = RequestContext::new(tenant, Actor::User(viewer), Role::Viewer);
    db.database().clone().close().await;

    let Json(answer) = about(State(state), context)
        .await
        .expect("the About answer needs no database");
    let environment =
        serde_json::to_value(&answer.environment).expect("the environment serialises");
    assert!(environment.get("database").is_none(), "{environment}");
    assert_eq!(environment["os"].as_str(), Some(std::env::consts::OS));

    db.cleanup().await;
}

/// The schema level is the second fact of a broken installation, so the About answer of the
/// Platform Admin arrives without it while the database answers no query (ADR-0054).
#[tokio::test]
async fn the_about_answer_of_a_platform_admin_survives_a_database_that_takes_no_connection() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "about-schema-without-database").await;
    let admin = create_user(
        db.database(),
        tenant,
        "about-schema-without-database-user",
        "phc",
        "platform_admin",
    )
    .await;
    let state = AppState::new(db.database().clone());
    let mut context =
        RequestContext::new(PLATFORM_TENANT_ID, Actor::User(admin), Role::PlatformAdmin);
    context.platform_admin = true;
    db.database().clone().close().await;

    let Json(answer) = about(State(state), context)
        .await
        .expect("the About answer needs no database");
    let body = serde_json::to_value(&answer).expect("the answer serialises");
    assert!(body.get("schema_level").is_none(), "{body}");

    db.cleanup().await;
}

/// A Viewer gives the environment of the server to support (ADR-0055).
///
/// The test does not search the body for a value: it compares the set of the fields and the shape
/// of each value. A shape holds no path, no address, no host name and no connection string.
/// A distroless image reports neither the distribution nor the kernel, so the test does not
/// demand them.
#[tokio::test]
async fn adr_0055_a_viewer_reads_the_environment_of_the_server() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "about-environment").await;
    let app = app(db.database());

    let viewer = seed_session(db.database(), tenant, "about-environment-viewer", "viewer").await;
    let (status, _, body) = send(&app, request("GET", "/api/v1/about", Some(&viewer), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let environment = body
        .get("environment")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("the body holds no environment object: {body}"));

    let mut fields: Vec<&str> = environment.keys().map(String::as_str).collect();
    fields.sort_unstable();
    let mut expected = vec!["arch", "database", "os"];
    for optional in ["container", "kernel", "os_version"] {
        if environment.contains_key(optional) {
            expected.push(optional);
        }
    }
    expected.sort_unstable();
    assert_eq!(fields, expected, "{body}");

    for key in ["os", "os_version", "kernel", "arch", "database"] {
        let Some(value) = environment.get(key) else {
            continue;
        };
        let value = value
            .as_str()
            .unwrap_or_else(|| panic!("{key} is no string: {body}"));
        assert!(is_safe_fact(value), "{key} is {value:?}: {body}");
    }
    if let Some(container) = environment.get("container") {
        assert!(container.is_boolean(), "{body}");
    }

    let database = environment["database"]
        .as_str()
        .unwrap_or_else(|| panic!("no database version: {body}"));
    assert!(
        database.chars().all(|c| c.is_ascii_digit() || c == '.')
            && database.starts_with(|c: char| c.is_ascii_digit()),
        "the PostgreSQL version is {database:?}"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_02_platform_tenant_refuses_a_lower_role() {
    let Some(db) = common::test_db().await else {
        return;
    };
    // The platform Admin holds the account right and no membership (ADR-0029).
    let user = create_user(
        db.database(),
        PLATFORM_TENANT_ID,
        "root",
        "phc",
        "platform_admin",
    )
    .await;

    // An API token with a lower role in the platform tenant is refused.
    let mut tx = db
        .database()
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let token = api_tokens::create(
        &mut tx,
        &api_tokens::NewApiToken {
            name: "lower",
            token_hash: &[1u8; 32],
            display_prefix: "rh_abcdefgh",
            role: Role::Viewer,
            created_by: user,
            expires_at: None,
        },
    )
    .await;
    assert!(
        token
            .as_ref()
            .is_err_and(|error| error.is_check_violation()),
        "{token:?}"
    );
    drop(tx);

    db.cleanup().await;
}
