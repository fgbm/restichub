//! The agent registry endpoints (ADR-0026, FR-ID-04, FR-TN-01, FR-TN-03).
//!
//! An Admin lists, reads, renames and revokes the agents of its tenant. A Viewer reads. The
//! platform tenant gets HTTP 403 (the endpoints are tenant-only). An agent of another tenant gives
//! HTTP 404 (AC-TN-01).
//!
//! The revoke writes the status and the audit record in one transaction. The stream close follows
//! in #52; the layer reads the status of this row (AC-ID-04).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use restichub_db::agents as repo;
use restichub_db::audit::{AuditAction, diff};
use restichub_db::{AgentId, AgentStatus};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::server::AppState;
use crate::validation::Reason;

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentView {
    pub id: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub agent_version: String,
    pub restic_version: String,
    pub labels: Vec<String>,
    /// Lower-case hexadecimal.
    pub certificate_serial: String,
    /// In RFC 3339.
    pub certificate_expires_at: String,
    pub status: AgentStatus,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub revoked_at: Option<String>,
    pub revoked_by_user_id: Option<String>,
}

impl From<&repo::Agent> for AgentView {
    fn from(agent: &repo::Agent) -> Self {
        Self {
            id: agent.id.to_string(),
            hostname: agent.hostname.clone(),
            os: agent.os.clone(),
            arch: agent.arch.clone(),
            agent_version: agent.agent_version.clone(),
            restic_version: agent.restic_version.clone(),
            labels: agent.labels.clone(),
            certificate_serial: to_hex(&agent.certificate_serial),
            certificate_expires_at: format_time(agent.certificate_expires_at),
            status: agent.status,
            last_seen_at: agent.last_seen_at.map(format_time),
            created_at: format_time(agent.created_at),
            revoked_at: agent.revoked_at.map(format_time),
            revoked_by_user_id: agent.revoked_by_user_id.map(|id| id.to_string()),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentListResponse {
    pub items: Vec<AgentView>,
    /// Null when the table holds no later agent.
    pub next_cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct AgentListQuery {
    /// `active` or `revoked`.
    pub status: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameAgentRequest {
    pub hostname: String,
}

/// Lists the agents of the tenant with keyset pagination and an optional status filter.
#[utoipa::path(
    get,
    path = "/api/v1/agents",
    security(("cookie" = []), ("bearer" = [])),
    params(AgentListQuery),
    responses(
        (status = 200, description = "The agents of the tenant.", body = AgentListResponse),
        (status = 400, description = "A query parameter is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_agents(
    State(state): State<AppState>,
    context: RequestContext,
    Query(query): Query<AgentListQuery>,
) -> Result<Json<AgentListResponse>, ApiError> {
    context.authorize(Operation::AgentsRead)?;
    let status = match query.status.as_deref() {
        Some(value) => Some(
            AgentStatus::parse(value)
                .ok_or_else(|| ApiError::invalid_field("status", Reason::Unknown))?,
        ),
        None => None,
    };
    let page = repo::AgentPage {
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
        .tenant_tx(&state, async |tx| Ok(repo::list(tx, status, &page).await?))
        .await?;
    let next_cursor = if result.has_more {
        result.agents.last().map(|agent| {
            encode_cursor(repo::AgentCursor {
                created_at: agent.created_at,
                id: agent.id,
            })
        })
    } else {
        None
    };
    Ok(Json(AgentListResponse {
        items: result.agents.iter().map(AgentView::from).collect(),
        next_cursor,
    }))
}

/// Reads one agent of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/agents/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the agent.")),
    responses(
        (status = 200, description = "The agent.", body = AgentView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The agent is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_agent(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<AgentView>, ApiError> {
    context.authorize(Operation::AgentsRead)?;
    let agent_id = parse_agent_id(&id)?;
    let agent = context
        .tenant_tx(&state, async |tx| Ok(repo::get(tx, agent_id).await?))
        .await?;
    match agent {
        Some(agent) => Ok(Json(AgentView::from(&agent))),
        None => Err(ApiError::not_found()),
    }
}

/// Renames an agent of the tenant.
#[utoipa::path(
    patch,
    path = "/api/v1/agents/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the agent.")),
    request_body = RenameAgentRequest,
    responses(
        (status = 200, description = "The renamed agent.", body = AgentView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The agent is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn rename_agent(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RenameAgentRequest>,
) -> Result<Json<AgentView>, ApiError> {
    context.authorize(Operation::AgentsWrite)?;
    let agent_id = parse_agent_id(&id)?;
    let hostname = crate::validation::validate_display_name("hostname", &body.hostname)?;
    let agent = context
        .tenant_tx(&state, async |tx| {
            let Some(current) = repo::get(tx, agent_id).await? else {
                return Ok(None);
            };
            if current.hostname != hostname {
                repo::rename(tx, agent_id, &hostname).await?;
                let mut event =
                    context.audit_event(AuditAction::AgentUpdate, "agent", agent_id.as_uuid());
                event.diff = Some(diff(
                    Some(&serde_json::json!({"hostname": current.hostname})),
                    Some(&serde_json::json!({"hostname": hostname})),
                    &[],
                ));
                restichub_db::audit::insert(tx, &event).await?;
            }
            Ok(repo::get(tx, agent_id).await?)
        })
        .await?;
    match agent {
        Some(agent) => Ok(Json(AgentView::from(&agent))),
        None => Err(ApiError::not_found()),
    }
}

/// Revokes an agent of the tenant (FR-ID-04, AC-ID-04).
///
/// The endpoint is idempotent: a second call changes nothing. The status change and the audit
/// record commit in one transaction.
#[utoipa::path(
    post,
    path = "/api/v1/agents/{id}/revoke",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the agent.")),
    responses(
        (status = 204, description = "The agent is revoked."),
        crate::error::ProtectedResponses,
        (status = 404, description = "The agent is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn revoke_agent(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::AgentsWrite)?;
    let agent_id = parse_agent_id(&id)?;
    let actor = context.actor_user(&state).await?;
    let now = state.auth().clock.now();
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let existing = repo::get(&mut tx, agent_id).await?;
    if existing.is_none() {
        return Err(ApiError::not_found());
    }
    let changed = repo::revoke(&mut tx, agent_id, actor, now).await?;
    if changed {
        let mut event = context.audit_event(AuditAction::AgentRevoke, "agent", agent_id.as_uuid());
        event.diff = Some(diff(
            Some(&serde_json::json!({"status": AgentStatus::Active.as_str()})),
            Some(&serde_json::json!({"status": AgentStatus::Revoked.as_str()})),
            &[],
        ));
        restichub_db::audit::insert(&mut tx, &event).await?;
    }
    tx.commit().await?;
    // A revoke closes an open stream at once (FR-ID-04, AC-ID-04).
    state.agent_registry().close_by_agent(agent_id);
    Ok(StatusCode::NO_CONTENT)
}

fn parse_agent_id(value: &str) -> Result<AgentId, ApiError> {
    Uuid::parse_str(value)
        .map(AgentId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause("id", Reason::InvalidFormat, error.to_string())
        })
}

fn encode_cursor(cursor: repo::AgentCursor) -> String {
    format!("{};{}", cursor.created_at.unix_timestamp_nanos(), cursor.id)
}

fn decode_cursor(value: &str) -> Result<repo::AgentCursor, String> {
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
    Ok(repo::AgentCursor {
        created_at,
        id: AgentId::from_uuid(id),
    })
}

fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

fn to_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}
