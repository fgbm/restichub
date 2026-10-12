//! The request context of an authenticated request (ADR-0023, section Roles).
//!
//! The extractor [`RequestContext`] authenticates a request from the session cookie or from the
//! `Authorization: Bearer` header. It builds the tenant, the actor, the role and the session.
//!
//! A request that holds a session cookie and a bearer token at the same time gives HTTP 400
//! `ambiguous_credentials`. A request with no credential gives HTTP 401 `unauthenticated`.
//!
//! A cookie-authenticated unsafe method needs the header `X-ResticHub-CSRF: 1`. A bearer-token
//! request does not need it.
//!
//! The module also holds the authorization table "operation to minimum role" and the
//! [`ROUTE_OPERATIONS`] table. The router and the AC-AU-02 test share the table.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use restichub_db::audit::Actor;
use restichub_db::{SessionId, TenantId, api_tokens, auth, memberships, sessions, tenants, users};

use crate::error::ApiError;
use crate::server::{AppState, request_id_of};
use crate::token;

/// The role of an authenticated actor (requirements section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The Platform Admin manages tenants.
    PlatformAdmin,
    /// The Admin manages one tenant.
    Admin,
    /// The Operator runs jobs.
    Operator,
    /// The Viewer reads data.
    Viewer,
}

impl Role {
    /// Returns the stable name of the role.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlatformAdmin => "platform_admin",
            Self::Admin => "admin",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
        }
    }

    /// Parses the stable name of a role.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "platform_admin" => Some(Self::PlatformAdmin),
            "admin" => Some(Self::Admin),
            "operator" => Some(Self::Operator),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }

    /// Returns the rank of the role. A higher rank includes the lower roles.
    pub fn rank(self) -> u8 {
        match self {
            Self::PlatformAdmin => 4,
            Self::Admin => 3,
            Self::Operator => 2,
            Self::Viewer => 1,
        }
    }

    /// Returns true if the role includes the minimum role.
    pub fn allows(self, minimum: Self) -> bool {
        self.rank() >= minimum.rank()
    }
}

/// An operation of the API.
///
/// The authorization table gives the minimum role of each operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Read the audit records of one tenant.
    AuditRead,
    /// Read the audit records of the platform.
    PlatformAuditRead,
    /// Read the tenants of the platform.
    PlatformTenantsRead,
    /// Create, suspend, resume and delete the tenants of the platform.
    PlatformTenantsWrite,
    /// Read the own user and tenant.
    AuthMe,
    /// Sign out.
    AuthSignOut,
    /// Change the own password.
    AuthPasswordChange,
    /// Read the users of the tenant.
    UsersRead,
    /// Create, change and disable the users of the tenant.
    UsersWrite,
    /// Read the API tokens of the tenant.
    ApiTokensRead,
    /// Create and revoke the API tokens of the tenant.
    ApiTokensWrite,
    /// Read the metadata of the secrets of the tenant.
    SecretsRead,
    /// Create, replace and remove the secrets of the tenant.
    SecretsWrite,
}

impl Operation {
    /// Returns the stable name of the operation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AuditRead => "audit.read",
            Self::PlatformAuditRead => "platform.audit.read",
            Self::PlatformTenantsRead => "platform.tenants.read",
            Self::PlatformTenantsWrite => "platform.tenants.write",
            Self::AuthMe => "auth.me",
            Self::AuthSignOut => "auth.sign_out",
            Self::AuthPasswordChange => "auth.password",
            Self::UsersRead => "users.read",
            Self::UsersWrite => "users.write",
            Self::ApiTokensRead => "api_tokens.read",
            Self::ApiTokensWrite => "api_tokens.write",
            Self::SecretsRead => "secrets.read",
            Self::SecretsWrite => "secrets.write",
        }
    }

    /// Returns true if the operation acts on the platform.
    pub fn is_platform(self) -> bool {
        matches!(
            self,
            Self::PlatformAuditRead | Self::PlatformTenantsRead | Self::PlatformTenantsWrite
        )
    }

    /// Returns true if the operation acts in one tenant only.
    ///
    /// The control plane refuses a tenant-only operation in the platform tenant (HTTP 403). The
    /// secrets belong to a real tenant; the platform tenant holds the Platform Admin users and the
    /// platform audit records (FR-TN-02).
    pub fn is_tenant_only(self) -> bool {
        matches!(self, Self::SecretsRead | Self::SecretsWrite)
    }
}

