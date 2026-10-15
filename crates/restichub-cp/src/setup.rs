//! Initial setup (FR-AU-03, ADR-0024).
//!
//! When the setup is required, the server generates a one-time setup token. It stores only the
//! SHA-256 hash with an expiry of 24 hours and prints the value once at WARN level. The value is
//! the only secret that the control plane ever logs. The alternative would be a default password,
//! which is worse. The value expires in 24 hours and a restart generates a new one.
//!
//! The setup is required while the tenant that the setup creates holds no user: the tenant
//! `default` in single mode, the platform tenant in multi mode. The token therefore protects both
//! modes.
//!
//! `POST /api/v1/setup` creates the first administrator and needs the setup token. In multi mode
//! it creates the first Platform Admin. In single mode it creates the tenant `default` and its
//! first Admin. A second call is refused in both modes.

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use restichub_db::audit::{Actor, AuditAction, AuditEvent, diff};
use restichub_db::tenants::{self as tenant_repo, BootstrapAdmin, BootstrapTenant};
use restichub_db::{PLATFORM_TENANT_ID, Role, SetupTokenId, TenantId, UserId, setup, users};
use restichub_secrets::{generate_dek, wrap_dek};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::client_ip::ClientIp;
use crate::clock::Clock;
use crate::config::{TenancyConfig, TenancyMode};
use crate::error::{ApiError, map_write_error};
use crate::json::ApiJson;
use crate::password;
use crate::rate_limit;
use crate::server::AppState;
use crate::token;
use crate::validation;
use crate::views::UserView;

pub const SETUP_TOKEN_TTL: time::Duration = time::Duration::hours(24);

const PLATFORM_ADMIN_ROLE: Role = Role::PlatformAdmin;

const SETUP_LOCK: &str = "restichub.setup:";

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error(transparent)]
    Database(#[from] restichub_db::DbError),
    #[error(transparent)]
    Token(#[from] token::TokenError),
}

/// Generates a setup token when the setup is required (ADR-0023, ADR-0024).
///
/// The token belongs to the platform tenant, which always exists. The function prints the value
/// once at WARN level. The value is the only secret that the control plane ever logs.
pub async fn ensure_setup_token(
    db: &restichub_db::Database,
    clock: &dyn Clock,
    tenancy: &TenancyConfig,
) -> Result<(), SetupError> {
    if !setup_required_db(db, tenancy).await? {
        return Ok(());
    }
    let value = token::random_token()?;
    let hash = token::hash_token(&value);
    let now = clock.now();
    let expires_at = now + SETUP_TOKEN_TTL;
    let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
    setup::create(&mut tx, &hash, expires_at).await?;
    tx.commit().await?;
    tracing::warn!(
        setup_token = %value,
        expires_at = %expires_at,
        "the setup is required; use this one-time setup token with POST /api/v1/setup before it expires"
    );
    Ok(())
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SetupStatusResponse {
    /// True while the installation has no administrator.
    pub setup_required: bool,
    pub tenancy_mode: TenancyMode,
}

/// Reports if the setup is required and the tenancy mode.
#[utoipa::path(
    get,
    path = "/api/v1/setup/status",
    responses((status = 200, description = "The setup status.", body = SetupStatusResponse))
)]
pub async fn status(State(state): State<AppState>) -> Result<Json<SetupStatusResponse>, ApiError> {
    let setup_required = setup_required(&state).await?;
    Ok(Json(SetupStatusResponse {
        setup_required,
        tenancy_mode: state.tenancy().mode,
    }))
}

/// True while the installation has no administrator (ADR-0024).
///
/// In multi mode the administrator is the Platform Admin of the platform tenant. In single mode it
/// is the first Admin of the tenant `default`; the tenant may be absent before the setup.
pub async fn setup_required(state: &AppState) -> Result<bool, ApiError> {
    Ok(setup_required_db(state.db(), state.tenancy()).await?)
}

