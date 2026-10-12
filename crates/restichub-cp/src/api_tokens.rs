//! The API token endpoints for the Admin role (issue #18, ADR-0023).
//!
//! The value of a token is returned only in the create response. The list holds the display
//! prefix. A token cannot create a token with a higher role than its own, and a user cannot
//! either.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::audit::{AuditAction, AuditEvent, diff};
use restichub_db::{ApiTokenId, api_tokens};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::context::{Operation, RequestContext, allowed_role};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::server::{AppState, RequestIdHeader};
use crate::token;

/// The body of the create-token request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTokenRequest {
    /// The name for the user interface.
    pub name: String,
    /// The role of the token in the tenant.
    pub role: String,
    /// The optional expiry time in RFC 3339.
    pub expires_at: Option<String>,
}

/// The body of the create-token response. The value appears here only once.
#[derive(Debug, Serialize, ToSchema)]
pub struct CreateTokenResponse {
    /// The identifier of the token.
    pub id: String,
    /// The token value. The control plane returns it only here.
    pub token: String,
    /// The name for the user interface.
    pub name: String,
    /// The role of the token.
    pub role: String,
    /// The display prefix of the token.
    pub display_prefix: String,
    /// The optional expiry time.
    pub expires_at: Option<String>,
}

/// A token in a list response.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiTokenView {
    /// The identifier of the token.
    pub id: String,
    /// The name for the user interface.
    pub name: String,
    /// The role of the token.
    pub role: String,
    /// The display prefix of the token.
    pub display_prefix: String,
    /// The creation time.
    pub created_at: String,
    /// The optional expiry time.
    pub expires_at: Option<String>,
    /// The time of the last use.
    pub last_used_at: Option<String>,
    /// The revocation time.
    pub revoked_at: Option<String>,
}

impl From<&api_tokens::ApiToken> for ApiTokenView {
    fn from(token: &api_tokens::ApiToken) -> Self {
        Self {
            id: token.id.to_string(),
            name: token.name.clone(),
            role: token.role.clone(),
            display_prefix: token.display_prefix.clone(),
            created_at: format_time(token.created_at),
            expires_at: token.expires_at.map(format_time),
            last_used_at: token.last_used_at.map(format_time),
            revoked_at: token.revoked_at.map(format_time),
        }
    }
}

/// A list of API tokens.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiTokenListResponse {
    /// The tokens.
    pub items: Vec<ApiTokenView>,
}

/// Lists the API tokens of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/api-tokens",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The API tokens of the tenant.", body = ApiTokenListResponse),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody)
    )
)]
pub async fn list_tokens(
    State(state): State<AppState>,
    context: RequestContext,
    request_id: RequestIdHeader,
) -> Result<Json<ApiTokenListResponse>, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::ApiTokensRead, &request_id)?;
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let tokens = api_tokens::list(&mut tx)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    Ok(Json(ApiTokenListResponse {
        items: tokens.iter().map(ApiTokenView::from).collect(),
    }))
}

/// Creates an API token in the tenant.
#[utoipa::path(
    post,
    path = "/api/v1/api-tokens",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateTokenRequest,
    responses(
        (status = 201, description = "The token is created. The value appears once.", body = CreateTokenResponse),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient or the role is too high.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_token(
    State(state): State<AppState>,
    context: RequestContext,
    request_id: RequestIdHeader,
    ApiJson(body): ApiJson<CreateTokenRequest>,
) -> Result<(StatusCode, Json<CreateTokenResponse>), ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::ApiTokensWrite, &request_id)?;
    let role = allowed_role(&context, &body.role, &request_id)?;
    let name = crate::validation::validate_token_name(&body.name)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;
    let now = state.auth().clock.now();
    let expires_at = match &body.expires_at {
        Some(value) => {
            let parsed = OffsetDateTime::parse(value, &Rfc3339)
                .map_err(|_| ApiError::invalid_argument(&request_id))?;
            crate::validation::validate_expiry(parsed, now)
                .map_err(|_| ApiError::invalid_argument(&request_id))?;
            Some(parsed)
        }
        None => None,
    };
    let created_by = match context.actor {
        restichub_db::audit::Actor::User(user_id) => user_id,
        restichub_db::audit::Actor::ApiToken(token_id) => {
            creator_of(&state, context.tenant_id, token_id, &request_id).await?
        }
        _ => return Err(ApiError::forbidden(&request_id)),
    };
    let (value, display_prefix) =
        token::random_api_token().map_err(|error| ApiError::internal(&request_id, &error))?;
    let hash = token::hash_token(&value);
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let id = api_tokens::create(
        &mut tx,
        &api_tokens::NewApiToken {
            name: &name,
            token_hash: &hash,
            display_prefix: &display_prefix,
            role: role.as_str(),
            created_by,
            expires_at,
        },
    )
    .await
    .map_err(|error| {
        if error.is_unique_violation() {
            ApiError::conflict(&request_id)
        } else {
            ApiError::internal(&request_id, &error)
        }
    })?;
    let mut event = AuditEvent::new(AuditAction::ApiTokenCreate, "api_token");
    event.actor = context.actor.clone();
    event.object_id = Some(id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({
            "name": name,
            "role": role.as_str(),
            "display_prefix": display_prefix,
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    Ok((
        StatusCode::CREATED,
        Json(CreateTokenResponse {
            id: id.to_string(),
            token: value,
            name,
            role: role.as_str().to_owned(),
            display_prefix,
            expires_at: expires_at.map(format_time),
        }),
    ))
}

/// Revokes an API token of the tenant.
///
/// A second call on the same token changes nothing (idempotent).
#[utoipa::path(
    delete,
    path = "/api/v1/api-tokens/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the token.")),
    responses(
        (status = 204, description = "The token is revoked."),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody),
        (status = 404, description = "The token is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn revoke_token(
    State(state): State<AppState>,
    context: RequestContext,
    request_id: RequestIdHeader,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::ApiTokensWrite, &request_id)?;
    let token_id = Uuid::parse_str(&id)
        .map(ApiTokenId::from_uuid)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;
    let now = state.auth().clock.now();
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let token = api_tokens::get(&mut tx, token_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    if token.is_none() {
        return Err(ApiError::not_found(&request_id));
    }
    let changed = api_tokens::revoke(&mut tx, token_id, now)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    if changed {
        let mut event = AuditEvent::new(AuditAction::ApiTokenRevoke, "api_token");
        event.actor = context.actor.clone();
        event.object_id = Some(token_id.as_uuid());
        restichub_db::audit::insert(&mut tx, &event)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
    }
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Reads the creator of an API token.
async fn creator_of(
    state: &AppState,
    tenant: restichub_db::TenantId,
    token_id: ApiTokenId,
    request_id: &str,
) -> Result<restichub_db::UserId, ApiError> {
    let mut tx = state
        .db()
        .begin_tenant(tenant)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    let token = api_tokens::get(&mut tx, token_id)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    // An API token is owned by the tenant. A token that creates a token records the same creator
    // as the current token.
    token
        .map(|token| token.created_by)
        .ok_or_else(|| ApiError::internal_error(request_id))
}

/// Formats a time as RFC 3339.
fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}