/// Returns the minimum role of an operation (ADR-0023, section Roles).
pub fn minimum_role(operation: Operation) -> Role {
    match operation {
        Operation::AuditRead => Role::Viewer,
        Operation::PlatformAuditRead => Role::PlatformAdmin,
        Operation::PlatformTenantsRead => Role::PlatformAdmin,
        Operation::PlatformTenantsWrite => Role::PlatformAdmin,
        Operation::AuthMe => Role::Viewer,
        Operation::AuthSignOut => Role::Viewer,
        Operation::AuthPasswordChange => Role::Viewer,
        Operation::UsersRead => Role::Admin,
        Operation::UsersWrite => Role::Admin,
        Operation::ApiTokensRead => Role::Admin,
        Operation::ApiTokensWrite => Role::Admin,
        Operation::SecretsRead => Role::Viewer,
        Operation::SecretsWrite => Role::Admin,
    }
}

/// Returns the role of a request body, or an error when the role is not allowed.
///
/// The function applies the rules:
///
/// - The platform tenant holds the role `platform_admin` only (FR-TN-02). The roles Admin,
///   Operator and Viewer apply in one tenant.
/// - The role `platform_admin` is valid in the platform tenant only.
/// - An actor cannot grant a role above its own. A token cannot create a token with a higher role
///   than its own, and a user cannot either.
pub fn allowed_role(
    context: &RequestContext,
    value: &str,
    request_id: &str,
) -> Result<Role, ApiError> {
    let role = Role::parse(value).ok_or_else(|| ApiError::invalid_argument(request_id))?;
    let platform = context.tenant_id == restichub_db::PLATFORM_TENANT_ID;
    if platform && role != Role::PlatformAdmin {
        return Err(ApiError::forbidden(request_id));
    }
    if !platform && role == Role::PlatformAdmin {
        return Err(ApiError::forbidden(request_id));
    }
    if !context.role.allows(role) {
        return Err(ApiError::forbidden(request_id));
    }
    Ok(role)
}

/// Refuses an operation on a target whose role rank is above the actor's.
///
/// An actor cannot change, disable, delete or reset the password of a user with a higher role.
pub fn ensure_target_not_above_actor(
    context: &RequestContext,
    target_role: &str,
    request_id: &str,
) -> Result<(), ApiError> {
    let target = Role::parse(target_role).ok_or_else(|| ApiError::internal_error(request_id))?;
    if !context.role.allows(target) {
        return Err(ApiError::forbidden(request_id));
    }
    Ok(())
}

/// The access rule of one route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// The route needs no authentication.
    Public,
    /// The route needs the minimum role of the operation.
    Protected(Operation),
}

/// One route and its access rule.
#[derive(Debug, Clone, Copy)]
pub struct RouteOperation {
    /// The HTTP method in lower case, as the OpenAPI document holds it.
    pub method: &'static str,
    /// The path template, as the OpenAPI document holds it.
    pub path: &'static str,
    /// The access rule.
    pub access: Access,
}

