//! The routes table of the control plane (ADR-0023).
//!
//! One table holds every route: the method, the path, the access rule and the handler. The router
//! and the OpenAPI document of [`crate::server`] derive from the table, and the AC-AU-02 and
//! AC-TN-01 tests read it. Each handler declares its operation in the table row.
//!
//! The table is a declaration for the tests, not the access check. Each handler calls
//! [`crate::context::RequestContext::authorize`] with its operation; that call is the real check.
//! The AC-AU-02 test reads the table to prove that the two agree.

use utoipa_axum::router::{UtoipaMethodRouter, UtoipaMethodRouterExt};
use utoipa_axum::routes;

use crate::context::{Access, Operation};
use crate::server::{AppState, BODY_LIMIT, SECRET_BODY_LIMIT};

/// One route and its access rule.
#[derive(Debug)]
pub struct Route {
    /// The HTTP method in lower case, as the OpenAPI document holds it.
    pub method: &'static str,
    /// The path template, as the OpenAPI document holds it.
    pub path: &'static str,
    /// The access rule.
    pub access: Access,
    /// Builds the method router and the OpenAPI paths of the handler.
    pub handler: fn() -> UtoipaMethodRouter<AppState>,
}

/// Every route of the control plane.
pub const ROUTES: &[Route] = &[
    Route {
        method: "get",
        path: "/healthz",
        access: Access::Public,
        handler: || routes!(crate::server::healthz),
    },
    Route {
        method: "get",
        path: "/readyz",
        access: Access::Public,
        handler: || routes!(crate::server::readyz),
    },
    Route {
        method: "get",
        path: "/api/v1/openapi.json",
        access: Access::Public,
        handler: || routes!(crate::server::openapi),
    },
    Route {
        method: "post",
        path: "/api/v1/auth/sign-in",
        access: Access::Public,
        handler: || {
            routes!(crate::auth::sign_in).layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "post",
        path: "/api/v1/auth/sign-out",
        access: Access::Protected(Operation::AuthSignOut),
        handler: || routes!(crate::auth::sign_out),
    },
    Route {
        method: "get",
        path: "/api/v1/auth/me",
        access: Access::Protected(Operation::AuthMe),
        handler: || routes!(crate::auth::me),
    },
    Route {
        method: "post",
        path: "/api/v1/auth/password",
        access: Access::Protected(Operation::AuthPasswordChange),
        handler: || {
            routes!(crate::auth::change_password)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "post",
        path: "/api/v1/setup",
        access: Access::Public,
        handler: || {
            routes!(crate::setup::setup).layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "get",
        path: "/api/v1/setup/status",
        access: Access::Public,
        handler: || routes!(crate::setup::status),
    },
    Route {
        method: "get",
        path: "/api/v1/audit",
        access: Access::Protected(Operation::AuditRead),
        handler: || routes!(crate::audit::list_tenant_audit),
    },
    Route {
        method: "get",
        path: "/api/v1/platform/audit",
        access: Access::Protected(Operation::PlatformAuditRead),
        handler: || routes!(crate::audit::list_platform_audit),
    },
    Route {
        method: "get",
        path: "/api/v1/platform/tenants",
        access: Access::Protected(Operation::PlatformTenantsRead),
        handler: || routes!(crate::tenants::list_tenants),
    },
    Route {
        method: "post",
        path: "/api/v1/platform/tenants",
        access: Access::Protected(Operation::PlatformTenantsWrite),
        handler: || {
            routes!(crate::tenants::create_tenant)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "get",
        path: "/api/v1/platform/tenants/{id}",
        access: Access::Protected(Operation::PlatformTenantsRead),
        handler: || routes!(crate::tenants::get_tenant),
    },
    Route {
        method: "post",
        path: "/api/v1/platform/tenants/{id}/suspend",
        access: Access::Protected(Operation::PlatformTenantsWrite),
        handler: || routes!(crate::tenants::suspend_tenant),
    },
    Route {
        method: "post",
        path: "/api/v1/platform/tenants/{id}/resume",
        access: Access::Protected(Operation::PlatformTenantsWrite),
        handler: || routes!(crate::tenants::resume_tenant),
    },
    Route {
        method: "delete",
        path: "/api/v1/platform/tenants/{id}",
        access: Access::Protected(Operation::PlatformTenantsWrite),
        handler: || routes!(crate::tenants::delete_tenant),
    },
    Route {
        method: "get",
        path: "/api/v1/users",
        access: Access::Protected(Operation::UsersRead),
        handler: || routes!(crate::users::list_users),
    },
    Route {
        method: "post",
        path: "/api/v1/users",
        access: Access::Protected(Operation::UsersWrite),
        handler: || {
            routes!(crate::users::create_user)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "get",
        path: "/api/v1/users/{id}",
        access: Access::Protected(Operation::UsersRead),
        handler: || routes!(crate::users::get_user),
    },
    Route {
        method: "patch",
        path: "/api/v1/users/{id}",
        access: Access::Protected(Operation::UsersWrite),
        handler: || {
            routes!(crate::users::update_user)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "delete",
        path: "/api/v1/users/{id}",
        access: Access::Protected(Operation::UsersWrite),
        handler: || routes!(crate::users::delete_user),
    },
    Route {
        method: "post",
        path: "/api/v1/users/{id}/password",
        access: Access::Protected(Operation::UsersWrite),
        handler: || {
            routes!(crate::users::reset_password)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "get",
        path: "/api/v1/api-tokens",
        access: Access::Protected(Operation::ApiTokensRead),
        handler: || routes!(crate::api_tokens::list_tokens),
    },
    Route {
        method: "post",
        path: "/api/v1/api-tokens",
        access: Access::Protected(Operation::ApiTokensWrite),
        handler: || {
            routes!(crate::api_tokens::create_token)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "delete",
        path: "/api/v1/api-tokens/{id}",
        access: Access::Protected(Operation::ApiTokensWrite),
        handler: || routes!(crate::api_tokens::revoke_token),
    },
    Route {
        method: "get",
        path: "/api/v1/secrets",
        access: Access::Protected(Operation::SecretsRead),
        handler: || routes!(crate::secrets::list_secrets),
    },
    Route {
        method: "post",
        path: "/api/v1/secrets",
        access: Access::Protected(Operation::SecretsWrite),
        handler: || {
            routes!(crate::secrets::create_secret)
                .layer(axum::extract::DefaultBodyLimit::max(SECRET_BODY_LIMIT))
        },
    },
    Route {
        method: "get",
        path: "/api/v1/secrets/{id}",
        access: Access::Protected(Operation::SecretsRead),
        handler: || routes!(crate::secrets::get_secret),
    },
    Route {
        method: "put",
        path: "/api/v1/secrets/{id}/value",
        access: Access::Protected(Operation::SecretsWrite),
        handler: || {
            routes!(crate::secrets::replace_secret_value)
                .layer(axum::extract::DefaultBodyLimit::max(SECRET_BODY_LIMIT))
        },
    },
    Route {
        method: "delete",
        path: "/api/v1/secrets/{id}",
        access: Access::Protected(Operation::SecretsWrite),
        handler: || routes!(crate::secrets::delete_secret),
    },
];
