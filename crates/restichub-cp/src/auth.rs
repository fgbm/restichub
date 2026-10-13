//! Local authentication endpoints (ADR-0025).
//!
//! An account is installation-wide. Sign-in asks for the username and the password only. The
//! control plane finds the account through the definer function `auth_find_account`, verifies the
//! password (a dummy hash covers an absent account), then lists the active memberships of the
//! account in active tenants and picks the session tenant.

use std::net::IpAddr;
use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use restichub_db::audit::{Actor, AuditAction, AuditEvent, diff};
use restichub_db::{
    PLATFORM_TENANT_ID, Role, SessionId, TenantId, UserId, auth, sessions, tenants, users,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::client_ip::ClientIp;
use crate::config::TenancyMode;
use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::password;
use crate::rate_limit;
use crate::server::AppState;
use crate::token;
use crate::validation;
use crate::validation::Reason;
use crate::views::{TenantView, UserView};

/// Limit of the username.
const MAX_SIGN_IN_FIELD: usize = 128;

/// The sign-in body. It holds the username and the password only. An unknown field, for example
/// `tenant`, is ignored (ADR-0025).
#[derive(Debug, Deserialize, ToSchema)]
pub struct SignInRequest {
    /// Unique in the installation; ignores the letter case.
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SignInResponse {
    pub user: UserView,
    /// Null for a session with no organization (ADR-0029).
    pub tenant: Option<TenantView>,
    pub role: Role,
    /// True when the account holds the installation-wide right (ADR-0029).
    pub platform_admin: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    /// At least 12 characters.
    pub new_password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MeResponse {
    /// Null for an API token.
    pub user: Option<UserView>,
    /// Null for a session with no organization (ADR-0029).
    pub tenant: Option<TenantView>,
    pub role: Role,
    /// True when the account holds the installation-wide right (ADR-0029).
    pub platform_admin: bool,
}

/// The memberships of the caller. Each item holds the tenant id, the tenant name and the role.
#[derive(Debug, Serialize, ToSchema)]
pub struct TenantAccessView {
    pub id: String,
    pub name: String,
    pub role: Role,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TenantAccessListResponse {
    pub items: Vec<TenantAccessView>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SwitchTenantRequest {
    pub tenant_id: String,
}

/// Signs a user in with the username and the password only.
///
/// The endpoint returns the same error for an unknown account, a wrong password, a disabled
/// account and an account with no active membership. An absent account is verified against a dummy
/// hash, so the answer and the time are the same.
#[utoipa::path(
    post,
    path = "/api/v1/auth/sign-in",
    request_body = SignInRequest,
    responses(
        (status = 200, description = "The session cookie is set.", body = SignInResponse),
        (status = 400, description = "The request body is not valid."),
        (status = 401, description = "The credentials are not valid.", body = crate::error::ProblemBody),
        (status = 403, description = "The CSRF header is missing.", body = crate::error::ProblemBody),
        (status = 429, description = "Too many sign-in attempts.", body = crate::error::ProblemBody),
        (status = 503, description = "The password work is busy.", body = crate::error::ProblemBody)
    )
)]
pub async fn sign_in(
    State(state): State<AppState>,
    client: ClientIp,
    headers: HeaderMap,
    ApiJson(body): ApiJson<SignInRequest>,
) -> Result<Response, ApiError> {
    // Login CSRF: sign-in needs the header too.
    if !token::csrf_ok(&headers) {
        return Err(ApiError::csrf_required());
    }
    // The IP bucket is checked first, before the user map is touched.
    check_ip(&state, client.0)?;
    // The length check follows the IP bucket and gives the generic error.
    if body.username.len() > MAX_SIGN_IN_FIELD {
        return Err(ApiError::invalid_credentials());
    }

    let now = state.auth().clock.now();
    let account = users::find_account(state.db(), &body.username).await?;

    // The user bucket uses the resolved account id when the account is known and the normalized
    // username otherwise. The sign-in has no tenant, so the bucket is installation-wide.
    match &account {
        Some(account) => check_resolved(&state, account.id)?,
        None => check_synthetic(&state, &body.username)?,
    }

    // The hash runs without a database transaction. An absent account verifies a dummy hash.
    let hash = account
        .as_ref()
        .map(|account| account.password_hash.clone());
    let password_ok =
        password::verify_or_dummy(&state.auth().hash_semaphore, body.password.clone(), hash)
            .await?;

    let Some(account) = account else {
        audit_failed_platform(&state, client.0).await?;
        tracing::info!("sign-in failed: the account is unknown");
        return Err(ApiError::invalid_credentials());
    };
    if !password_ok || account.disabled {
        audit_failed_platform(&state, client.0).await?;
        tracing::info!("sign-in failed: the credentials are not valid");
        return Err(ApiError::invalid_credentials());
    }

    // The active memberships of the account in active tenants.
    let memberships = auth::list_memberships(state.db(), account.id).await?;
    let memberships = visible_memberships(&state, memberships);
    // An account with no active membership still signs in (ADR-0029). A Platform Admin reaches the
    // platform screens; another account sees the empty state. The session tenant is the internal
    // platform tenant, which the control plane never shows.
    let (tenant_id, role) = match choose_tenant(&memberships, account.last_used_tenant_id) {
        Some(chosen) => (chosen.tenant_id, chosen.role),
        None => (
            PLATFORM_TENANT_ID,
            if account.platform_admin {
                Role::PlatformAdmin
            } else {
                Role::Viewer
            },
        ),
    };

    // The endpoint never reuses an incoming cookie value.
    let value = token::random_token()?;
    let token_hash = token::hash_token(&value);
    let expires_at = now + state.auth().config.session_absolute;
    let user_agent = user_agent(&headers);

    let mut tx = state.db().begin_tenant(tenant_id).await?;
    let session_id = sessions::create(
        &mut tx,
        account.id,
        &token_hash,
        expires_at,
        now,
        Some(client.0),
        user_agent.as_deref(),
    )
    .await?;
    if tenant_id != PLATFORM_TENANT_ID {
        users::set_last_tenant(&mut tx, account.id, session_id, tenant_id).await?;
    }

    let mut event = AuditEvent::new(AuditAction::SignIn, "user");
    event.actor = Actor::User(account.id);
    event.source_ip = Some(client.0);
    event.object_id = Some(account.id.as_uuid());
    restichub_db::audit::insert(&mut tx, &event).await?;

    // The membership and the tenant are read before the commit, so a lost membership leaves no
    // session behind.
    let member = users::get_member(&mut tx, account.id).await?;
    let tenant = tenants::get(&mut tx, tenant_id).await?;
    let Some(tenant) = tenant else {
        return Err(ApiError::invalid_credentials());
    };
    tx.commit().await?;

    let insecure = state.auth().config.insecure_cookies;
    let name = token::session_cookie_name(insecure);
    let cookie = token::set_cookie_header(name, &value, !insecure);
    let user = match &member {
        Some(member) => UserView::from_member(member),
        None => {
            let full = users::find_account_by_id(state.db(), account.id)
                .await?
                .ok_or_else(|| ApiError::internal("the signed-in account is not found"))?;
            UserView::from_account(&full, role)
        }
    };
    let tenant_view = if tenant_id == PLATFORM_TENANT_ID {
        None
    } else {
        Some(TenantView::from(&tenant))
    };
    let response = SignInResponse {
        user,
        tenant: tenant_view,
        role,
        platform_admin: account.platform_admin,
    };
    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(response),
    )
        .into_response())
}

/// Lists the memberships of the caller only (ADR-0025).
///
/// The platform tenant is included for a Platform Admin in multi mode. In single mode the platform
/// tenant is hidden (ADR-0024).
#[utoipa::path(
    get,
    path = "/api/v1/auth/tenants",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The memberships of the caller.", body = TenantAccessListResponse),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody)
    )
)]
pub async fn list_tenants(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<TenantAccessListResponse>, ApiError> {
    context.authorize(Operation::AuthTenants)?;
    let items = match context.actor {
        Actor::User(user_id) => {
            let memberships = auth::list_memberships(state.db(), user_id).await?;
            visible_memberships(&state, memberships)
                .into_iter()
                .map(|membership| TenantAccessView {
                    id: membership.tenant_id.to_string(),
                    name: membership.tenant_name,
                    role: membership.role,
                })
                .collect()
        }
        // A platform session and a platform API token hold no organization (ADR-0029).
        _ if context.tenant_id == PLATFORM_TENANT_ID => vec![],
        // API tokens carry one tenant and one role.
        _ => vec![TenantAccessView {
            id: context.tenant_id.to_string(),
            name: tenant_name(&state, context.tenant_id).await,
            role: context.role,
        }],
    };
    Ok(Json(TenantAccessListResponse { items }))
}

async fn tenant_name(state: &AppState, tenant_id: TenantId) -> String {
    let Ok(mut tx) = state.db().begin_tenant(tenant_id).await else {
        return String::new();
    };
    let name = tenants::get(&mut tx, tenant_id)
        .await
        .ok()
        .flatten()
        .map(|tenant| tenant.name)
        .unwrap_or_default();
    let _ = tx.commit().await;
    name
}

/// Switches the session to another tenant (ADR-0025).
///
/// The endpoint checks the membership of the caller in the target tenant, deletes the source
/// session, creates a new session in the target tenant with the source expiry and the source user
/// agent, and rotates the cookie. A source session that another request already removed gives HTTP
/// 401. Switching to the current tenant gives HTTP 400.
#[utoipa::path(
    post,
    path = "/api/v1/auth/switch",
    security(("cookie" = []), ("bearer" = [])),
    request_body = SwitchTenantRequest,
    responses(
        (status = 200, description = "The session moved to the target tenant.", body = SignInResponse),
        (status = 400, description = "The target tenant is the current tenant.", body = crate::error::ProblemBody),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The caller is not a member of the target tenant.", body = crate::error::ProblemBody)
    )
)]
pub async fn switch_tenant(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<SwitchTenantRequest>,
) -> Result<Response, ApiError> {
    context.authorize(Operation::AuthSwitch)?;
    let Actor::User(user_id) = context.actor else {
        return Err(ApiError::forbidden());
    };
    let Some(session_id) = context.session_id else {
        return Err(ApiError::forbidden());
    };
    let tenant_id = parse_tenant_id(&body.tenant_id)?;
    if tenant_id == context.tenant_id {
        return Err(ApiError::invalid_argument("tenant_id"));
    }

    // The membership check. A suspended or deleted tenant and a disabled membership are refused.
    let memberships = auth::list_memberships(state.db(), user_id).await?;
    let memberships = visible_memberships(&state, memberships);
    let Some(target) = memberships
        .iter()
        .find(|membership| membership.tenant_id == tenant_id)
    else {
        return Err(ApiError::forbidden());
    };
    let role = target.role;

    let now = state.auth().clock.now();
    let value = token::random_token()?;
    let token_hash = token::hash_token(&value);
    // The new session keeps the absolute expiry and the user agent of the source session.
    let expires_at = context
        .session_expires_at
        .unwrap_or_else(|| now + state.auth().config.session_absolute);
    let user_agent = context.session_user_agent.clone();
    let source_tenant = context.tenant_id;

    let mut tx = state.db().begin_tenant(tenant_id).await?;
    // The source session is deleted in the same transaction, so two parallel switches leave one
    // session. A missing row means another request already moved the session.
    let deleted = auth::delete_session(&mut tx, session_id, user_id).await?;
    if deleted == 0 {
        return Err(ApiError::unauthenticated());
    }
    let new_session_id = sessions::create(
        &mut tx,
        user_id,
        &token_hash,
        expires_at,
        now,
        context.source_ip,
        user_agent.as_deref(),
    )
    .await?;
    users::set_last_tenant(&mut tx, user_id, new_session_id, tenant_id).await?;
    let mut event =
        context.audit_event(AuditAction::SessionSwitch, "session", session_id.as_uuid());
    // The target-tenant record names the tenant the session moved from.
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({"source_tenant_id": source_tenant.to_string()})),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    let member = users::get_member(&mut tx, user_id).await?;
    let tenant = tenants::get(&mut tx, tenant_id).await?;
    let (Some(member), Some(tenant)) = (member, tenant) else {
        return Err(ApiError::forbidden());
    };
    tx.commit().await?;

    // Record the switch in the source tenant too. The audit of another tenant cannot join the target
    // transaction under the tenant policy, so a failure here must not deny the caller the new
    // cookie: the switch is already committed.
    if let Err(error) = write_switch_audit(&state, source_tenant, &context, session_id).await {
        tracing::warn!(
            source_tenant = %source_tenant,
            session_id = %session_id,
            ?error,
            "the source-tenant switch audit failed"
        );
    }

    let insecure = state.auth().config.insecure_cookies;
    let name = token::session_cookie_name(insecure);
    let cookie = token::set_cookie_header(name, &value, !insecure);
    let response = SignInResponse {
        user: UserView::from_member(&member),
        tenant: Some(TenantView::from(&tenant)),
        role,
        platform_admin: context.platform_admin,
    };
    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(response),
    )
        .into_response())
}

