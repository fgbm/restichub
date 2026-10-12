//! The initial setup (issue #18, FR-AU-03).
//!
//! At start, when the platform tenant has no user, the server generates a one-time setup token. It
//! stores only the SHA-256 hash with an expiry of 24 hours and prints the value once at WARN level.
//! The value is the only secret that the control plane ever logs. The alternative would be a
//! default password, which is worse. The value expires in 24 hours and a restart generates a new
//! one.
//!
//! `POST /api/v1/setup` creates the first Platform Admin while the platform tenant has no user. It
//! marks the token used. A second call is refused.

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use restichub_db::audit::{Actor, AuditAction, AuditEvent, diff};
use restichub_db::{PLATFORM_TENANT_ID, memberships, setup, users};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::client_ip::ClientIp;
use crate::clock::Clock;
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::password;
use crate::rate_limit;
use crate::server::{AppState, RequestIdHeader};
use crate::token;
use crate::validation;
use crate::views::UserView;

/// The lifetime of a setup token.
pub const SETUP_TOKEN_TTL: time::Duration = time::Duration::hours(24);

/// The role of the first user of the platform tenant.
const PLATFORM_ADMIN_ROLE: &str = "platform_admin";

/// The advisory lock scope of the initial setup.
const SETUP_LOCK: &str = "restichub.setup:";

/// The error of the setup token generation.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// A database operation failed.
    #[error(transparent)]
    Database(#[from] restichub_db::DbError),
    /// The random token generation failed.
    #[error(transparent)]
    Token(#[from] token::TokenError),
}

/// Generates a setup token when the platform tenant has no user.
///
/// The function prints the value once at WARN level. The value is the only secret that the control
/// plane ever logs.
pub async fn ensure_setup_token(
    db: &restichub_db::Database,
    clock: &dyn Clock,
) -> Result<(), SetupError> {
    let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
    if setup::user_count(&mut tx).await? > 0 {
        tx.commit().await?;
        return Ok(());
    }
    let value = token::random_token()?;
    let hash = token::hash_token(&value);
    let now = clock.now();
    let expires_at = now + SETUP_TOKEN_TTL;
    setup::create(&mut tx, &hash, expires_at).await?;
    tx.commit().await?;
    tracing::warn!(
        setup_token = %value,
        expires_at = %expires_at,
        "the platform tenant has no user; use this one-time setup token with POST /api/v1/setup before it expires"
    );
    Ok(())
}

/// The body of the setup status response.
#[derive(Debug, Serialize, ToSchema)]
pub struct SetupStatusResponse {
    /// True while the platform tenant has no user.
    pub setup_required: bool,
}

/// Reports if the setup is required.
#[utoipa::path(
    get,
    path = "/api/v1/setup/status",
    responses((status = 200, description = "The setup status.", body = SetupStatusResponse))
)]
pub async fn status(
    State(state): State<AppState>,
    request_id: RequestIdHeader,
) -> Result<Json<SetupStatusResponse>, ApiError> {
    let request_id = request_id.0;
    let mut tx = state
        .db()
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let count = setup::user_count(&mut tx)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    Ok(Json(SetupStatusResponse {
        setup_required: count == 0,
    }))
}

/// The body of the setup request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetupRequest {
    /// The one-time setup token from the server log.
    pub setup_token: String,
    /// The username of the Platform Admin.
    pub username: String,
    /// The password of the Platform Admin. It must hold at least 12 characters.
    pub password: String,
    /// The name for the user interface.
    pub display_name: String,
}

/// Creates the first Platform Admin.
///
/// The endpoint needs the CSRF header and it is rate-limited for each source IP, as sign-in is. The
/// platform tenant lock and the row lock of the token serialize two setup requests with the same
/// token, so exactly one succeeds.
#[utoipa::path(
    post,
    path = "/api/v1/setup",
    request_body = SetupRequest,
    responses(
        (status = 201, description = "The Platform Admin is created.", body = UserView),
        (status = 400, description = "The password or a field is not valid.", body = crate::error::ProblemBody),
        (status = 401, description = "The setup token is not valid.", body = crate::error::ProblemBody),
        (status = 403, description = "The CSRF header is missing.", body = crate::error::ProblemBody),
        (status = 409, description = "The setup is already done.", body = crate::error::ProblemBody),
        (status = 429, description = "Too many setup attempts.", body = crate::error::ProblemBody),
        (status = 503, description = "The password work is busy.", body = crate::error::ProblemBody)
    )
)]
pub async fn setup(
    State(state): State<AppState>,
    client: ClientIp,
    request_id: RequestIdHeader,
    headers: HeaderMap,
    ApiJson(body): ApiJson<SetupRequest>,
) -> Result<(StatusCode, Json<UserView>), ApiError> {
    let request_id = request_id.0;
    if !token::csrf_ok(&headers) {
        return Err(ApiError::csrf_required(&request_id));
    }
    // The setup endpoint uses the same per-IP rate limit as sign-in.
    if let Err(wait) = state
        .auth()
        .ip_limiter
        .check(&rate_limit::ip_key(client.0), Instant::now())
    {
        return Err(ApiError::rate_limited(&request_id, wait.as_secs().max(1)));
    }
    let username = validation::validate_username(&body.username)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;
    let display_name = validation::validate_display_name(&body.display_name)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;
    password::validate_password(&body.password)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;
    let now = state.auth().clock.now();
    let mut tx = state
        .db()
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    // The lock serializes the whole setup for the platform tenant.
    tx.lock(SETUP_LOCK)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    if setup::user_count(&mut tx)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?
        > 0
    {
        return Err(ApiError::conflict(&request_id));
    }
    let hash = token::hash_token(&body.setup_token);
    let token_id = setup::find_valid(&mut tx, &hash, now)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let Some(token_id) = token_id else {
        return Err(ApiError::invalid_credentials(&request_id));
    };
    let password_hash =
        password::hash_password_limited(&state.auth().hash_semaphore, body.password.clone())
            .await
            .map_err(|error| ApiError::from_password(&request_id, &error))?;
    let user_id = users::create(
        &mut tx,
        &users::NewUser {
            username: &username,
            password_hash: &password_hash,
            display_name: &display_name,
        },
    )
    .await
    .map_err(|error| ApiError::internal(&request_id, &error))?;
    memberships::set_role(&mut tx, user_id, PLATFORM_ADMIN_ROLE)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    // A second call with the same token sees `used_at` set and is refused.
    let marked = setup::mark_used(&mut tx, token_id, now)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    if !marked {
        return Err(ApiError::conflict(&request_id));
    }
    let mut event = AuditEvent::new(AuditAction::UserCreate, "user");
    event.actor = Actor::System;
    event.object_id = Some(user_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({
            "username": username,
            "display_name": display_name,
            "role": PLATFORM_ADMIN_ROLE,
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let user = users::get(&mut tx, user_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let Some(user) = user else {
        return Err(ApiError::internal_error(&request_id));
    };
    tracing::info!(request_id = %request_id, "the first Platform Admin is created");
    Ok((
        StatusCode::CREATED,
        Json(UserView::new(&user, PLATFORM_ADMIN_ROLE)),
    ))
}
