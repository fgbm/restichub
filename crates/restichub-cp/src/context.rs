//! The request context of an authenticated request (ADR-0023, section Roles).
//!
//! The extractor [`RequestContext`] authenticates a request from the session cookie or from the
//! `Authorization: Bearer` header. It builds the tenant, the actor, the role, the session and the
//! source IP of the request.
//!
//! A request that holds a session cookie and a bearer token at the same time gives HTTP 400
//! `ambiguous_credentials`. A request with no credential gives HTTP 401 `unauthenticated`.
//!
//! A cookie-authenticated unsafe method needs the header `X-ResticHub-CSRF: 1`. A bearer-token
//! request does not need it.
//!
//! The module also holds the authorization table "operation to minimum role". The routes table of
//! [`crate::routes`] refers to an [`Operation`], and the AC-AU-02 test reads the same table.

use std::net::IpAddr;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use restichub_db::audit::{Actor, AuditAction, AuditEvent};
use restichub_db::{
    Role, SessionId, TenantId, TenantTx, api_tokens, auth, sessions, tenants, users,
};

use crate::client_ip::ClientIp;
use crate::error::ApiError;
use crate::server::AppState;
use crate::token;

/// The authorization table gives the minimum role that each operation needs (ADR-0023, section
/// Roles).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    AuditRead,
    PlatformAuditRead,
    PlatformTenantsRead,
    PlatformTenantsWrite,
    PlatformUsersRead,
    PlatformUsersWrite,
    AuthMe,
    AuthSignOut,
    AuthPasswordChange,
    AuthTenants,
    AuthSwitch,
    UsersRead,
    UsersWrite,
    ApiTokensRead,
    ApiTokensWrite,
    AgentsRead,
    AgentsWrite,
    EnrollmentTokensRead,
    EnrollmentTokensWrite,
    SecretsRead,
    SecretsWrite,
    RepositoriesRead,
    RepositoriesWrite,
    JobsRead,
}

impl Operation {
    pub fn is_platform(self) -> bool {
        matches!(
            self,
            Self::PlatformAuditRead
                | Self::PlatformTenantsRead
                | Self::PlatformTenantsWrite
                | Self::PlatformUsersRead
                | Self::PlatformUsersWrite
        )
    }

    /// Returns true if the operation acts in one tenant only.
    ///
    /// The control plane refuses a tenant-only operation in the platform tenant (HTTP 403). The
    /// secrets belong to a real tenant; the platform tenant holds the Platform Admin users and the
    /// platform audit records (FR-TN-02).
    pub fn is_tenant_only(self) -> bool {
        matches!(
            self,
            Self::SecretsRead
                | Self::SecretsWrite
                | Self::AgentsRead
                | Self::AgentsWrite
                | Self::EnrollmentTokensRead
                | Self::EnrollmentTokensWrite
                | Self::RepositoriesRead
                | Self::RepositoriesWrite
                | Self::JobsRead
        )
    }

    /// Returns true for the account operations of an authenticated session.
    ///
    /// A session with no organization runs in the internal platform tenant. Only these operations
    /// pass there (ADR-0029).
    pub fn is_auth(self) -> bool {
        matches!(
            self,
            Self::AuthMe
                | Self::AuthSignOut
                | Self::AuthPasswordChange
                | Self::AuthTenants
                | Self::AuthSwitch
        )
    }

    /// Every operation. A new variant stops the build in `the_platform_tenant_allow_list` and in
    /// [`crate::context::Operation::classify`].
    pub const ALL: [Operation; 24] = [
        Operation::AuditRead,
        Operation::PlatformAuditRead,
        Operation::PlatformTenantsRead,
        Operation::PlatformTenantsWrite,
        Operation::PlatformUsersRead,
        Operation::PlatformUsersWrite,
        Operation::AuthMe,
        Operation::AuthSignOut,
        Operation::AuthPasswordChange,
        Operation::AuthTenants,
        Operation::AuthSwitch,
        Operation::UsersRead,
        Operation::UsersWrite,
        Operation::ApiTokensRead,
        Operation::ApiTokensWrite,
        Operation::AgentsRead,
        Operation::AgentsWrite,
        Operation::EnrollmentTokensRead,
        Operation::EnrollmentTokensWrite,
        Operation::SecretsRead,
        Operation::SecretsWrite,
        Operation::RepositoriesRead,
        Operation::RepositoriesWrite,
        Operation::JobsRead,
    ];

