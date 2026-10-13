//! The sign-in, sign-out, self and password endpoints (ADR-0023, section Local authentication).

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use restichub_db::audit::{Actor, AuditAction, AuditEvent, diff};
use restichub_db::{
    PLATFORM_TENANT_ID, Role, TenantId, TenantStatus, UserId, auth, memberships, sessions, tenants,
    users,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::client_ip::ClientIp;
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

/// The largest length of the tenant name and the username of a sign-in request.
const MAX_SIGN_IN_FIELD: usize = 128;

/// The body of the sign-in request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SignInRequest {
    /// The name of the tenant. The comparison ignores the letter case.
    pub tenant: String,
    /// The username.
    pub username: String,
    /// The password.
    pub password: String,
}

/// The body of the sign-in response.
#[derive(Debug, Serialize, ToSchema)]
pub struct SignInResponse {
    /// The user.
    pub user: UserView,
    /// The tenant.
    pub tenant: TenantView,
    /// The role of the user in the tenant.
    pub role: Role,
}

/// The body of the self password change request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    /// The current password.
    pub current_password: String,
    /// The new password. It must hold at least 12 characters.
    pub new_password: String,
}

/// The body of the self response.
#[derive(Debug, Serialize, ToSchema)]
pub struct MeResponse {
    /// The user. It is null for an API token.
    pub user: Option<UserView>,
    /// The tenant.
    pub tenant: TenantView,
    /// The role of the actor in the tenant.
    pub role: Role,
}

/// Signs a user in.
///
/// The endpoint returns the same error for an unknown tenant, an unknown user, a wrong password, a
/// disabled user and a suspended tenant. The control plane logs the real reason.
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
    // The length checks follow the IP bucket and give the generic error.
    if body.tenant.len() > MAX_SIGN_IN_FIELD || body.username.len() > MAX_SIGN_IN_FIELD {
        return Err(ApiError::invalid_credentials());
    }

    let now = state.auth().clock.now();
    let lookup = auth::find_tenant(state.db(), &body.tenant).await?;
    let active_id = lookup
        .as_ref()
        .filter(|tenant| tenant.status == TenantStatus::Active)
        .map(|tenant| tenant.id);

    match active_id {
        Some(tenant_id) => sign_in_tenant(&state, tenant_id, &body, client.0, now, &headers).await,
        None => {
            // An unknown tenant and a suspended tenant are the same failure. The dummy
            // verification keeps the time similar.
            check_synthetic(&state, &body.tenant, &body.username)?;
            let _ = password::verify_or_dummy(
                &state.auth().hash_semaphore,
                body.password.clone(),
                None,
            )
            .await?;
            audit_failed_platform(&state, client.0).await?;
            tracing::info!("sign-in failed: the tenant is unknown or not active");
            Err(ApiError::invalid_credentials())
        }
    }
}

/// Signs a user in against a known, active tenant.
///
/// The function reads the user in one transaction, then hashes without a transaction, then opens a
/// second transaction for the session and the audit record. A hash never holds a database
/// connection.
async fn sign_in_tenant(
    state: &AppState,
    tenant_id: TenantId,
    body: &SignInRequest,
    client: std::net::IpAddr,
    now: time::OffsetDateTime,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    // Read the user and the role.
    let (user, role) = {
        let mut tx = state.db().begin_tenant(tenant_id).await?;
        let user = users::find_for_auth(&mut tx, &body.username).await?;
        let role = match &user {
            Some(user) => memberships::role_of(&mut tx, user.id).await?,
            None => None,
        };
        tx.commit().await?;
        (user, role)
    };

    // The user bucket uses the resolved identity when the user is known and the normalized input
    // otherwise.
    match &user {
        Some(user) => check_resolved(state, tenant_id, user.id)?,
        None => check_synthetic(state, &body.tenant, &body.username)?,
    }

    // Verify without a transaction.
    let hash = user.as_ref().map(|user| user.password_hash.clone());
    let password_ok =
        password::verify_or_dummy(&state.auth().hash_semaphore, body.password.clone(), hash)
            .await?;
    let user_ok = user.as_ref().is_some_and(|user| !user.disabled) && role.is_some();

    if !password_ok || !user_ok {
        let mut tx = state.db().begin_tenant(tenant_id).await?;
        audit_failed_in_tenant(&mut tx, user.as_ref(), client).await?;
        tx.commit().await?;
        tracing::info!("sign-in failed: the credentials are not valid");
        return Err(ApiError::invalid_credentials());
    }

    let (Some(user), Some(role)) = (user, role) else {
        return Err(ApiError::invalid_credentials());
    };

    // A new session token. The endpoint never reuses an incoming cookie value.
    let value = token::random_token()?;
    let token_hash = token::hash_token(&value);
    let expires_at = now + state.auth().config.session_absolute;
    let user_agent = user_agent(headers);

    // Write the session and the audit record in a second transaction.
    let mut tx = state.db().begin_tenant(tenant_id).await?;
    sessions::create(
        &mut tx,
        user.id,
        &token_hash,
        expires_at,
        now,
        Some(client),
        user_agent.as_deref(),
    )
    .await?;

    let mut event = AuditEvent::new(AuditAction::SignIn, "user");
    event.actor = Actor::User(user.id);
    event.source_ip = Some(client);
    event.object_id = Some(user.id.as_uuid());
    restichub_db::audit::insert(&mut tx, &event).await?;

    let full_user = users::get(&mut tx, user.id).await?;
    let tenant = tenants::get(&mut tx, tenant_id).await?;
    tx.commit().await?;

    let (Some(full_user), Some(tenant)) = (full_user, tenant) else {
        return Err(ApiError::invalid_credentials());
    };
    let insecure = state.auth().config.insecure_cookies;
    let name = token::session_cookie_name(insecure);
    let cookie = token::set_cookie_header(name, &value, !insecure);
    let response = SignInResponse {
        user: UserView::new(&full_user, role),
        tenant: TenantView::from(&tenant),
        role,
    };
    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(response),
    )
        .into_response())
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
    let user = match context.actor {
        Actor::User(user_id) => users::get(&mut tx, user_id).await?,
        _ => None,
    };
    tx.commit().await?;
    let Some(tenant) = tenant else {
        return Err(ApiError::not_found());
    };
    Ok(Json(MeResponse {
        user: user.map(|user| UserView::new(&user, context.role)),
        tenant: TenantView::from(&tenant),
        role: context.role,
    }))
}

