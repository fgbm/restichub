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
use restichub_db::audit::{AuditAction, diff};
use restichub_db::enrollment_tokens as repo;
use restichub_db::{AgentId, EnrollmentTokenId};
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
    /// Carried to the enrolled agent row in the registry. Each holds 1 to 63 characters from
    /// `a-z`, `0-9` and `-`. Duplicates are removed.
    pub labels: Option<Vec<String>>,
    /// The time to live in seconds. Between 60 and 86400. The default is 3600.
    pub ttl_seconds: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct CreateEnrollmentTokenResponse {
    pub id: String,
    /// Returned only here.
    pub token: String,
    pub display_prefix: String,
    pub labels: Vec<String>,
    /// In RFC 3339.
    pub expires_at: String,
    pub created_at: String,
    /// The host that the token re-registers. Null for a token of a new host.
    pub replaces_agent_id: Option<String>,
}

/// The manual `Debug` hides the token value, so a log line cannot print it by accident.
impl std::fmt::Debug for CreateEnrollmentTokenResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CreateEnrollmentTokenResponse")
            .field("id", &self.id)
            .field("token", &"<redacted>")
            .field("display_prefix", &self.display_prefix)
            .field("labels", &self.labels)
            .field("expires_at", &self.expires_at)
            .field("created_at", &self.created_at)
            .field("replaces_agent_id", &self.replaces_agent_id)
            .finish()
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct EnrollmentTokenView {
    pub id: String,
    pub display_prefix: String,
    pub labels: Vec<String>,
    /// In RFC 3339.
    pub expires_at: String,
    pub used_at: Option<String>,
    pub revoked_at: Option<String>,
    pub revoked_by_user_id: Option<String>,
    /// In RFC 3339.
    pub created_at: String,
    pub created_by: String,
    /// The host that the token re-registers. Null for a token of a new host.
    pub replaces_agent_id: Option<String>,
    /// The host that the token enrolled. Null until the token is used.
    pub used_by_agent_id: Option<String>,
}