    /// The class of an operation, for the platform-tenant allow-list. The `match` lists every
    /// variant, so a new variant stops the build.
    pub fn classify(self) -> OperationClass {
        match self {
            Operation::AuthMe
            | Operation::AuthSignOut
            | Operation::AuthPasswordChange
            | Operation::AuthTenants
            | Operation::AuthSwitch => OperationClass::Auth,
            Operation::PlatformAuditRead
            | Operation::PlatformTenantsRead
            | Operation::PlatformTenantsWrite
            | Operation::PlatformUsersRead
            | Operation::PlatformUsersWrite => OperationClass::Platform,
            Operation::AuditRead
            | Operation::UsersRead
            | Operation::UsersWrite
            | Operation::ApiTokensRead
            | Operation::ApiTokensWrite
            | Operation::AgentsRead
            | Operation::AgentsWrite
            | Operation::EnrollmentTokensRead
            | Operation::EnrollmentTokensWrite
            | Operation::SecretsRead
            | Operation::SecretsWrite
            | Operation::RepositoriesRead
            | Operation::RepositoriesWrite
            | Operation::JobsRead => OperationClass::Tenant,
        }
    }
}

/// The class of an operation for the platform-tenant allow-list (ADR-0029).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationClass {
    /// An account operation of an authenticated session.
    Auth,
    /// A platform administration operation; it needs the Platform Admin right.
    Platform,
    /// A tenant data operation; it needs a real organization.
    Tenant,
}

/// The role that an operation needs (ADR-0023, section Roles).
pub fn minimum_role(operation: Operation) -> Role {
    match operation {
        Operation::AuditRead => Role::Viewer,
        Operation::PlatformAuditRead => Role::PlatformAdmin,
        Operation::PlatformTenantsRead => Role::PlatformAdmin,
        Operation::PlatformTenantsWrite => Role::PlatformAdmin,
        Operation::PlatformUsersRead => Role::PlatformAdmin,
        Operation::PlatformUsersWrite => Role::PlatformAdmin,
        Operation::AuthMe => Role::Viewer,
        Operation::AuthSignOut => Role::Viewer,
        Operation::AuthPasswordChange => Role::Viewer,
        Operation::AuthTenants => Role::Viewer,
        Operation::AuthSwitch => Role::Viewer,
        Operation::UsersRead => Role::Admin,
        Operation::UsersWrite => Role::Admin,
        Operation::ApiTokensRead => Role::Admin,
        Operation::ApiTokensWrite => Role::Admin,
        Operation::AgentsRead => Role::Viewer,
        Operation::AgentsWrite => Role::Admin,
        Operation::EnrollmentTokensRead => Role::Viewer,
        Operation::EnrollmentTokensWrite => Role::Admin,
        Operation::SecretsRead => Role::Viewer,
        Operation::SecretsWrite => Role::Admin,
        Operation::RepositoriesRead => Role::Viewer,
        Operation::RepositoriesWrite => Role::Admin,
        Operation::JobsRead => Role::Viewer,
    }
}

/// Checks the role of a request body against the actor.
///
/// The function applies the rules:
///
/// - The role `platform_admin` is never a membership and never an API token role in the MVP. The
///   Platform Admin is an account right (ADR-0029). The roles Admin, Operator and Viewer apply in
///   one tenant.
/// - An actor cannot grant a role above its own. A token cannot create a token with a higher role
///   than its own, and a user cannot either.
pub fn allowed_role(context: &RequestContext, role: Role) -> Result<(), ApiError> {
    if role == Role::PlatformAdmin {
        return Err(ApiError::forbidden());
    }
    if !context.role.allows(role) {
        return Err(ApiError::forbidden());
    }
    Ok(())
}

