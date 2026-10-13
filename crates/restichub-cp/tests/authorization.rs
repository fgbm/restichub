//! The role and operation matrix (AC-AU-02) and the cross-tenant token isolation (AC-TN-04).

mod common;

use axum::http::StatusCode;
use restichub_cp::context::{Access, Operation, minimum_role};
use restichub_cp::routes::ROUTES;
use restichub_db::{PLATFORM_TENANT_ID, Role, api_tokens, audit, memberships};
use serde_json::{Value, json};

use common::{
    app, app_with_kek, create_tenant, create_tenant_with_key, create_user, request, seed_api_token,
    seed_session, send,
};

/// The four roles in the order of the requirements section 2.
const ROLES: [Role; 4] = [
    Role::PlatformAdmin,
    Role::Admin,
    Role::Operator,
    Role::Viewer,
];

/// Builds a unique username for a fresh session.
fn unique(role: Role, index: usize) -> String {
    format!("user-{}-{index}", role.as_str())
}

/// Builds a body that passes the JSON extraction for an operation.
///
/// The matrix checks the authorization result. A malformed body would stop the request before the
/// handler runs the authorization check, so the test sends a valid body. A Platform Admin acts in
/// the platform tenant, where the only valid role is `platform_admin`.
fn body_for(method: &str, path: &str, index: usize, platform_admin: bool) -> Option<Value> {
    let role = if platform_admin {
        "platform_admin"
    } else {
        "viewer"
    };
    match (method, path) {
        ("GET", _) | ("DELETE", _) => None,
        ("POST", "/api/v1/users") => Some(json!({
            "username": format!("matrix-{index}"),
            "password": "a long enough password",
            "display_name": "Matrix",
            "role": role,
        })),
        ("POST", "/api/v1/users/{id}/password") => {
            Some(json!({"new_password": "a long enough password"}))
        }
        ("POST", "/api/v1/api-tokens") => Some(json!({"name": "matrix", "role": role})),
        ("POST", "/api/v1/secrets") => Some(json!({
            "name": format!("matrix-secret-{index}"),
            "purpose": "repository_password",
            "value": "matrix-value",
        })),
        ("PUT", _) => Some(json!({"value": "matrix-value"})),
        ("POST", "/api/v1/platform/tenants") => Some(json!({
            "name": format!("matrix-{index}"),
            "first_admin": {
                "username": format!("admin-{index}"),
                "display_name": "Matrix",
                "password": "a long enough password",
            },
        })),
        ("POST", "/api/v1/auth/password") => Some(json!({
            "current_password": "wrong",
            "new_password": "a long enough password",
        })),
        ("POST", "/api/v1/auth/sign-out") => None,
        ("PATCH", _) => Some(json!({"display_name": "Matrix"})),
        _ => Some(json!({})),
    }
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
            let body = body_for(&method, route.path, index, role == Role::PlatformAdmin);
            let (status, _, _) = send(&app, request(&method, &path, Some(&cookie), body)).await;

            let allowed_by_role = role.rank() >= minimum.rank();
            // A platform operation needs the platform tenant. A tenant-only operation is refused
            // in the platform tenant.
            let tenant_ok = if operation.is_platform() {
                role == Role::PlatformAdmin
            } else if operation.is_tenant_only() {
                session_tenant != PLATFORM_TENANT_ID
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
    assert!(!Operation::UsersRead.is_tenant_only());

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
    // The platform tenant holds the platform_admin role only.
    let user = create_user(
        db.database(),
        PLATFORM_TENANT_ID,
        "root",
        "phc",
        "platform_admin",
    )
    .await;

    // A membership with a lower role in the platform tenant is refused by the database.
    let mut tx = db
        .database()
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let membership = memberships::set_role(&mut tx, user, Role::Admin).await;
    assert!(
        membership
            .as_ref()
            .is_err_and(|error| error.is_check_violation()),
        "{membership:?}"
    );
    drop(tx);

    // An API token with a lower role in the platform tenant is refused too.
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

#[tokio::test]
async fn ac_tn_04_api_token_of_tenant_a_cannot_access_tenant_b() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant_a = create_tenant(db.database(), "tenant-a").await;
    let tenant_b = create_tenant(db.database(), "tenant-b").await;

    // Seed a user and an audit record of tenant B.
    let user_b = create_user(db.database(), tenant_b, "bob", "phc", "admin").await;
    let mut tx = db
        .database()
        .begin_tenant(tenant_b)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut event = audit::AuditEvent::new(audit::AuditAction::UserCreate, "user");
    event.object_id = Some(user_b.as_uuid());
    audit::insert(&mut tx, &event)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));

    // Seed a user and an audit record of tenant A.
    let user_a = create_user(db.database(), tenant_a, "alice", "phc", "admin").await;
    let mut tx = db
        .database()
        .begin_tenant(tenant_a)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut event = audit::AuditEvent::new(audit::AuditAction::UserCreate, "user");
    event.object_id = Some(user_a.as_uuid());
    audit::insert(&mut tx, &event)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));

    let token_a = seed_api_token(db.database(), tenant_a, "admin").await;
    let app = app(db.database());

    // The token of tenant A cannot read a user of tenant B (HTTP 404).
    let (status, _, _) = send(
        &app,
        common::bearer_request(
            "GET",
            &format!("/api/v1/users/{}", user_b.as_uuid()),
            &token_a,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The audit list of tenant A holds no event of tenant B.
    let (status, _, page) = send(
        &app,
        common::bearer_request("GET", "/api/v1/audit", &token_a, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no items"));
    assert!(!items.is_empty(), "tenant A must see its own event");
    for item in items {
        assert_eq!(
            item.get("tenant_id").and_then(Value::as_str),
            Some(tenant_a.to_string().as_str()),
            "the audit list of tenant A holds an event of another tenant"
        );
    }

    // The token of tenant A cannot read a user of tenant B through the list either.
    let (status, _, page) = send(
        &app,
        common::bearer_request("GET", "/api/v1/users", &token_a, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no items"));
    for item in items {
        assert_ne!(
            item.get("id").and_then(Value::as_str),
            Some(user_b.to_string().as_str())
        );
    }

    db.cleanup().await;
}