/// Records a switch in one tenant. The caller treats a failure as a warning only.
async fn write_switch_audit(
    state: &AppState,
    tenant_id: TenantId,
    context: &RequestContext,
    session_id: SessionId,
) -> Result<(), ApiError> {
    let mut tx = state.db().begin_tenant(tenant_id).await?;
    let event = context.audit_event(AuditAction::SessionSwitch, "session", session_id.as_uuid());
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(())
}

/// Signs the current session out.
#[utoipa::path(
    post,
    path = "/api/v1/auth/sign-out",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 204, description = "The session is removed."),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The CSRF header is missing.", body = crate::error::ProblemBody)
    )
)]
pub async fn sign_out(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Response, ApiError> {
    context.authorize(Operation::AuthSignOut)?;
    if let Some(session_id) = context.session_id {
        let mut tx = state.db().begin_tenant(context.tenant_id).await?;
        sessions::delete(&mut tx, session_id).await?;
        let event = context.audit_event(AuditAction::SignOut, "session", session_id.as_uuid());
        restichub_db::audit::insert(&mut tx, &event).await?;
        tx.commit().await?;
    }
    let insecure = state.auth().config.insecure_cookies;
    let name = token::session_cookie_name(insecure);
    let cookie = token::clear_cookie_header(name, !insecure);
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response())
}