/// Refuses an operation on a target whose role rank is above the actor's.
///
/// An actor cannot change, disable, delete or reset the password of a user with a higher role.
pub fn ensure_target_not_above_actor(
    context: &RequestContext,
    target_role: Role,
) -> Result<(), ApiError> {
    if !context.role.allows(target_role) {
        return Err(ApiError::forbidden());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Public,
    Protected(Operation),
}

impl Access {
    /// True for a route under `/api/v1/platform/*` (ADR-0024).
    ///
    /// The router hides these routes in single mode. The rule reads the operation of the route
    /// table, so the two agree.
    pub fn is_platform(self) -> bool {
        matches!(self, Self::Protected(operation) if operation.is_platform())
    }
}

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub tenant_id: TenantId,
    pub actor: Actor,
    pub role: Role,
    /// True when the account holds the installation-wide right (ADR-0029). It is the authority for
    /// a platform operation, independent of the session tenant.
    pub platform_admin: bool,
    /// Absent for an API token.
    pub session_id: Option<SessionId>,
    pub source_ip: Option<IpAddr>,
    /// The absolute expiry of the cookie session. Absent for an API token.
    pub session_expires_at: Option<time::OffsetDateTime>,
    /// Present for a cookie only, absent for a token.
    pub session_user_agent: Option<String>,
}

impl RequestContext {
    pub fn new(tenant_id: TenantId, actor: Actor, role: Role) -> Self {
        Self {
            tenant_id,
            actor,
            role,
            platform_admin: false,
            session_id: None,
            source_ip: None,
            session_expires_at: None,
            session_user_agent: None,
        }
    }

