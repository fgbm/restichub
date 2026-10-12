//! The sign-in, sign-out, self and password endpoints (ADR-0023, section Local authentication).

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use restichub_db::audit::{Actor, AuditAction, AuditEvent, diff};
use restichub_db::{
    PLATFORM_TENANT_ID, TenantId, UserId, auth, memberships, sessions, tenants, users,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::client_ip::ClientIp;
use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::password;
use crate::rate_limit;
use crate::server::{AppState, RequestIdHeader};
use crate::token;
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
    pub role: String,
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
    pub role: String,
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
    request_id: RequestIdHeader,
    headers: HeaderMap,
    ApiJson(body): ApiJson<SignInRequest>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    // Login CSRF: sign-in needs the header too.
    if !token::csrf_ok(&headers) {
        return Err(ApiError::csrf_required(&request_id));
    }
    // The IP bucket is checked first, before the user map is touched.
    check_ip(&state, client.0, &request_id)?;
    // The length checks follow the IP bucket and give the generic error.
    if body.tenant.len() > MAX_SIGN_IN_FIELD || body.username.len() > MAX_SIGN_IN_FIELD {
        return Err(ApiError::invalid_credentials(&request_id));
    }

    let now = state.auth().clock.now();
    let lookup = auth::find_tenant(state.db(), &body.tenant)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let active_id = lookup
        .as_ref()
        .filter(|tenant| tenant.status == "active")
        .map(|tenant| tenant.id);

    match active_id {
        Some(tenant_id) => {
            sign_in_tenant(
                &state,
                tenant_id,
                &body,
                client.0,
                now,
                &request_id,
                &headers,
            )
            .await
        }
        None => {
            // An unknown tenant and a suspended tenant are the same failure. The dummy
            // verification keeps the time similar.
            check_synthetic(&state, &body.tenant, &body.username, &request_id)?;
            let _ = password::verify_or_dummy(
                &state.auth().hash_semaphore,
                body.password.clone(),
                None,
            )
            .await
            .map_err(|error| ApiError::from_password(&request_id, &error))?;
            audit_failed_platform(&state, client.0, &request_id).await?;
            tracing::info!(request_id = %request_id, "sign-in failed: the tenant is unknown or not active");
            Err(ApiError::invalid_credentials(&request_id))
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
    request_id: &str,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    // Read the user and the role.
    let (user, role) = {
        let mut tx = state
            .db()
            .begin_tenant(tenant_id)
            .await
            .map_err(|error| ApiError::internal(request_id, &error))?;
        let user = users::find_for_auth(&mut tx, &body.username)
            .await
            .map_err(|error| ApiError::internal(request_id, &error))?;
        let role = match &user {
            Some(user) => memberships::role_of(&mut tx, user.id)
                .await
                .map_err(|error| ApiError::internal(request_id, &error))?,
            None => None,
        };
        tx.commit()
            .await
            .map_err(|error| ApiError::internal(request_id, &error))?;
        (user, role)
    };

    // The user bucket uses the resolved identity when the user is known and the normalized input
    // otherwise.
    match &user {
        Some(user) => check_resolved(state, tenant_id, user.id, request_id)?,
        None => check_synthetic(state, &body.tenant, &body.username, request_id)?,
    }

    // Verify without a transaction.
    let hash = user.as_ref().map(|user| user.password_hash.clone());
    let password_ok =
        password::verify_or_dummy(&state.auth().hash_semaphore, body.password.clone(), hash)
            .await
            .map_err(|error| ApiError::from_password(request_id, &error))?;
    let user_ok = user.as_ref().is_some_and(|user| !user.disabled) && role.is_some();

    if !password_ok || !user_ok {
        let mut tx = state
            .db()
            .begin_tenant(tenant_id)
            .await
            .map_err(|error| ApiError::internal(request_id, &error))?;
        audit_failed_in_tenant(&mut tx, user.as_ref(), client, request_id).await?;
        tx.commit()
            .await
            .map_err(|error| ApiError::internal(request_id, &error))?;
        tracing::info!(request_id = %request_id, "sign-in failed: the credentials are not valid");
        return Err(ApiError::invalid_credentials(request_id));
    }

    let (Some(user), Some(role)) = (user, role) else {
        return Err(ApiError::invalid_credentials(request_id));
    };

    // A new session token. The endpoint never reuses an incoming cookie value.
    let value = token::random_token().map_err(|error| ApiError::internal(request_id, &error))?;
    let token_hash = token::hash_token(&value);
    let expires_at = now + state.auth().config.session_absolute;
    let user_agent = user_agent(headers);

    // Write the session and the audit record in a second transaction.
    let mut tx = state
        .db()
        .begin_tenant(tenant_id)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    sessions::create(
        &mut tx,
        user.id,
        &token_hash,
        expires_at,
        now,
        Some(client),
        user_agent.as_deref(),
    )
    .await
    .map_err(|error| ApiError::internal(request_id, &error))?;

    let mut event = AuditEvent::new(AuditAction::SignIn, "user");
    event.actor = Actor::User(user.id);
    event.source_ip = Some(client);
    event.object_id = Some(user.id.as_uuid());
    restichub_db::audit::insert(&mut tx, &event)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;

    let full_user = users::get(&mut tx, user.id)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    let tenant = tenants::get(&mut tx, tenant_id)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;

    let (Some(full_user), Some(tenant)) = (full_user, tenant) else {
        return Err(ApiError::invalid_credentials(request_id));
    };
    let insecure = state.auth().config.insecure_cookies;
    let name = token::session_cookie_name(insecure);
    let cookie = token::set_cookie_header(name, &value, !insecure);
    let response = SignInResponse {
        user: UserView::new(&full_user, &role),
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
    request_id: RequestIdHeader,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::AuthSignOut, &request_id)?;
    if let Some(session_id) = context.session_id {
        let mut tx = state
            .db()
            .begin_tenant(context.tenant_id)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        sessions::delete(&mut tx, session_id)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        let mut event = AuditEvent::new(AuditAction::SignOut, "session");
        event.actor = context.actor.clone();
        event.object_id = Some(session_id.as_uuid());
        restichub_db::audit::insert(&mut tx, &event)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        tx.commit()
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
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
    request_id: RequestIdHeader,
) -> Result<Json<MeResponse>, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::AuthMe, &request_id)?;
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let tenant = tenants::get(&mut tx, context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let user = match context.actor {
        Actor::User(user_id) => users::get(&mut tx, user_id)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?,
        _ => None,
    };
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let Some(tenant) = tenant else {
        return Err(ApiError::not_found(&request_id));
    };
    Ok(Json(MeResponse {
        user: user.map(|user| UserView::new(&user, context.role.as_str())),
        tenant: TenantView::from(&tenant),
        role: context.role.as_str().to_owned(),
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
    request_id: RequestIdHeader,
    ApiJson(body): ApiJson<ChangePasswordRequest>,
) -> Result<Response, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::AuthPasswordChange, &request_id)?;
    let Actor::User(user_id) = context.actor else {
        return Err(ApiError::forbidden(&request_id));
    };
    password::validate_password(&body.new_password)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;
    // A wrong current password counts against the user bucket.
    check_resolved(&state, context.tenant_id, user_id, &request_id)?;

    // Read the current hash.
    let user = {
        let mut tx = state
            .db()
            .begin_tenant(context.tenant_id)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        let user = users::get_auth(&mut tx, user_id)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        tx.commit()
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        user
    };
    let Some(user) = user else {
        return Err(ApiError::not_found(&request_id));
    };

    // Verify and hash without a transaction.
    let current_ok = password::verify_password_limited(
        &state.auth().hash_semaphore,
        body.current_password.clone(),
        user.password_hash.clone(),
    )
    .await
    .map_err(|error| ApiError::from_password(&request_id, &error))?;
    if !current_ok {
        return Err(ApiError::invalid_argument(&request_id));
    }
    let hash =
        password::hash_password_limited(&state.auth().hash_semaphore, body.new_password.clone())
            .await
            .map_err(|error| ApiError::from_password(&request_id, &error))?;

    // Write the new hash and remove the other sessions.
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    users::set_password(&mut tx, user_id, &hash)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    if let Some(session_id) = context.session_id {
        sessions::delete_for_user_except(&mut tx, user_id, session_id)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
    } else {
        sessions::delete_for_user(&mut tx, user_id)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
    }
    let mut event = AuditEvent::new(AuditAction::UserUpdate, "user");
    event.actor = context.actor.clone();
    event.object_id = Some(user_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({"password": "<changed>"})),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Applies the IP rate limit.
fn check_ip(state: &AppState, ip: std::net::IpAddr, request_id: &str) -> Result<(), ApiError> {
    match state
        .auth()
        .ip_limiter
        .check(&rate_limit::ip_key(ip), Instant::now())
    {
        Ok(()) => Ok(()),
        Err(wait) => Err(ApiError::rate_limited(request_id, wait.as_secs().max(1))),
    }
}

/// Applies the synthetic user rate limit.
fn check_synthetic(
    state: &AppState,
    tenant: &str,
    username: &str,
    request_id: &str,
) -> Result<(), ApiError> {
    let key = rate_limit::synthetic_user_key(tenant, username);
    match state.auth().user_limiter.check(&key, Instant::now()) {
        Ok(()) => Ok(()),
        Err(wait) => Err(ApiError::rate_limited(request_id, wait.as_secs().max(1))),
    }
}

/// Applies the resolved user rate limit.
fn check_resolved(
    state: &AppState,
    tenant: TenantId,
    user: UserId,
    request_id: &str,
) -> Result<(), ApiError> {
    let key = rate_limit::resolved_user_key(tenant, user);
    match state.auth().resolved_limiter.check(&key, Instant::now()) {
        Ok(()) => Ok(()),
        Err(wait) => Err(ApiError::rate_limited(request_id, wait.as_secs().max(1))),
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
    request_id: &str,
) -> Result<(), ApiError> {
    let mut event = AuditEvent::new(AuditAction::SignInFailed, "user");
    event.actor = Actor::System;
    event.source_ip = Some(ip);
    match user {
        Some(user) => event.object_id = Some(user.id.as_uuid()),
        None => event.diff = Some(diff(None, Some(&unknown_user_diff()), &[])),
    }
    restichub_db::audit::insert(tx, &event)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    Ok(())
}

/// Writes a `sign_in_failed` record in the platform tenant.
async fn audit_failed_platform(
    state: &AppState,
    ip: std::net::IpAddr,
    request_id: &str,
) -> Result<(), ApiError> {
    let mut tx = state
        .db()
        .begin_platform()
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    let mut event = AuditEvent::new(AuditAction::SignInFailed, "user");
    event.actor = Actor::System;
    event.source_ip = Some(ip);
    event.diff = Some(diff(None, Some(&unknown_user_diff()), &[]));
    restichub_db::audit::insert_platform(&mut tx, PLATFORM_TENANT_ID, &event)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
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