impl From<&repo::EnrollmentToken> for EnrollmentTokenView {
    fn from(entry: &repo::EnrollmentToken) -> Self {
        Self {
            id: entry.id.to_string(),
            display_prefix: entry.display_prefix.clone(),
            labels: entry.labels.clone(),
            expires_at: format_time(entry.expires_at),
            used_at: entry.used_at.map(format_time),
            revoked_at: entry.revoked_at.map(format_time),
            revoked_by_user_id: entry.revoked_by_user_id.map(|id| id.to_string()),
            created_at: format_time(entry.created_at),
            created_by: entry.created_by.to_string(),
            replaces_agent_id: entry.replaces_agent_id.map(|id| id.to_string()),
            used_by_agent_id: entry.used_by_agent_id.map(|id| id.to_string()),
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
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentTokenListQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    /// `true` lists only the tokens that can still enroll a host: not used, not revoked and not
    /// expired.
    pub pending: Option<bool>,
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
    let response = issue_token(&state, &context, labels, ttl, None).await?;
    Ok((StatusCode::CREATED, Json(response)))
}

/// Creates a token and its audit record in one transaction. `replaces` binds the token to an
/// existing host (FR-ID-01).
pub(crate) async fn issue_token(
    state: &AppState,
    context: &RequestContext,
    labels: Vec<String>,
    ttl: i64,
    replaces: Option<AgentId>,
) -> Result<CreateEnrollmentTokenResponse, ApiError> {
    let created_by = context.actor_user(state).await?;
    let now = state.auth().clock.now();
    let expires_at = now + time::Duration::seconds(ttl);
    let (value, display_prefix) = token::random_enrollment_token()?;
    let hash = token::hash_token(&value);
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    if let Some(agent) = replaces {
        // One spendable bound token per host: a leaked older token cannot take the host over
        // (FR-ID-10, T8).
        repo::revoke_unused_for_agent(&mut tx, agent, Some(created_by), now).await?;
    }
    let id = repo::create(
        &mut tx,
        &repo::NewEnrollmentToken {
            token_hash: &hash,
            display_prefix: &display_prefix,
            labels: &labels,
            expires_at,
            created_by,
            replaces_agent_id: replaces,
        },
    )
    .await
    .map_err(crate::error::map_write_error)?;
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
            "replaces_agent_id": replaces.map(|agent| agent.to_string()),
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(CreateEnrollmentTokenResponse {
        id: id.to_string(),
        token: value,
        display_prefix,
        labels,
        expires_at: format_time(expires_at),
        created_at: format_time(now),
        replaces_agent_id: replaces.map(|agent| agent.to_string()),
    })
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
        pending_at: query
            .pending
            .unwrap_or(false)
            .then(|| state.auth().clock.now()),
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

/// Reads the metadata of one enrollment token.
#[utoipa::path(
    get,
    path = "/api/v1/enrollment-tokens/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the token.")),
    responses(
        (status = 200, description = "The token metadata.", body = EnrollmentTokenView),
        (status = 400, description = "The identifier is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The token is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_enrollment_token(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<EnrollmentTokenView>, ApiError> {
    context.authorize(Operation::EnrollmentTokensRead)?;
    let token_id = parse_token_id(&id)?;
    let entry = context
        .tenant_tx(&state, async |tx| Ok(repo::get(tx, token_id).await?))
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok(Json(EnrollmentTokenView::from(&entry)))
}

/// Revokes an unused enrollment token of the tenant (a soft change).
///
/// A used, an already revoked or an expired token gives HTTP 409 with the code `token_used`,
/// `token_already_revoked` or `token_expired` and changes nothing. An unknown token gives HTTP 404.
/// The conditional update and the re-read answer without a time-of-check gap, and a failed revoke
/// writes no audit record.
#[utoipa::path(
    delete,
    path = "/api/v1/enrollment-tokens/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the token.")),
    responses(
        (status = 204, description = "The token is revoked."),
        crate::error::ProtectedResponses,
        (status = 404, description = "The token is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The token is used (`token_used`), already revoked (`token_already_revoked`) or expired (`token_expired`).", body = crate::error::ProblemBody)
    )
)]
pub async fn revoke_enrollment_token(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::EnrollmentTokensWrite)?;
    let token_id = parse_token_id(&id)?;
    let actor = context.actor_user(&state).await?;
    let now = state.auth().clock.now();
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let changed = repo::revoke(&mut tx, token_id, actor, now).await?;
    if !changed {
        // The conditional update touched no row: the token is unknown, used or already revoked.
        let existing = repo::get(&mut tx, token_id).await?;
        return match existing {
            None => Err(ApiError::not_found()),
            Some(entry) if entry.used_at.is_some() => Err(ApiError::token_used()),
            Some(entry) if entry.revoked_at.is_some() => Err(ApiError::token_already_revoked()),
            Some(_) => Err(ApiError::token_expired()),
        };
    }
    let existing = repo::get(&mut tx, token_id).await?;
    let Some(existing) = existing else {
        return Err(ApiError::not_found());
    };
    let mut event = context.audit_event(
        AuditAction::EnrollmentTokenRevoke,
        "enrollment_token",
        token_id.as_uuid(),
    );
    event.diff = Some(diff(
        Some(&serde_json::json!({"display_prefix": existing.display_prefix})),
        Some(&serde_json::json!({"revoked_at": format_time(now)})),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

fn labels(input: &Option<Vec<String>>) -> Result<Vec<String>, ApiError> {
    let mut labels = Vec::new();
    for value in input.as_deref().unwrap_or_default() {
        let label = validation::validate_label("labels", value)?;
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    Ok(labels)
}

pub(crate) fn validate_ttl(input: Option<i64>) -> Result<i64, ApiError> {
    validate_ttl_up_to(input, repo::MAX_TTL_SECONDS)
}

/// The time to live of a token bound to a host: the default and the highest value are the lower
/// of the normal limit and the limit for bound tokens (FR-ID-10).
pub(crate) fn validate_bound_ttl(input: Option<i64>) -> Result<i64, ApiError> {
    let max = crate::reenroll_limits::reenroll_token_max_ttl_secs().min(repo::MAX_TTL_SECONDS);
    match input {
        None => Ok(repo::DEFAULT_TTL_SECONDS.min(max)),
        Some(_) => validate_ttl_up_to(input, max),
    }
}

fn validate_ttl_up_to(input: Option<i64>, max: i64) -> Result<i64, ApiError> {
    match input {
        None => Ok(repo::DEFAULT_TTL_SECONDS),
        Some(ttl) if (MIN_TTL_SECONDS..=max).contains(&ttl) => Ok(ttl),
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