    /// Checks the right of the context against the operation.
    ///
    /// A platform operation needs the Platform Admin right (ADR-0029). A tenant-only operation is
    /// refused in the platform tenant.
    pub fn authorize(&self, operation: Operation) -> Result<(), ApiError> {
        if operation.is_platform() {
            return if self.platform_admin {
                Ok(())
            } else {
                Err(ApiError::forbidden())
            };
        }
        // The platform tenant is internal: only the account operations pass there (ADR-0029).
        if self.tenant_id == restichub_db::PLATFORM_TENANT_ID && !operation.is_auth() {
            return Err(ApiError::forbidden());
        }
        if self.role.allows(minimum_role(operation)) {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }

    pub fn audit_event(
        &self,
        action: AuditAction,
        object_type: &str,
        object_id: uuid::Uuid,
    ) -> AuditEvent {
        let mut event = AuditEvent::new(action, object_type);
        event.actor = self.actor.clone();
        event.source_ip = self.source_ip;
        event.object_id = Some(object_id);
        event
    }

    /// Resolves the account behind the actor.
    ///
    /// A user actor is its own account. An API token actor stands for the account of its creator
    /// (ADR-0023).
    pub async fn actor_user(&self, state: &AppState) -> Result<restichub_db::UserId, ApiError> {
        match self.actor {
            Actor::User(user_id) => Ok(user_id),
            Actor::ApiToken(token_id) => {
                let mut tx = state.db().begin_tenant(self.tenant_id).await?;
                let token = api_tokens::get(&mut tx, token_id).await?;
                tx.commit().await?;
                token.map(|token| token.created_by).ok_or_else(|| {
                    ApiError::internal(format!("the token {token_id} is not found after the read"))
                })
            }
            _ => Err(ApiError::forbidden()),
        }
    }

    /// Opens a tenant transaction for the context, runs the closure and commits.
    ///
    /// A failure of the closure rolls the transaction back.
    pub async fn tenant_tx<T>(
        &self,
        state: &AppState,
        f: impl AsyncFnOnce(&mut TenantTx<'_>) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let mut tx = state.db().begin_tenant(self.tenant_id).await?;
        let value = f(&mut tx).await?;
        tx.commit().await?;
        Ok(value)
    }
}

impl FromRequestParts<AppState> for RequestContext {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let name = token::session_cookie_name(state.auth().config.insecure_cookies);
        let cookie = token::session_cookie(&parts.headers, name);
        let bearer = token::bearer_token(&parts.headers);
        match (cookie, bearer) {
            (Some(_), Some(_)) => Err(ApiError::ambiguous_credentials()),
            (Some(value), None) => {
                let mut context = authenticate_session(state, &value).await?;
                if platform_hidden_in_single(state, context.tenant_id) {
                    return Err(ApiError::unauthenticated());
                }
                // A cookie-authenticated unsafe method needs the CSRF header.
                if token::is_unsafe_method(&parts.method) && !token::csrf_ok(&parts.headers) {
                    return Err(ApiError::csrf_required());
                }
                // The source IP is read after the authentication, so a request with no credential
                // is refused with HTTP 401 even when the socket peer is absent.
                context.source_ip = Some(ClientIp::from_request_parts(parts, state).await?.0);
                Ok(context)
            }
            (None, Some(value)) => {
                let mut context = authenticate_bearer(state, &value).await?;
                if platform_hidden_in_single(state, context.tenant_id) {
                    return Err(ApiError::unauthenticated());
                }
                context.source_ip = Some(ClientIp::from_request_parts(parts, state).await?.0);
                Ok(context)
            }
            (None, None) => Err(ApiError::unauthenticated()),
        }
    }
}

/// True for a platform-tenant credential in single mode (ADR-0024).
///
/// Single mode has no platform tenant in the user interface and in the API, so a session or an
/// API token of the platform tenant cannot authenticate. The check is the same 401 as a request
/// with no valid credential, so the mode reveals no platform account.
fn platform_hidden_in_single(state: &AppState, tenant_id: TenantId) -> bool {
    state.tenancy().mode.is_single() && tenant_id == restichub_db::PLATFORM_TENANT_ID
}

async fn authenticate_session(state: &AppState, value: &str) -> Result<RequestContext, ApiError> {
    let hash = token::hash_token(value);
    let lookup = auth::find_session(state.db(), &hash).await?;
    let Some(lookup) = lookup else {
        return Err(ApiError::unauthenticated());
    };
    let now = state.auth().clock.now();
    if lookup.expires_at <= now {
        return Err(ApiError::unauthenticated());
    }
    let idle_deadline = lookup.last_seen_at + state.auth().config.session_idle;
    if idle_deadline <= now {
        return Err(ApiError::unauthenticated());
    }
    let mut tx = state.db().begin_tenant(lookup.tenant_id).await?;
    let tenant = tenants::get(&mut tx, lookup.tenant_id).await?;
    if tenant.is_none_or(|tenant| tenant.status != restichub_db::TenantStatus::Active) {
        return Err(ApiError::unauthenticated());
    }
    // The account holds the Platform Admin right (ADR-0029). A session with no organization has the
    // internal platform tenant as its session tenant and no membership.
    let account = users::find_account_by_id(state.db(), lookup.user_id).await?;
    let Some(account) = account else {
        return Err(ApiError::unauthenticated());
    };
    if account.disabled {
        return Err(ApiError::unauthenticated());
    }
    let role = if lookup.tenant_id == restichub_db::PLATFORM_TENANT_ID {
        // A session with no organization. A Platform Admin reaches the platform screens; another
        // account reaches the account operations only and sees the empty state (ADR-0029).
        if account.platform_admin {
            Role::PlatformAdmin
        } else {
            Role::Viewer
        }
    } else {
        // The account lives in the platform tenant, so the member read goes through the definer
        // function of migration 0017 (ADR-0025).
        let member = users::get_member(&mut tx, lookup.user_id).await?;
        let Some(member) = member else {
            return Err(ApiError::unauthenticated());
        };
        if member.disabled {
            return Err(ApiError::unauthenticated());
        }
        member.role
    };
    sessions::touch(&mut tx, lookup.session_id, now).await?;
    tx.commit().await?;
    Ok(RequestContext {
        tenant_id: lookup.tenant_id,
        actor: Actor::User(lookup.user_id),
        role,
        platform_admin: account.platform_admin,
        session_id: Some(lookup.session_id),
        source_ip: None,
        session_expires_at: Some(lookup.expires_at),
        session_user_agent: lookup.user_agent,
    })
}

async fn authenticate_bearer(state: &AppState, value: &str) -> Result<RequestContext, ApiError> {
    let hash = token::hash_token(value);
    let lookup = auth::find_api_token(state.db(), &hash).await?;
    let Some(lookup) = lookup else {
        return Err(ApiError::unauthenticated());
    };
    let now = state.auth().clock.now();
    if lookup.revoked_at.is_some() {
        return Err(ApiError::unauthenticated());
    }
    if lookup.expires_at.is_some_and(|expires| expires <= now) {
        return Err(ApiError::unauthenticated());
    }
    // The MVP holds no platform-scope API token (ADR-0029). A token of the internal platform tenant
    // never authenticates, and no token carries the Platform Admin right.
    if lookup.tenant_id == restichub_db::PLATFORM_TENANT_ID {
        return Err(ApiError::unauthenticated());
    }
    let mut tx = state.db().begin_tenant(lookup.tenant_id).await?;
    let tenant = tenants::get(&mut tx, lookup.tenant_id).await?;
    if tenant.is_none_or(|tenant| tenant.status != restichub_db::TenantStatus::Active) {
        return Err(ApiError::unauthenticated());
    }
    let role = lookup.role;
    api_tokens::touch(&mut tx, lookup.token_id, now).await?;
    tx.commit().await?;
    Ok(RequestContext {
        tenant_id: lookup.tenant_id,
        actor: Actor::ApiToken(lookup.token_id),
        role,
        platform_admin: false,
        session_id: None,
        source_ip: None,
        session_expires_at: None,
        session_user_agent: None,
    })
}

#[cfg(test)]
mod tests {
    use restichub_db::Role;
    use restichub_db::TenantId;
    use restichub_db::audit::Actor;
    use uuid::Uuid;