/// The operation of each route.
///
/// The router and the AC-AU-02 test share the table. Each OpenAPI operation has exactly one entry.
/// The public operations are listed here as [`Access::Public`].
pub const ROUTE_OPERATIONS: &[RouteOperation] = &[
    RouteOperation {
        method: "get",
        path: "/healthz",
        access: Access::Public,
    },
    RouteOperation {
        method: "get",
        path: "/readyz",
        access: Access::Public,
    },
    RouteOperation {
        method: "get",
        path: "/api/v1/openapi.json",
        access: Access::Public,
    },
    RouteOperation {
        method: "post",
        path: "/api/v1/auth/sign-in",
        access: Access::Public,
    },
    RouteOperation {
        method: "post",
        path: "/api/v1/auth/sign-out",
        access: Access::Protected(Operation::AuthSignOut),
    },
    RouteOperation {
        method: "get",
        path: "/api/v1/auth/me",
        access: Access::Protected(Operation::AuthMe),
    },
    RouteOperation {
        method: "post",
        path: "/api/v1/auth/password",
        access: Access::Protected(Operation::AuthPasswordChange),
    },
    RouteOperation {
        method: "post",
        path: "/api/v1/setup",
        access: Access::Public,
    },
    RouteOperation {
        method: "get",
        path: "/api/v1/setup/status",
        access: Access::Public,
    },
    RouteOperation {
        method: "get",
        path: "/api/v1/audit",
        access: Access::Protected(Operation::AuditRead),
    },
    RouteOperation {
        method: "get",
        path: "/api/v1/platform/audit",
        access: Access::Protected(Operation::PlatformAuditRead),
    },
    RouteOperation {
        method: "get",
        path: "/api/v1/platform/tenants",
        access: Access::Protected(Operation::PlatformTenantsRead),
    },
    RouteOperation {
        method: "post",
        path: "/api/v1/platform/tenants",
        access: Access::Protected(Operation::PlatformTenantsWrite),
    },
    RouteOperation {
        method: "get",
        path: "/api/v1/platform/tenants/{id}",
        access: Access::Protected(Operation::PlatformTenantsRead),
    },
    RouteOperation {
        method: "post",
        path: "/api/v1/platform/tenants/{id}/suspend",
        access: Access::Protected(Operation::PlatformTenantsWrite),
    },
    RouteOperation {
        method: "post",
        path: "/api/v1/platform/tenants/{id}/resume",
        access: Access::Protected(Operation::PlatformTenantsWrite),
    },
    RouteOperation {
        method: "delete",
        path: "/api/v1/platform/tenants/{id}",
        access: Access::Protected(Operation::PlatformTenantsWrite),
    },
    RouteOperation {
        method: "get",
        path: "/api/v1/users",
        access: Access::Protected(Operation::UsersRead),
    },
    RouteOperation {
        method: "post",
        path: "/api/v1/users",
        access: Access::Protected(Operation::UsersWrite),
    },
    RouteOperation {
        method: "get",
        path: "/api/v1/users/{id}",
        access: Access::Protected(Operation::UsersRead),
    },
    RouteOperation {
        method: "patch",
        path: "/api/v1/users/{id}",
        access: Access::Protected(Operation::UsersWrite),
    },
    RouteOperation {
        method: "delete",
        path: "/api/v1/users/{id}",
        access: Access::Protected(Operation::UsersWrite),
    },
    RouteOperation {
        method: "post",
        path: "/api/v1/users/{id}/password",
        access: Access::Protected(Operation::UsersWrite),
    },
    RouteOperation {
        method: "get",
        path: "/api/v1/api-tokens",
        access: Access::Protected(Operation::ApiTokensRead),
    },
    RouteOperation {
        method: "post",
        path: "/api/v1/api-tokens",
        access: Access::Protected(Operation::ApiTokensWrite),
    },
    RouteOperation {
        method: "delete",
        path: "/api/v1/api-tokens/{id}",
        access: Access::Protected(Operation::ApiTokensWrite),
    },
    RouteOperation {
        method: "get",
        path: "/api/v1/secrets",
        access: Access::Protected(Operation::SecretsRead),
    },
    RouteOperation {
        method: "post",
        path: "/api/v1/secrets",
        access: Access::Protected(Operation::SecretsWrite),
    },
    RouteOperation {
        method: "get",
        path: "/api/v1/secrets/{id}",
        access: Access::Protected(Operation::SecretsRead),
    },
    RouteOperation {
        method: "put",
        path: "/api/v1/secrets/{id}/value",
        access: Access::Protected(Operation::SecretsWrite),
    },
    RouteOperation {
        method: "delete",
        path: "/api/v1/secrets/{id}",
        access: Access::Protected(Operation::SecretsWrite),
    },
];

/// The tenant, the actor, the role and the session of a request.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// The tenant of the request.
    pub tenant_id: TenantId,
    /// The actor of the request.
    pub actor: Actor,
    /// The role of the actor.
    pub role: Role,
    /// The session of the request. It is absent for an API token.
    pub session_id: Option<SessionId>,
}

impl RequestContext {
    /// Creates a request context.
    pub fn new(tenant_id: TenantId, actor: Actor, role: Role) -> Self {
        Self {
            tenant_id,
            actor,
            role,
            session_id: None,
        }
    }

    /// Checks the role of the context against the minimum role of the operation.
    ///
    /// A platform operation needs the platform tenant too. A tenant-only operation is refused in
    /// the platform tenant.
    pub fn authorize(&self, operation: Operation, request_id: &str) -> Result<(), ApiError> {
        if operation.is_platform() && self.tenant_id != restichub_db::PLATFORM_TENANT_ID {
            return Err(ApiError::forbidden(request_id));
        }
        if operation.is_tenant_only() && self.tenant_id == restichub_db::PLATFORM_TENANT_ID {
            return Err(ApiError::forbidden(request_id));
        }
        if self.role.allows(minimum_role(operation)) {
            Ok(())
        } else {
            Err(ApiError::forbidden(request_id))
        }
    }
}

impl FromRequestParts<AppState> for RequestContext {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let request_id = request_id_of(parts);
        let name = token::session_cookie_name(state.auth().config.insecure_cookies);
        let cookie = token::session_cookie(&parts.headers, name);
        let bearer = token::bearer_token(&parts.headers);
        match (cookie, bearer) {
            (Some(_), Some(_)) => Err(ApiError::ambiguous_credentials(&request_id)),
            (Some(value), None) => {
                let context = authenticate_session(state, &value, &request_id).await?;
                // A cookie-authenticated unsafe method needs the CSRF header.
                if token::is_unsafe_method(&parts.method) && !token::csrf_ok(&parts.headers) {
                    return Err(ApiError::csrf_required(&request_id));
                }
                Ok(context)
            }
            (None, Some(value)) => authenticate_bearer(state, &value, &request_id).await,
            (None, None) => Err(ApiError::unauthenticated(&request_id)),
        }
    }
}

