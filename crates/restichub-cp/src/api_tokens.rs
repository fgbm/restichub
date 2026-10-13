//! The API token endpoints for the Admin role (ADR-0023).
//!
//! The value of a token is returned only in the create response. The list holds the display
//! prefix. A token cannot create a token with a higher role than its own, and a user cannot
//! either.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::audit::{Actor, AuditAction, diff};
use restichub_db::{ApiTokenId, Role, api_tokens};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::context::{Operation, RequestContext, allowed_role};
use crate::error::{ApiError, map_write_error};
use crate::json::ApiJson;
use crate::server::AppState;
use crate::token;
use crate::validation::Reason;

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
    pub role: Role,
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
    pub role: Role,
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
            role: token.role,
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
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_tokens(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<ApiTokenListResponse>, ApiError> {
    context.authorize(Operation::ApiTokensRead)?;
    let tokens = context
        .tenant_tx(&state, async |tx| Ok(api_tokens::list(tx).await?))
        .await?;
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
        crate::error::ProtectedResponses,
    )
)]
pub async fn create_token(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CreateTokenRequest>,
) -> Result<(StatusCode, Json<CreateTokenResponse>), ApiError> {
    context.authorize(Operation::ApiTokensWrite)?;
    let role = allowed_role(&context, &body.role)?;
    let name = crate::validation::validate_token_name("name", &body.name)?;
    let now = state.auth().clock.now();
    let expires_at = match &body.expires_at {
        Some(value) => {
            let parsed = OffsetDateTime::parse(value, &Rfc3339).map_err(|error| {
                ApiError::invalid_field_cause(
                    "expires_at",
                    Reason::InvalidFormat,
                    error.to_string(),
                )
            })?;
            crate::validation::validate_expiry("expires_at", parsed, now)?;
            Some(parsed)
        }
        None => None,
    };
    let created_by = match context.actor {
        Actor::User(user_id) => user_id,
        Actor::ApiToken(token_id) => creator_of(&state, context.tenant_id, token_id).await?,
        _ => return Err(ApiError::forbidden()),
    };
    let (value, display_prefix) = token::random_api_token()?;
    let hash = token::hash_token(&value);
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let id = api_tokens::create(
        &mut tx,
        &api_tokens::NewApiToken {
            name: &name,
            token_hash: &hash,
            display_prefix: &display_prefix,
            role,
            created_by,
            expires_at,
        },
    )
    .await
    .map_err(map_write_error)?;
    let mut event = context.audit_event(AuditAction::ApiTokenCreate, "api_token", id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({
            "name": name,
            "role": role.as_str(),
            "display_prefix": display_prefix,
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(CreateTokenResponse {
            id: id.to_string(),
            token: value,
            name,
            role,
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
        crate::error::ProtectedResponses,
        (status = 404, description = "The token is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn revoke_token(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::ApiTokensWrite)?;
    let token_id = Uuid::parse_str(&id)
        .map(ApiTokenId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause("id", Reason::InvalidFormat, error.to_string())
        })?;
    let now = state.auth().clock.now();
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let token = api_tokens::get(&mut tx, token_id).await?;
    if token.is_none() {
        return Err(ApiError::not_found());
    }
    let changed = api_tokens::revoke(&mut tx, token_id, now).await?;
    if changed {
        let event =
            context.audit_event(AuditAction::ApiTokenRevoke, "api_token", token_id.as_uuid());
        restichub_db::audit::insert(&mut tx, &event).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Reads the creator of an API token.
async fn creator_of(
    state: &AppState,
    tenant: restichub_db::TenantId,
    token_id: ApiTokenId,
) -> Result<restichub_db::UserId, ApiError> {
    let mut tx = state.db().begin_tenant(tenant).await?;
    let token = api_tokens::get(&mut tx, token_id).await?;
    tx.commit().await?;
    // An API token is owned by the tenant. A token that creates a token records the same creator
    // as the current token.
    token.map(|token| token.created_by).ok_or_else(|| {
        ApiError::internal(format!("the token {token_id} is not found after the read"))
    })
}

/// Formats a time as RFC 3339.
fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}
