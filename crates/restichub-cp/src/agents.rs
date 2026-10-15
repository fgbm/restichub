//! The agent registry endpoints (ADR-0026, FR-ID-04, FR-TN-01, FR-TN-03).
//!
//! An Admin lists, reads, renames and revokes the agents of its tenant. A Viewer reads. The
//! platform tenant gets HTTP 403 (the endpoints are tenant-only). An agent of another tenant gives
//! HTTP 404 (AC-TN-01).
//!
//! The revoke writes the status and the audit record in one transaction. The instance that serves
//! the revoke cancels the running jobs and closes the stream; the other instances close theirs with
//! the revocation sweep (AC-ID-04).

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

/// Stores the facts of the last hello of an agent for the diagnostics, the Hosts screen, the
/// Restore screen (FR-PR-09, FR-RS-03). The job builder never reads
/// the capabilities: it uses the capabilities of the live connection.
pub(crate) async fn record_hello_facts(
    state: &AppState,
    tenant: restichub_db::TenantId,
    agent: AgentId,
    hello: &restichub_proto::agent::v1::AgentHello,
    capabilities: &[restichub_proto::AgentCapability],
    restore_roots: &[String],
    scripts: &crate::scripts::HelloScripts,
) {
    let names: Vec<String> = capabilities
        .iter()
        .map(|capability| capability.as_str().to_owned())
        .collect();
    let agent_version = clean_version(&hello.agent_version);
    let restic_version = clean_version(&hello.restic_version);
    let result = async {
        let mut tx = state.db().begin_tenant(tenant).await?;
        let facts = repo::HelloFacts {
            capabilities: &names,
            restore_roots,
            agent_version: &agent_version,
            restic_version: &restic_version,
            protocol_version_major: hello.protocol_version_major,
            protocol_version_minor: hello.protocol_version_minor,
            server_scripts: scripts.state,
            script_interpreters: &scripts.interpreters,
        };
        repo::set_hello_facts(&mut tx, agent, &facts).await?;
        tx.commit().await
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(agent = %agent, cause = ?error, "the agent hello facts were not stored");
    }
}

/// The longest version text that the registry keeps.
const MAX_VERSION_CHARS: usize = 64;

/// Bounds a version text that an agent reported. A value with a control character is dropped.
fn clean_version(value: &str) -> String {
    if value.chars().count() > MAX_VERSION_CHARS || restichub_proto::validate::has_forbidden(value)
    {
        return String::new();
    }
    value.to_owned()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentView {
    pub id: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub agent_version: String,
    pub restic_version: String,
    pub labels: Vec<String>,
    /// Absolute directories where the agent restores files, from its last connection. A restore
    /// target must lie inside one of them.
    pub restore_roots: Vec<String>,
    /// Whether the host trusts scripts from the control plane (FR-HK-08, ADR-0037). The host
    /// administrator changes it with `restichub-agent trust-server-scripts`.
    pub server_scripts: restichub_db::ServerScriptsState,
    /// The interpreters the agent found, for example `bash`. Empty unless the mode is enabled.
    pub script_interpreters: Vec<String>,
    /// Lower-case hexadecimal.
    pub certificate_serial: String,
    /// In RFC 3339.
    pub certificate_expires_at: String,
    /// In RFC 3339. Null until the first rotation of the certificate.
    pub last_certificate_rotation_at: Option<String>,
    /// In RFC 3339. Null unless a connection presented the certificate that the last rotation
    /// replaced. It can mean a copy of the old key (T17).
    pub superseded_certificate_used_at: Option<String>,
    /// Reported in the last hello as `major.minor`. Null before the first hello.
    pub protocol_version: Option<String>,
    /// The package version of the control plane that serves the request.
    pub control_plane_version: String,
    pub version_status: crate::agent_version::AgentVersionStatus,
    pub status: AgentStatus,
    pub last_seen_at: Option<String>,
    /// The time of the first stream open. Null while an active host waits for its first connection.
    pub first_connected_at: Option<String>,
    pub created_at: String,
    pub revoked_at: Option<String>,
    pub revoked_by_user_id: Option<String>,
    /// Another active host of the tenant that reported the same machine identifier.
    pub possible_duplicate_of: Option<DuplicateOfView>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DuplicateOfView {
    pub id: String,
    pub hostname: String,
    pub created_at: String,
    pub last_seen_at: Option<String>,
    /// True when the other record was created before this one. Only the newer record offers the
    /// replacement of the older one (FR-ID-11).
    pub is_older: bool,
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
            restore_roots: agent.restore_roots.clone(),
            server_scripts: agent.server_scripts,
            script_interpreters: agent.script_interpreters.clone(),
            certificate_serial: to_hex(&agent.certificate_serial),
            certificate_expires_at: format_time(agent.certificate_expires_at),
            last_certificate_rotation_at: agent.last_certificate_rotation_at.map(format_time),
            superseded_certificate_used_at: agent.superseded_certificate_used_at.map(format_time),
            protocol_version: agent
                .protocol_version_major
                .zip(agent.protocol_version_minor)
                .map(|(major, minor)| format!("{major}.{minor}")),
            control_plane_version: crate::agent_version::CONTROL_PLANE_VERSION.to_owned(),
            version_status: crate::agent_version::status(
                &agent.agent_version,
                agent
                    .protocol_version_major
                    .zip(agent.protocol_version_minor),
            ),
            status: agent.status,
            last_seen_at: agent.last_seen_at.map(format_time),
            first_connected_at: agent.first_connected_at.map(format_time),
            created_at: format_time(agent.created_at),
            revoked_at: agent.revoked_at.map(format_time),
            revoked_by_user_id: agent.revoked_by_user_id.map(|id| id.to_string()),
            possible_duplicate_of: agent.possible_duplicate_of.as_ref().map(|other| {
                DuplicateOfView {
                    id: other.id.to_string(),
                    hostname: other.hostname.clone(),
                    created_at: format_time(other.created_at),
                    last_seen_at: other.last_seen_at.map(format_time),
                    is_older: other.created_at < agent.created_at,
                }
            }),
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
    let jobs = restichub_db::queue::active_job_ids_for_agent(&mut tx, agent_id).await?;
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
    // A revoke cancels the running jobs and closes an open stream at once (FR-ID-04, AC-ID-04). A
    // stream on another instance closes with the revocation sweep of that instance.
    crate::agent_stream::cancel_jobs_and_close(state.agent_registry(), agent_id, &jobs);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReenrollAgentRequest {
    /// The time to live in seconds. Between 60 and 3600. The default is 3600.
    pub ttl_seconds: Option<i64>,
}

/// Issues an enrollment token bound to an existing host (FR-ID-01).
///
/// The enrollment with the token keeps the agent identifier, the plans, the repositories and the
/// labels of the host and replaces its certificate. A revoked host gives HTTP 409.
#[utoipa::path(
    post,
    path = "/api/v1/agents/{id}/reenroll",
    summary = "Issues an enrollment token that re-registers an existing host.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the agent.")),
    request_body = ReenrollAgentRequest,
    responses(
        (status = 201, description = "The token is created. The value appears once.", body = crate::enrollment_tokens::CreateEnrollmentTokenResponse),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The agent is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The agent is revoked.", body = crate::error::ProblemBody)
    )
)]
pub async fn reenroll_agent(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ReenrollAgentRequest>,
) -> Result<
    (
        StatusCode,
        Json<crate::enrollment_tokens::CreateEnrollmentTokenResponse>,
    ),
    ApiError,