async fn setup_required_db(
    db: &restichub_db::Database,
    tenancy: &TenancyConfig,
) -> Result<bool, restichub_db::DbError> {
    match tenancy.mode {
        // In multi mode the setup is required while no account holds the Platform Admin right
        // (ADR-0029).
        TenancyMode::Multi => {
            let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
            let count = users::platform_admin_count(&mut tx).await?;
            tx.commit().await?;
            Ok(count == 0)
        }
        TenancyMode::Single => {
            let tenant = {
                let mut tx = db.begin_platform().await?;
                let tenant = tenant_repo::find_by_name(&mut tx, &tenancy.default_tenant).await?;
                tx.commit().await?;
                tenant
            };
            let Some(tenant) = tenant else {
                return Ok(true);
            };
            let mut tx = db.begin_tenant(tenant.id).await?;
            let count = setup::user_count(&mut tx).await?;
            tx.commit().await?;
            Ok(count == 0)
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetupRequest {
    /// The one-time token that the control plane prints at start.
    pub setup_token: Option<String>,
    pub username: String,
    /// At least 12 characters.
    pub password: String,
    pub display_name: String,
}

#[utoipa::path(
    post,
    path = "/api/v1/setup",
    summary = "Creates the first administrator.",
    description = "The endpoint needs the one-time setup token, the CSRF header and the per-IP rate limit of sign-in. In multi mode it creates the first Platform Admin. In single mode it creates the tenant `default` and its first Admin.",
    request_body = SetupRequest,
    responses(
        (status = 201, description = "The administrator is created.", body = UserView),
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
    headers: HeaderMap,
    ApiJson(body): ApiJson<SetupRequest>,
) -> Result<(StatusCode, Json<UserView>), ApiError> {
    if !token::csrf_ok(&headers) {
        return Err(ApiError::csrf_required());
    }
    // The setup endpoint uses the same per-IP rate limit as sign-in.
    if let Err(wait) = state
        .auth()
        .ip_limiter
        .check(&rate_limit::ip_key(client.0), Instant::now())
    {
        return Err(ApiError::rate_limited(wait.as_secs().max(1)));
    }
    let Some(setup_token) = body.setup_token.as_deref() else {
        return Err(ApiError::invalid_field(
            "setup_token",
            validation::Reason::Empty,
        ));
    };
    let username = validation::validate_username("username", &body.username)?;
    let display_name = validation::validate_display_name("display_name", &body.display_name)?;
    validation::validate_password("password", &body.password)?;
    match state.tenancy().mode {
        TenancyMode::Single => {
            setup_single(
                &state,
                setup_token,
                &body.password,
                &username,
                &display_name,
                client.0,
            )
            .await
        }
        TenancyMode::Multi => {
            setup_multi(
                &state,
                setup_token,
                &body.password,
                &username,
                &display_name,
            )
            .await
        }
    }
}

/// Creates the tenant `default` and its first Admin (ADR-0024).
///
/// Each step opens one transaction and releases it before the next step, so the setup never holds
/// a pooled connection while it waits for another. The token belongs to the platform tenant,
/// which always exists. The unique index on the tenant name makes exactly one of two concurrent
/// setup calls win; the loser gets HTTP 409 from the create. The tenant, its key row, its first
/// user and the audit records commit in one transaction (ADR-0023, section Tenant isolation).
async fn setup_single(
    state: &AppState,
    setup_token: &str,
    password: &str,
    username: &str,
    display_name: &str,
    source_ip: std::net::IpAddr,
) -> Result<(StatusCode, Json<UserView>), ApiError> {
    let tenancy = state.tenancy();
    let now = state.auth().clock.now();
    // The check runs before the lock, so a repeated call gives 409 at once and holds no
    // connection while it waits for another. The tenant is absent before the setup and present
    // after it, so the cheap check is monotonic in the safe direction.
    if tenant_exists(state, &tenancy.default_tenant).await? {
        return Err(ApiError::setup_already_done());
    }
    let token_id = claim_setup_token(state, setup_token, now).await?;

    let password_hash =
        password::hash_password_limited(&state.auth().hash_semaphore, password.to_owned()).await?;
    let kek = state
        .kek()
        .ok_or_else(|| ApiError::internal("the server holds no key encryption key provider"))?;
    let tenant_id = TenantId::new();
    let dek = generate_dek()?;
    let wrapped = wrap_dek(kek.current(), tenant_id.as_uuid(), 1, &dek)?;
    let input = BootstrapTenant {
        id: tenant_id,
        name: &tenancy.default_tenant,
        key_version: 1,
        kek_id: wrapped.kek_id.as_str(),
        wrapped_dek: &wrapped.ciphertext,
        nonce: wrapped.nonce.as_slice(),
        actor: Actor::System,
        source_ip: Some(source_ip),
        admin: Some(BootstrapAdmin {
            username,
            password_hash: &password_hash,
            display_name,
        }),
    };
    let created = {
        let mut tx = state.db().begin_platform().await?;
        let created = tenant_repo::create_with_bootstrap(&mut tx, &input)
            .await
            .map_err(map_write_error)?;
        tx.commit().await?;
        created
    };
    // The token is consumed only after the tenant exists.
    consume_setup_token(state, token_id, now).await?;

    let member = read_member(state, created, username).await?;
    tracing::info!("the first Admin of the single-tenant installation is created");
    Ok((StatusCode::CREATED, Json(UserView::from_member(&member))))
}

/// Creates the first Platform Admin (ADR-0023, section Local authentication).
///
/// The lock serializes the setup for the platform tenant. A successful setup marks the used token
/// and every other unused token of the tenant as used in the same transaction.
async fn setup_multi(
    state: &AppState,
    setup_token: &str,
    password: &str,
    username: &str,
    display_name: &str,
) -> Result<(StatusCode, Json<UserView>), ApiError> {
    let now = state.auth().clock.now();
    let mut tx = state.db().begin_tenant(PLATFORM_TENANT_ID).await?;
    tx.lock(SETUP_LOCK).await?;
    if users::platform_admin_count(&mut tx).await? > 0 {
        return Err(ApiError::setup_already_done());
    }
    let hash = token::hash_token(setup_token);
    let token_id = setup::find_valid(&mut tx, &hash, now).await?;
    let Some(token_id) = token_id else {
        return Err(ApiError::invalid_setup_token());
    };
    let password_hash =
        password::hash_password_limited(&state.auth().hash_semaphore, password.to_owned()).await?;
    // The first account holds the Platform Admin right and no membership (ADR-0029).
    let user_id = UserId::new();
    let created = users::create_platform_admin(
        &mut tx,
        user_id,
        &users::NewUser {
            username,
            password_hash: &password_hash,
            display_name,
        },
    )
    .await?;
    if !created {
        return Err(ApiError::setup_already_done());
    }
    let marked = setup::mark_used(&mut tx, token_id, now).await?;
    if !marked {
        return Err(ApiError::setup_already_done());
    }
    // Every other unused token becomes used too, so no old token can run the setup again.
    setup::mark_all_used(&mut tx, now).await?;
    let mut event = AuditEvent::new(AuditAction::UserCreate, "user");
    event.actor = Actor::System;
    event.object_id = Some(user_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({
            "username": username,
            "display_name": display_name,
            "platform_admin": true,
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    let user = UserView {
        id: user_id.to_string(),
        username: username.to_owned(),
        display_name: display_name.to_owned(),
        disabled: false,
        role: PLATFORM_ADMIN_ROLE,
        shared: false,
    };
    tracing::info!("the first Platform Admin is created");
    Ok((StatusCode::CREATED, Json(user)))
}

/// Finds the unused, unexpired setup token in the platform tenant.
///
/// The transaction is read-only: it takes the lock and the row lock of the token, then releases
/// both. The token is marked used only after the tenant or the user exists.
async fn claim_setup_token(
    state: &AppState,
    setup_token: &str,
    now: time::OffsetDateTime,
) -> Result<SetupTokenId, ApiError> {
    let hash = token::hash_token(setup_token);
    let mut tx = state.db().begin_tenant(PLATFORM_TENANT_ID).await?;
    tx.lock(SETUP_LOCK).await?;
    let token_id = setup::find_valid(&mut tx, &hash, now).await?;
    tx.commit().await?;
    token_id.ok_or_else(ApiError::invalid_setup_token)
}

/// Marks the token and every other unused token of the platform tenant as used.
///
/// A lost race leaves the token valid, because the transaction of the loser rolls back.
async fn consume_setup_token(
    state: &AppState,
    token_id: SetupTokenId,
    now: time::OffsetDateTime,
) -> Result<(), ApiError> {
    let mut tx = state.db().begin_tenant(PLATFORM_TENANT_ID).await?;
    tx.lock(SETUP_LOCK).await?;
    let marked = setup::mark_used(&mut tx, token_id, now).await?;
    if !marked {
        return Err(ApiError::setup_already_done());
    }
    setup::mark_all_used(&mut tx, now).await?;
    tx.commit().await?;
    Ok(())
}

async fn tenant_exists(state: &AppState, name: &str) -> Result<bool, ApiError> {
    let mut tx = state.db().begin_platform().await?;
    let tenant = tenant_repo::find_by_name(&mut tx, name).await?;
    tx.commit().await?;
    Ok(tenant.is_some())
}

async fn read_member(
    state: &AppState,
    tenant: TenantId,
    username: &str,
) -> Result<users::Member, ApiError> {
    let mut tx = state.db().begin_tenant(tenant).await?;
    let member = users::list_members(&mut tx)
        .await?
        .into_iter()
        .find(|member| member.username.eq_ignore_ascii_case(username));
    tx.commit().await?;
    member.ok_or_else(|| ApiError::internal("the created Admin is not found after the read"))
}
