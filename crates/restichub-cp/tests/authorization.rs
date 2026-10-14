//! The role and operation matrix (AC-AU-02). The cross-tenant API token isolation (AC-TN-04) is in
//! `ac_tn_04.rs`.

mod common;

use axum::http::StatusCode;
use restichub_cp::context::{Access, Operation, minimum_role};
use restichub_cp::routes::ROUTES;
use restichub_db::{PLATFORM_TENANT_ID, Role, api_tokens};
use serde_json::Value;

use common::{app, app_with_kek, create_tenant_with_key, create_user, request, seed_session, send};

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
    // in ac_tn_06 (ADR-0024).
    let app = app_with_kek(db.database());
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
            let (status, _, _) = send(&app, request(&method, &path, Some(&cookie), body)).await;

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
