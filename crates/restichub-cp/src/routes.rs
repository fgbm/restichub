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

#[derive(Debug)]
pub struct Route {
    /// Lower case, as the OpenAPI document holds it.
    pub method: &'static str,
    pub path: &'static str,
    pub access: Access,
    pub handler: fn() -> UtoipaMethodRouter<AppState>,
}

/// True if the path is under a platform route of the table (ADR-0024).
///
/// The guard of the router uses the function to hide the platform routes in single mode. The path
/// prefix of each platform route comes from the table, so the two agree. A path parameter cuts the
/// prefix at the first `{`.
pub fn is_platform_path(path: &str) -> bool {
    ROUTES
        .iter()
        .filter(|route| route.access.is_platform())
        .any(|route| {
            let prefix = route.path.split('{').next().unwrap_or(route.path);
            path.starts_with(prefix)
        })
}

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
        path: "/metrics",
        access: Access::Protected(Operation::MetricsRead),
        handler: || routes!(crate::metrics::metrics),
    },
    Route {
        method: "post",
        path: "/agent/v1/enroll",
        access: Access::Public,
        handler: || {
            routes!(crate::enrollment::enroll)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
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
        method: "get",
        path: "/api/v1/auth/tenants",
        access: Access::Protected(Operation::AuthTenants),
        handler: || routes!(crate::auth::list_tenants),
    },
    Route {
        method: "post",
        path: "/api/v1/auth/switch",
        access: Access::Protected(Operation::AuthSwitch),
        handler: || {
            routes!(crate::auth::switch_tenant)
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
        path: "/api/v1/instance",
        access: Access::Public,
        handler: || routes!(crate::instance::instance),
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
        path: "/api/v1/platform/users",
        access: Access::Protected(Operation::PlatformUsersRead),
        handler: || routes!(crate::platform_users::list_accounts),
    },
    Route {
        method: "post",
        path: "/api/v1/platform/users",
        access: Access::Protected(Operation::PlatformUsersWrite),
        handler: || {
            routes!(crate::platform_users::create_account)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "post",
        path: "/api/v1/platform/users/{id}/memberships",
        access: Access::Protected(Operation::PlatformUsersWrite),
        handler: || {
            routes!(crate::platform_users::add_membership)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "patch",
        path: "/api/v1/platform/users/{id}/memberships/{tenant_id}",
        access: Access::Protected(Operation::PlatformUsersWrite),
        handler: || {
            routes!(crate::platform_users::set_membership_role)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "delete",
        path: "/api/v1/platform/users/{id}/memberships/{tenant_id}",
        access: Access::Protected(Operation::PlatformUsersWrite),
        handler: || routes!(crate::platform_users::remove_membership),
    },
    Route {
        method: "patch",
        path: "/api/v1/platform/users/{id}",
        access: Access::Protected(Operation::PlatformUsersWrite),
        handler: || {
            routes!(crate::platform_users::update_account)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
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
        path: "/api/v1/agents",
        access: Access::Protected(Operation::AgentsRead),
        handler: || routes!(crate::agents::list_agents),
    },
    Route {
        method: "get",
        path: "/api/v1/agents/{id}",
        access: Access::Protected(Operation::AgentsRead),
        handler: || routes!(crate::agents::get_agent),
    },
    Route {
        method: "patch",
        path: "/api/v1/agents/{id}",
        access: Access::Protected(Operation::AgentsWrite),
        handler: || {
            routes!(crate::agents::rename_agent)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "post",
        path: "/api/v1/agents/{id}/revoke",
        access: Access::Protected(Operation::AgentsWrite),
        handler: || routes!(crate::agents::revoke_agent),
    },
    Route {
        method: "get",
        path: "/api/v1/enrollment-tokens",
        access: Access::Protected(Operation::EnrollmentTokensRead),
        handler: || routes!(crate::enrollment_tokens::list_enrollment_tokens),
    },
    Route {
        method: "post",
        path: "/api/v1/enrollment-tokens",
        access: Access::Protected(Operation::EnrollmentTokensWrite),
        handler: || {
            routes!(crate::enrollment_tokens::create_enrollment_token)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "delete",
        path: "/api/v1/enrollment-tokens/{id}",
        access: Access::Protected(Operation::EnrollmentTokensWrite),
        handler: || routes!(crate::enrollment_tokens::revoke_enrollment_token),
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
    Route {
        method: "get",
        path: "/api/v1/repositories",
        access: Access::Protected(Operation::RepositoriesRead),
        handler: || routes!(crate::repositories::list_repositories),
    },
    Route {
        method: "post",
        path: "/api/v1/repositories",
        access: Access::Protected(Operation::RepositoriesWrite),
        handler: || {
            routes!(crate::repositories::create_repository)
                .layer(axum::extract::DefaultBodyLimit::max(SECRET_BODY_LIMIT))
        },
    },
    Route {
        method: "get",
        path: "/api/v1/repositories/{id}",
        access: Access::Protected(Operation::RepositoriesRead),
        handler: || routes!(crate::repositories::get_repository),
    },
    Route {
        method: "patch",
        path: "/api/v1/repositories/{id}",
        access: Access::Protected(Operation::RepositoriesWrite),
        handler: || {
            routes!(crate::repositories::update_repository)
                .layer(axum::extract::DefaultBodyLimit::max(SECRET_BODY_LIMIT))
        },
    },
    Route {
        method: "delete",
        path: "/api/v1/repositories/{id}",
        access: Access::Protected(Operation::RepositoriesWrite),
        handler: || routes!(crate::repositories::delete_repository),
    },
    Route {
        method: "post",
        path: "/api/v1/repositories/{id}/init",
        access: Access::Protected(Operation::RepositoriesWrite),
        handler: || {
            routes!(crate::jobs::init_repository)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "post",
        path: "/api/v1/repositories/{id}/test-access",
        access: Access::Protected(Operation::RepositoriesWrite),
        handler: || {
            routes!(crate::jobs::test_access_repository)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "post",
        path: "/api/v1/repositories/{id}/unlock-all",
        access: Access::Protected(Operation::RepositoriesWrite),
        handler: || {
            routes!(crate::job_api::unlock_all_repository)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "get",
        path: "/api/v1/jobs",
        access: Access::Protected(Operation::JobsRead),
        handler: || routes!(crate::job_api::list_jobs),
    },
    Route {
        method: "get",
        path: "/api/v1/jobs/{id}",
        access: Access::Protected(Operation::JobsRead),
        handler: || routes!(crate::job_detail::get_job),
    },
    Route {
        method: "get",
        path: "/api/v1/jobs/{id}/logs",
        access: Access::Protected(Operation::JobsRead),
        handler: || routes!(crate::job_detail::get_job_logs),
    },
    Route {
        method: "post",
        path: "/api/v1/jobs/{id}/cancel",
        access: Access::Protected(Operation::JobsWrite),
        handler: || routes!(crate::job_api::cancel_job),
    },
    Route {
        method: "get",
        path: "/api/v1/profiles",
        access: Access::Protected(Operation::ProfilesRead),
        handler: || routes!(crate::profiles::list_profiles),
    },
    Route {
        method: "post",
        path: "/api/v1/profiles",
        access: Access::Protected(Operation::ProfilesWrite),
        handler: || {
            routes!(crate::profiles::create_profile)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "get",
        path: "/api/v1/profiles/{id}",
        access: Access::Protected(Operation::ProfilesRead),
        handler: || routes!(crate::profiles::get_profile),
    },
    Route {
        method: "patch",
        path: "/api/v1/profiles/{id}",
        access: Access::Protected(Operation::ProfilesWrite),
        handler: || {
            routes!(crate::profiles::update_profile)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "delete",
        path: "/api/v1/profiles/{id}",
        access: Access::Protected(Operation::ProfilesWrite),
        handler: || routes!(crate::profiles::delete_profile),
    },
    Route {
        method: "post",
        path: "/api/v1/profiles/{id}/preview",
        access: Access::Protected(Operation::ProfilesRead),
        handler: || {
            routes!(crate::profiles::preview_profile)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "get",
        path: "/api/v1/plans",
        access: Access::Protected(Operation::PlansRead),
        handler: || routes!(crate::plans::list_plans),
    },
    Route {
        method: "post",
        path: "/api/v1/plans",
        access: Access::Protected(Operation::PlansWrite),
        handler: || {
            routes!(crate::plans::create_plan)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "get",
        path: "/api/v1/plans/{id}",
        access: Access::Protected(Operation::PlansRead),
        handler: || routes!(crate::plans::get_plan),
    },
    Route {
        method: "patch",
        path: "/api/v1/plans/{id}",
        access: Access::Protected(Operation::PlansWrite),
        handler: || {
            routes!(crate::plans::update_plan)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
    Route {
        method: "delete",
        path: "/api/v1/plans/{id}",
        access: Access::Protected(Operation::PlansWrite),
        handler: || routes!(crate::plans::delete_plan),
    },
    Route {
        method: "post",
        path: "/api/v1/plans/{id}/run",
        access: Access::Protected(Operation::PlansWrite),
        handler: || routes!(crate::job_api::run_plan),
    },
    Route {
        method: "get",
        path: "/api/v1/plans/{id}/next-runs",
        access: Access::Protected(Operation::PlansRead),
        handler: || routes!(crate::plans::next_runs),
    },
    Route {
        method: "post",
        path: "/api/v1/plans/preview",
        access: Access::Protected(Operation::PlansRead),
        handler: || {
            routes!(crate::plans::preview_schedule)
                .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT))
        },
    },
];