    use super::{Operation, OperationClass, RequestContext, ensure_target_not_above_actor};

    fn context(role: Role) -> RequestContext {
        RequestContext::new(TenantId::from_uuid(Uuid::now_v7()), Actor::System, role)
    }

    fn platform_context(role: Role, platform_admin: bool) -> RequestContext {
        let mut context =
            RequestContext::new(restichub_db::PLATFORM_TENANT_ID, Actor::System, role);
        context.platform_admin = platform_admin;
        context
    }

    #[test]
    fn an_actor_cannot_touch_a_higher_role() {
        assert!(ensure_target_not_above_actor(&context(Role::Admin), Role::Admin).is_ok());
        assert!(ensure_target_not_above_actor(&context(Role::Admin), Role::Viewer).is_ok());
        assert!(ensure_target_not_above_actor(&context(Role::Admin), Role::PlatformAdmin).is_err());
        assert!(ensure_target_not_above_actor(&context(Role::Viewer), Role::Operator).is_err());
    }

    /// The platform tenant is an allow-list: only the account operations and the platform
    /// operations pass. `Operation::classify` lists every variant, so a new variant stops the build.
    #[test]
    fn the_platform_tenant_allow_list() {
        for operation in Operation::ALL {
            let class = operation.classify();
            assert_eq!(operation.is_auth(), class == OperationClass::Auth);

            // A Platform Admin reaches the platform operations and the account operations.
            let allowed = class != OperationClass::Tenant;
            assert_eq!(
                platform_context(Role::PlatformAdmin, true)
                    .authorize(operation)
                    .is_ok(),
                allowed,
                "{operation:?}"
            );
            // Without the right only the account operations pass.
            assert_eq!(
                platform_context(Role::Viewer, false)
                    .authorize(operation)
                    .is_ok(),
                class == OperationClass::Auth,
                "{operation:?}"
            );
        }
    }
}