/// Authenticates a request from a session cookie value.
async fn authenticate_session(
    state: &AppState,
    value: &str,
    request_id: &str,
) -> Result<RequestContext, ApiError> {
    let hash = token::hash_token(value);
    let lookup = auth::find_session(state.db(), &hash)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    let Some(lookup) = lookup else {
        return Err(ApiError::unauthenticated(request_id));
    };
    let now = state.auth().clock.now();
    if lookup.expires_at <= now {
        tracing::debug!(request_id = %request_id, "the session is past its absolute lifetime");
        return Err(ApiError::unauthenticated(request_id));
    }
    let idle_deadline = lookup.last_seen_at + state.auth().config.session_idle;
    if idle_deadline <= now {
        tracing::debug!(request_id = %request_id, "the session is idle");
        return Err(ApiError::unauthenticated(request_id));
    }
    let mut tx = state
        .db()
        .begin_tenant(lookup.tenant_id)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    let tenant = tenants::get(&mut tx, lookup.tenant_id)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    if tenant.is_none_or(|tenant| tenant.status != "active") {
        tracing::debug!(request_id = %request_id, "the tenant is not active");
        return Err(ApiError::unauthenticated(request_id));
    }
    let user = users::get(&mut tx, lookup.user_id)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    let Some(user) = user else {
        return Err(ApiError::unauthenticated(request_id));
    };
    if user.disabled {
        tracing::debug!(request_id = %request_id, "the user is disabled");
        return Err(ApiError::unauthenticated(request_id));
    }
    let role = memberships::role_of(&mut tx, lookup.user_id)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    let Some(role) = role.and_then(|role| Role::parse(&role)) else {
        return Err(ApiError::unauthenticated(request_id));
    };
    sessions::touch(&mut tx, lookup.session_id, now)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    Ok(RequestContext {
        tenant_id: lookup.tenant_id,
        actor: Actor::User(lookup.user_id),
        role,
        session_id: Some(lookup.session_id),
    })
}

/// Authenticates a request from a bearer token value.
async fn authenticate_bearer(
    state: &AppState,
    value: &str,
    request_id: &str,
) -> Result<RequestContext, ApiError> {
    let hash = token::hash_token(value);
    let lookup = auth::find_api_token(state.db(), &hash)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    let Some(lookup) = lookup else {
        return Err(ApiError::unauthenticated(request_id));
    };
    let now = state.auth().clock.now();
    if lookup.revoked_at.is_some() {
        return Err(ApiError::unauthenticated(request_id));
    }
    if lookup.expires_at.is_some_and(|expires| expires <= now) {
        return Err(ApiError::unauthenticated(request_id));
    }
    let mut tx = state
        .db()
        .begin_tenant(lookup.tenant_id)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    let tenant = tenants::get(&mut tx, lookup.tenant_id)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    if tenant.is_none_or(|tenant| tenant.status != "active") {
        return Err(ApiError::unauthenticated(request_id));
    }
    let Some(role) = Role::parse(&lookup.role) else {
        return Err(ApiError::unauthenticated(request_id));
    };
    api_tokens::touch(&mut tx, lookup.token_id, now)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    Ok(RequestContext {
        tenant_id: lookup.tenant_id,
        actor: Actor::ApiToken(lookup.token_id),
        role,
        session_id: None,
    })
}

#[cfg(test)]
mod tests {
    use restichub_db::TenantId;
    use restichub_db::audit::Actor;
    use uuid::Uuid;

    use super::{RequestContext, Role, ensure_target_not_above_actor};

    /// Builds a request context with a role.
    fn context(role: Role) -> RequestContext {
        RequestContext::new(TenantId::from_uuid(Uuid::now_v7()), Actor::System, role)
    }

    #[test]
    fn an_actor_cannot_touch_a_higher_role() {
        assert!(ensure_target_not_above_actor(&context(Role::Admin), "admin", "r").is_ok());
        assert!(ensure_target_not_above_actor(&context(Role::Admin), "viewer", "r").is_ok());
        assert!(
            ensure_target_not_above_actor(&context(Role::Admin), "platform_admin", "r").is_err()
        );
        assert!(ensure_target_not_above_actor(&context(Role::Viewer), "operator", "r").is_err());
    }

    #[test]
    fn an_unknown_target_role_is_refused() {
        assert!(ensure_target_not_above_actor(&context(Role::Admin), "nope", "r").is_err());
    }
}