> {
    context.authorize(Operation::AgentsWrite)?;
    let agent_id = parse_agent_id(&id)?;
    let ttl = crate::enrollment_tokens::validate_bound_ttl(body.ttl_seconds)?;
    let agent = context
        .tenant_tx(&state, async |tx| Ok(repo::get(tx, agent_id).await?))
        .await?
        .ok_or_else(ApiError::not_found)?;
    if agent.status != AgentStatus::Active {
        return Err(ApiError::conflict());
    }
    let response =
        crate::enrollment_tokens::issue_token(&state, &context, Vec::new(), ttl, Some(agent_id))
            .await?;
    Ok((StatusCode::CREATED, Json(response)))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceAgentRequest {
    /// The identifier of the old record. It must be the possible duplicate of this host.
    pub old_agent_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReplaceAgentResponse {
    pub plans_moved: u64,
    pub repositories_moved: u64,
    pub queued_jobs_moved: u64,
}

/// Moves the plans, maintenance host references and labels of an old duplicate record to this host
/// and revokes the old record (FR-ID-01).
///
/// Both records must be active and must report the same machine identifier. This record must be
/// newer than the old one, and the old one must have been silent for the online window, otherwise
/// the answer is HTTP 404 or HTTP 409 `agent_recently_online`. The move and the revoke commit in
/// one transaction; the running jobs of the old record are cancelled as for a revoke (FR-ID-04).
#[utoipa::path(
    post,
    path = "/api/v1/agents/{id}/replace",
    summary = "Replaces the old duplicate record of a host.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the new agent record.")),
    request_body = ReplaceAgentRequest,
    responses(
        (status = 200, description = "The old record is replaced.", body = ReplaceAgentResponse),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The pair is not a possible duplicate.", body = crate::error::ProblemBody),
        (status = 409, description = "The old record was online recently (`agent_recently_online`).", body = crate::error::ProblemBody)
    )
)]
pub async fn replace_agent(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ReplaceAgentRequest>,
) -> Result<Json<ReplaceAgentResponse>, ApiError> {
    context.authorize(Operation::AgentsWrite)?;
    let new_id = parse_agent_id(&id)?;
    let old_id = Uuid::parse_str(&body.old_agent_id)
        .map(AgentId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause("old_agent_id", Reason::InvalidFormat, error.to_string())
        })?;
    let actor = context.actor_user(&state).await?;
    let now = state.auth().clock.now();
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let moved = match repo::replace(
        &mut tx,
        new_id,
        old_id,
        actor,
        now,
        crate::reenroll_limits::replace_online_window(),
    )
    .await?
    {
        repo::ReplaceOutcome::Replaced(moved) => moved,
        repo::ReplaceOutcome::NotPair => return Err(ApiError::not_found()),
        repo::ReplaceOutcome::OldRecentlyOnline => return Err(ApiError::agent_recently_online()),
    };
    let mut revoke_event = context.audit_event(AuditAction::AgentRevoke, "agent", old_id.as_uuid());
    revoke_event.diff = Some(diff(
        Some(&serde_json::json!({"status": AgentStatus::Active.as_str()})),
        Some(&serde_json::json!({
            "status": AgentStatus::Revoked.as_str(),
            "reason": "replaced",
            "replaced_by_agent_id": new_id.to_string(),
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &revoke_event).await?;
    let mut event = context.audit_event(AuditAction::AgentReplace, "agent", new_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({
            "new_agent_id": new_id.to_string(),
            "old_agent_id": old_id.to_string(),
            "plans_moved": moved.plans,
            "repositories_moved": moved.repositories,
            "queued_jobs_moved": moved.queued_jobs,
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    crate::agent_stream::cancel_jobs_and_close(state.agent_registry(), old_id, &moved.active_jobs);
    Ok(Json(ReplaceAgentResponse {
        plans_moved: moved.plans,
        repositories_moved: moved.repositories,
        queued_jobs_moved: moved.queued_jobs,
    }))
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