/// Returns the user, the tenant and the role of the current actor.
#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The current actor.", body = MeResponse),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody)
    )
)]
pub async fn me(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<MeResponse>, ApiError> {
    context.authorize(Operation::AuthMe)?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let tenant = tenants::get(&mut tx, context.tenant_id).await?;
    let member = match context.actor {
        Actor::User(user_id) => users::get_member(&mut tx, user_id).await?,
        _ => None,
    };
    tx.commit().await?;
    // A session with no organization has no membership; the account gives the user view (ADR-0029).
    let account = match context.actor {
        Actor::User(user_id) => users::find_account_by_id(state.db(), user_id).await?,
        _ => None,
    };
    let user = match (&member, &account) {
        (Some(member), _) => Some(UserView::from_member(member)),
        (None, Some(account)) => Some(UserView::from_account(account, context.role)),
        _ => None,
    };
    let tenant_view = if context.tenant_id == PLATFORM_TENANT_ID {
        None
    } else {
        tenant.map(|tenant| TenantView::from(&tenant))
    };
    Ok(Json(MeResponse {
        user,
        tenant: tenant_view,
        role: context.role,
        platform_admin: context.platform_admin,
    }))
}

/// Changes the password of the current user (installation-wide).
///
/// The endpoint sets the account password and removes every other session of the account in every
/// tenant, and it keeps the current session. The database function binds the write to the caller's
/// own session. A wrong current password counts against the user rate limit. The hash runs without
/// a database transaction.
#[utoipa::path(
    post,
    path = "/api/v1/auth/password",
    security(("cookie" = []), ("bearer" = [])),
    request_body = ChangePasswordRequest,
    responses(
        (status = 204, description = "The password is changed."),
        (status = 400, description = "The new password is not valid.", body = crate::error::ProblemBody),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The actor is an API token or the CSRF header is missing.", body = crate::error::ProblemBody),
        (status = 429, description = "Too many attempts.", body = crate::error::ProblemBody),
        (status = 503, description = "The password work is busy.", body = crate::error::ProblemBody)
    )
)]
pub async fn change_password(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<ChangePasswordRequest>,
) -> Result<Response, ApiError> {
    context.authorize(Operation::AuthPasswordChange)?;
    let Actor::User(user_id) = context.actor else {
        return Err(ApiError::forbidden());
    };
    let Some(session_id) = context.session_id else {
        return Err(ApiError::forbidden());
    };
    validation::validate_password("new_password", &body.new_password)?;
    // A wrong current password counts against the user bucket.
    check_resolved(&state, user_id)?;

    // The hash comes from the caller's own account, bound to the caller's own session.
    let current_hash = {
        let mut tx = state.db().begin_tenant(context.tenant_id).await?;
        let hash = users::self_password_hash(&mut tx, user_id, session_id).await?;
        tx.commit().await?;
        hash
    };
    let Some(current_hash) = current_hash else {
        return Err(ApiError::unauthenticated());
    };

    let current_ok = password::verify_password_limited(
        &state.auth().hash_semaphore,
        body.current_password.clone(),
        current_hash,
    )
    .await?;
    if !current_ok {
        return Err(ApiError::invalid_field(
            "current_password",
            Reason::InvalidCredentials,
        ));
    }
    let hash =
        password::hash_password_limited(&state.auth().hash_semaphore, body.new_password.clone())
            .await?;

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let written = users::self_set_password(&mut tx, user_id, session_id, &hash).await?;
    if !written {
        return Err(ApiError::unauthenticated());
    }
    // The password change is installation-wide: it ends the sessions in every tenant and keeps the
    // current one (ADR-0025).
    auth::delete_account_sessions(&mut tx, user_id, Some(session_id)).await?;
    let mut event = context.audit_event(AuditAction::UserUpdate, "user", user_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({"password": "<changed>"})),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Filters the memberships that the current mode can show.
///
/// Single mode hides the platform tenant (ADR-0024).
fn visible_memberships(
    state: &AppState,
    memberships: Vec<auth::MembershipLookup>,
) -> Vec<auth::MembershipLookup> {
    if state.tenancy().mode == TenancyMode::Single {
        memberships
            .into_iter()
            .filter(|membership| membership.tenant_id != PLATFORM_TENANT_ID)
            .collect()
    } else {
        memberships
    }
}

/// Picks the session tenant (ADR-0025).
///
/// The last used tenant wins when it is in the list. Otherwise the first membership by tenant name
/// gives the session; the list is ordered by the tenant name.
fn choose_tenant(
    memberships: &[auth::MembershipLookup],
    last_used: Option<TenantId>,
) -> Option<&auth::MembershipLookup> {
    if let Some(last_used) = last_used
        && let Some(membership) = memberships
            .iter()
            .find(|membership| membership.tenant_id == last_used)
    {
        return Some(membership);
    }
    memberships.first()
}

fn check_ip(state: &AppState, ip: IpAddr) -> Result<(), ApiError> {
    match state
        .auth()
        .ip_limiter
        .check(&rate_limit::ip_key(ip), Instant::now())
    {
        Ok(()) => Ok(()),
        Err(wait) => Err(ApiError::rate_limited(wait.as_secs().max(1))),
    }
}

fn check_synthetic(state: &AppState, username: &str) -> Result<(), ApiError> {
    let key = rate_limit::synthetic_user_key(username);
    match state.auth().user_limiter.check(&key, Instant::now()) {
        Ok(()) => Ok(()),
        Err(wait) => Err(ApiError::rate_limited(wait.as_secs().max(1))),
    }
}

fn check_resolved(state: &AppState, user: UserId) -> Result<(), ApiError> {
    let key = rate_limit::resolved_user_key(user);
    match state.auth().resolved_limiter.check(&key, Instant::now()) {
        Ok(()) => Ok(()),
        Err(wait) => Err(ApiError::rate_limited(wait.as_secs().max(1))),
    }
}

/// Writes a `sign_in_failed` record in the platform tenant.
///
/// The record holds the marker `user_known: false` only. It never holds the attempted username, so
/// a password typed into the username field cannot reach the append-only log.
async fn audit_failed_platform(state: &AppState, ip: IpAddr) -> Result<(), ApiError> {
    let mut tx = state.db().begin_platform().await?;
    let mut event = AuditEvent::new(AuditAction::SignInFailed, "user");
    event.actor = Actor::System;
    event.source_ip = Some(ip);
    event.diff = Some(diff(None, Some(&unknown_user_diff()), &[]));
    restichub_db::audit::insert_platform(&mut tx, PLATFORM_TENANT_ID, &event).await?;
    tx.commit().await?;
    Ok(())
}

/// Holds no username.
fn unknown_user_diff() -> serde_json::Value {
    serde_json::json!({ "user_known": false })
}

fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn parse_tenant_id(value: &str) -> Result<TenantId, ApiError> {
    uuid::Uuid::parse_str(value)
        .map(TenantId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause(
                "tenant_id",
                validation::Reason::InvalidFormat,
                error.to_string(),
            )
        })
}
