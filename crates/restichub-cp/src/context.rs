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
    Role, SessionId, TenantId, TenantTx, api_tokens, auth, memberships, sessions, tenants, users,
};

use crate::client_ip::ClientIp;
use crate::error::ApiError;
use crate::server::AppState;
use crate::token;
use crate::validation::Reason;

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
pub fn allowed_role(context: &RequestContext, value: &str) -> Result<Role, ApiError> {
    let role =
        Role::parse(value).ok_or_else(|| ApiError::invalid_field("role", Reason::Unknown))?;
    let platform = context.tenant_id == restichub_db::PLATFORM_TENANT_ID;
    if platform && role != Role::PlatformAdmin {
        return Err(ApiError::forbidden());
    }
    if !platform && role == Role::PlatformAdmin {
        return Err(ApiError::forbidden());
    }
    if !context.role.allows(role) {
        return Err(ApiError::forbidden());
    }
    Ok(role)
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

/// The access rule of one route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// The route needs no authentication.
    Public,
    /// The route needs the minimum role of the operation.
    Protected(Operation),
}

/// The tenant, the actor, the role, the session and the source IP of a request.
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
    /// The source IP address of the request.
    pub source_ip: Option<IpAddr>,
}

impl RequestContext {
    /// Creates a request context with no session and no source IP.
    pub fn new(tenant_id: TenantId, actor: Actor, role: Role) -> Self {
        Self {
            tenant_id,
            actor,
            role,
            session_id: None,
            source_ip: None,
        }
    }

    /// Checks the role of the context against the minimum role of the operation.
    ///
    /// A platform operation needs the platform tenant too. A tenant-only operation is refused in
    /// the platform tenant.
    pub fn authorize(&self, operation: Operation) -> Result<(), ApiError> {
        if operation.is_platform() && self.tenant_id != restichub_db::PLATFORM_TENANT_ID {
            return Err(ApiError::forbidden());
        }
        if operation.is_tenant_only() && self.tenant_id == restichub_db::PLATFORM_TENANT_ID {
            return Err(ApiError::forbidden());
        }
        if self.role.allows(minimum_role(operation)) {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }

    /// Builds an audit event with the actor and the source IP of the request.
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
                context.source_ip = Some(ClientIp::from_request_parts(parts, state).await?.0);
                Ok(context)
            }
            (None, None) => Err(ApiError::unauthenticated()),
        }
    }
}

/// Authenticates a request from a session cookie value.
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
    let user = users::get(&mut tx, lookup.user_id).await?;
    let Some(user) = user else {
        return Err(ApiError::unauthenticated());
    };
    if user.disabled {
        return Err(ApiError::unauthenticated());
    }
    let role = memberships::role_of(&mut tx, lookup.user_id).await?;
    let Some(role) = role else {
        return Err(ApiError::unauthenticated());
    };
    sessions::touch(&mut tx, lookup.session_id, now).await?;
    tx.commit().await?;
    Ok(RequestContext {
        tenant_id: lookup.tenant_id,
        actor: Actor::User(lookup.user_id),
        role,
        session_id: Some(lookup.session_id),
        source_ip: None,
    })
}

/// Authenticates a request from a bearer token value.
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
        session_id: None,
        source_ip: None,
    })
}

#[cfg(test)]
mod tests {
    use restichub_db::Role;
    use restichub_db::TenantId;
    use restichub_db::audit::Actor;
    use uuid::Uuid;

    use super::{RequestContext, ensure_target_not_above_actor};

    /// Builds a request context with a role.
    fn context(role: Role) -> RequestContext {
        RequestContext::new(TenantId::from_uuid(Uuid::now_v7()), Actor::System, role)
    }

    #[test]
    fn an_actor_cannot_touch_a_higher_role() {
        assert!(ensure_target_not_above_actor(&context(Role::Admin), Role::Admin).is_ok());
        assert!(ensure_target_not_above_actor(&context(Role::Admin), Role::Viewer).is_ok());
        assert!(ensure_target_not_above_actor(&context(Role::Admin), Role::PlatformAdmin).is_err());
        assert!(ensure_target_not_above_actor(&context(Role::Viewer), Role::Operator).is_err());
    }
}