/// Changes the password of the current user.
///
/// The endpoint removes every other session of the user and keeps the current session. A wrong
/// current password counts against the user rate limit. The hash runs without a database
/// transaction.
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
    validation::validate_password("new_password", &body.new_password)?;
    // A wrong current password counts against the user bucket.
    check_resolved(&state, context.tenant_id, user_id)?;

    // Read the current hash.
    let user = {
        let mut tx = state.db().begin_tenant(context.tenant_id).await?;
        let user = users::get_auth(&mut tx, user_id).await?;
        tx.commit().await?;
        user
    };
    let Some(user) = user else {
        return Err(ApiError::not_found());
    };

    // Verify and hash without a transaction.
    let current_ok = password::verify_password_limited(
        &state.auth().hash_semaphore,
        body.current_password.clone(),
        user.password_hash.clone(),
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

    // Write the new hash and remove the other sessions.
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    users::set_password(&mut tx, user_id, &hash).await?;
    if let Some(session_id) = context.session_id {
        sessions::delete_for_user_except(&mut tx, user_id, session_id).await?;
    } else {
        sessions::delete_for_user(&mut tx, user_id).await?;
    }
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

/// Applies the IP rate limit.
fn check_ip(state: &AppState, ip: std::net::IpAddr) -> Result<(), ApiError> {
    match state
        .auth()
        .ip_limiter
        .check(&rate_limit::ip_key(ip), Instant::now())
    {
        Ok(()) => Ok(()),
        Err(wait) => Err(ApiError::rate_limited(wait.as_secs().max(1))),
    }
}

/// Applies the synthetic user rate limit.
fn check_synthetic(state: &AppState, tenant: &str, username: &str) -> Result<(), ApiError> {
    let key = rate_limit::synthetic_user_key(tenant, username);
    match state.auth().user_limiter.check(&key, Instant::now()) {
        Ok(()) => Ok(()),
        Err(wait) => Err(ApiError::rate_limited(wait.as_secs().max(1))),
    }
}

/// Applies the resolved user rate limit.
fn check_resolved(state: &AppState, tenant: TenantId, user: UserId) -> Result<(), ApiError> {
    let key = rate_limit::resolved_user_key(tenant, user);
    match state.auth().resolved_limiter.check(&key, Instant::now()) {
        Ok(()) => Ok(()),
        Err(wait) => Err(ApiError::rate_limited(wait.as_secs().max(1))),
    }
}

/// Writes a `sign_in_failed` record in the tenant transaction.
///
/// The record holds the user id of a known user and the marker `user_known: false` for an unknown
/// user. It never holds the attempted username, so a password typed into the username field cannot
/// reach the append-only log.
async fn audit_failed_in_tenant(
    tx: &mut restichub_db::TenantTx<'_>,
    user: Option<&users::AuthUser>,
    ip: std::net::IpAddr,
) -> Result<(), ApiError> {
    let mut event = AuditEvent::new(AuditAction::SignInFailed, "user");
    event.actor = Actor::System;
    event.source_ip = Some(ip);
    match user {
        Some(user) => event.object_id = Some(user.id.as_uuid()),
        None => event.diff = Some(diff(None, Some(&unknown_user_diff()), &[])),
    }
    restichub_db::audit::insert(tx, &event).await?;
    Ok(())
}

/// Writes a `sign_in_failed` record in the platform tenant.
async fn audit_failed_platform(state: &AppState, ip: std::net::IpAddr) -> Result<(), ApiError> {
    let mut tx = state.db().begin_platform().await?;
    let mut event = AuditEvent::new(AuditAction::SignInFailed, "user");
    event.actor = Actor::System;
    event.source_ip = Some(ip);
    event.diff = Some(diff(None, Some(&unknown_user_diff()), &[]));
    restichub_db::audit::insert_platform(&mut tx, PLATFORM_TENANT_ID, &event).await?;
    tx.commit().await?;
    Ok(())
}

/// Returns the diff of a failed sign-in against an unknown user. It holds no username.
fn unknown_user_diff() -> serde_json::Value {
    serde_json::json!({ "user_known": false })
}

/// Returns the `User-Agent` header value.
fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}
