//! The enrollment token endpoints (ADR-0026, FR-ID-01, FR-ID-05, FR-TN-03).
//!
//! An Admin creates a one-time, tenant-bound token with a short time to live and revokes an unused
//! token. A Viewer reads the metadata. The platform tenant gets HTTP 403 (the endpoints are
//! tenant-only).
//!
//! The control plane stores only the SHA-256 hash and a short display prefix, in the manner of the
//! API tokens of ADR-0023. The value appears once, at the create. The token is never logged.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use restichub_db::EnrollmentTokenId;
use restichub_db::audit::{AuditAction, diff};
use restichub_db::enrollment_tokens as repo;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::server::AppState;
use crate::token;
use crate::validation::{self, Reason};

/// Shortest accepted time to live.
const MIN_TTL_SECONDS: i64 = 60;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateEnrollmentTokenRequest {
    /// Carried to the enrolled agent row in the registry. Each holds 1 to 64 characters from
    /// `a-z`, `0-9`, `.`, `_` and `-`.
    pub labels: Option<Vec<String>>,
    /// The time to live in seconds. Between 60 and 86400. The default is 900.
    pub ttl_seconds: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateEnrollmentTokenResponse {
    pub id: String,
    /// Returned only here.
    pub token: String,
    pub display_prefix: String,
    pub labels: Vec<String>,
    /// In RFC 3339.
    pub expires_at: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EnrollmentTokenView {
    pub id: String,
    pub display_prefix: String,
    pub labels: Vec<String>,
    /// In RFC 3339.
    pub expires_at: String,
    pub used_at: Option<String>,
    /// In RFC 3339.
    pub created_at: String,
    pub created_by: String,
}

impl From<&repo::EnrollmentToken> for EnrollmentTokenView {
    fn from(entry: &repo::EnrollmentToken) -> Self {
        Self {
            id: entry.id.to_string(),
            display_prefix: entry.display_prefix.clone(),
            labels: entry.labels.clone(),
            expires_at: format_time(entry.expires_at),
            used_at: entry.used_at.map(format_time),
            created_at: format_time(entry.created_at),
            created_by: entry.created_by.to_string(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EnrollmentTokenListResponse {
    pub items: Vec<EnrollmentTokenView>,
    /// Null when the table holds no later token.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentTokenListQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

/// Creates an enrollment token and returns its value one time.
#[utoipa::path(
    post,
    path = "/api/v1/enrollment-tokens",
    summary = "Creates an enrollment token in the tenant.",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateEnrollmentTokenRequest,
    responses(
        (status = 201, description = "The token is created. The value appears once.", body = CreateEnrollmentTokenResponse),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
    )
)]
pub async fn create_enrollment_token(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CreateEnrollmentTokenRequest>,
) -> Result<(StatusCode, Json<CreateEnrollmentTokenResponse>), ApiError> {
    context.authorize(Operation::EnrollmentTokensWrite)?;
    let labels = labels(&body.labels)?;
    let ttl = validate_ttl(body.ttl_seconds)?;
    let created_by = context.actor_user(&state).await?;
    let now = state.auth().clock.now();
    let expires_at = now + time::Duration::seconds(ttl);
    let (value, display_prefix) = token::random_enrollment_token()?;
    let hash = token::hash_token(&value);
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let id = repo::create(
        &mut tx,
        &repo::NewEnrollmentToken {
            token_hash: &hash,
            display_prefix: &display_prefix,
            labels: &labels,
            expires_at,
            created_by,
        },
    )
    .await?;
    let mut event = context.audit_event(
        AuditAction::EnrollmentTokenCreate,
        "enrollment_token",
        id.as_uuid(),
    );
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({
            "display_prefix": display_prefix,
            "labels": labels,
            "expires_at": format_time(expires_at),
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(CreateEnrollmentTokenResponse {
            id: id.to_string(),
            token: value,
            display_prefix,
            labels,
            expires_at: format_time(expires_at),
            created_at: format_time(now),
        }),
    ))
}

/// Lists the enrollment token metadata of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/enrollment-tokens",
    security(("cookie" = []), ("bearer" = [])),
    params(EnrollmentTokenListQuery),
    responses(
        (status = 200, description = "The enrollment tokens of the tenant.", body = EnrollmentTokenListResponse),
        (status = 400, description = "A query parameter is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_enrollment_tokens(
    State(state): State<AppState>,
    context: RequestContext,
    Query(query): Query<EnrollmentTokenListQuery>,
) -> Result<Json<EnrollmentTokenListResponse>, ApiError> {
    context.authorize(Operation::EnrollmentTokensRead)?;
    let page = repo::EnrollmentTokenPage {
        limit: query.limit.unwrap_or(repo::DEFAULT_PAGE_LIMIT),
        after: query
            .cursor
            .as_deref()
            .map(decode_cursor)
            .transpose()
            .map_err(|cause| {
                ApiError::invalid_field_cause("cursor", Reason::InvalidFormat, cause)
            })?,
    };
    let result = context
        .tenant_tx(&state, async |tx| Ok(repo::list(tx, &page).await?))
        .await?;
    let next_cursor = if result.has_more {
        result.tokens.last().map(|entry| {
            encode_cursor(repo::EnrollmentTokenCursor {
                created_at: entry.created_at,
                id: entry.id,
            })
        })
    } else {
        None
    };
    Ok(Json(EnrollmentTokenListResponse {
        items: result
            .tokens
            .iter()
            .map(EnrollmentTokenView::from)
            .collect(),
        next_cursor,
    }))
}

/// Revokes an unused enrollment token of the tenant.
///
/// A used token gives HTTP 409 and changes nothing. A second call on a revoked token gives HTTP 404
/// (the row is removed).
#[utoipa::path(
    delete,
    path = "/api/v1/enrollment-tokens/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the token.")),
    responses(
        (status = 204, description = "The token is revoked."),
        crate::error::ProtectedResponses,
        (status = 404, description = "The token is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The token is used.", body = crate::error::ProblemBody)
    )
)]
pub async fn revoke_enrollment_token(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::EnrollmentTokensWrite)?;
    let token_id = parse_token_id(&id)?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let existing = repo::get(&mut tx, token_id).await?;
    let Some(existing) = existing else {
        return Err(ApiError::not_found());
    };
    if existing.used_at.is_some() {
        return Err(ApiError::conflict());
    }
    let removed = repo::revoke_unused(&mut tx, token_id).await?;
    if removed {
        let mut event = context.audit_event(
            AuditAction::EnrollmentTokenRevoke,
            "enrollment_token",
            token_id.as_uuid(),
        );
        event.diff = Some(diff(
            Some(&serde_json::json!({"display_prefix": existing.display_prefix})),
            None,
            &[],
        ));
        restichub_db::audit::insert(&mut tx, &event).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

fn labels(input: &Option<Vec<String>>) -> Result<Vec<String>, ApiError> {
    let mut labels = Vec::new();
    for value in input.as_deref().unwrap_or_default() {
        labels.push(validation::validate_username("labels", value)?);
    }
    Ok(labels)
}

fn validate_ttl(input: Option<i64>) -> Result<i64, ApiError> {
    match input {
        None => Ok(repo::DEFAULT_TTL_SECONDS),
        Some(ttl) if (MIN_TTL_SECONDS..=repo::MAX_TTL_SECONDS).contains(&ttl) => Ok(ttl),
        Some(_) => Err(ApiError::invalid_field("ttl_seconds", Reason::Unknown)),
    }
}

fn parse_token_id(value: &str) -> Result<EnrollmentTokenId, ApiError> {
    Uuid::parse_str(value)
        .map(EnrollmentTokenId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause("id", Reason::InvalidFormat, error.to_string())
        })
}

fn encode_cursor(cursor: repo::EnrollmentTokenCursor) -> String {
    format!("{};{}", cursor.created_at.unix_timestamp_nanos(), cursor.id)
}

fn decode_cursor(value: &str) -> Result<repo::EnrollmentTokenCursor, String> {
    let (time, id) = value
        .split_once(';')
        .ok_or_else(|| "the cursor holds no `;`".to_owned())?;
    let nanos: i128 = time
        .parse()
        .map_err(|error: std::num::ParseIntError| error.to_string())?;
    let created_at =
        OffsetDateTime::from_unix_timestamp_nanos(nanos).map_err(|error| error.to_string())?;
    if !(1970..=9999).contains(&created_at.year()) {
        return Err("the time of the cursor is outside the supported range".to_owned());
    }
    let id = id.parse().map_err(|error: uuid::Error| error.to_string())?;
    Ok(repo::EnrollmentTokenCursor {
        created_at,
        id: EnrollmentTokenId::from_uuid(id),
    })
}

fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}
