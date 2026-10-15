//! The Restore endpoints: the snapshot list, the tree of a snapshot and the restore itself
//! (FR-RS-01 to FR-RS-04, ADR-0003, ADR-0035).
//!
//! An Operator starts three kinds of job. A `snapshots` job fills the list, an `ls` job reads one
//! directory of a snapshot, and a `restore` job writes the files on the target agent. The row of
//! each job holds the typed request ([`JobRequest`]); the queue builds the step and signs the job
//! (ADR-0010). The data of a restore goes from the repository to the target agent, never through
//! the control plane (ADR-0003). The target agent gets the secrets of this repository for this job
//! only, and each delivery writes a `secret_delivered` audit record (AC-PR-03, ADR-0017).
//!
//! A browse job answers a person who waits, so the API returns at once and the screen asks again
//! for the result. A directory page that a finished job listed lately comes from the stored copy
//! and starts no job (AC-RS-02). A repository of another tenant gives HTTP 404 (AC-TN-01).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use restichub_db::agents as agents_repo;
use restichub_db::audit::{AuditAction, diff};
use restichub_db::jobs::{self as job_repo, Job, NewJob};
use restichub_db::queue as queue_repo;
use restichub_db::repositories as repo;
use restichub_db::restore::{
    self as restore_repo, JobRequest, LsRequest, OverwriteMode, RestoreRequest, SnapshotsRequest,
};
use restichub_db::{AgentId, JobId, JobKind, RepositoryId};
use restichub_proto::AgentCapability;
use restichub_proto::restore_path::{PathFlavor, components, is_within};
use restichub_proto::validate::{MAX_LS_LIMIT, has_forbidden, is_absolute_path};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::{IntoParams, ToSchema};

use crate::browse::{
    BrowseState, NodeItem, SnapshotItem, StoredPage, StoredSnapshots, error_code_of, state_of,
};
use crate::context::{Operation, RequestContext};
use crate::error::{ApiError, map_write_error};
use crate::jobs::{JobView, ensure_agent_active, format_time, parse_job_id, resolve_agent};
use crate::json::ApiJson;
use crate::repositories::parse_repository_id;
use crate::server::AppState;
use crate::validation::Reason;

/// How long a browse job waits in the queue before it expires. A person waits for it.
const BROWSE_LIFETIME: time::Duration = time::Duration::minutes(2);

/// How long a restore waits in the queue for its agent.
const RESTORE_LIFETIME: time::Duration = time::Duration::hours(1);

/// The browse jobs that wait or run on one agent. A further request gets HTTP 429, so a person
/// cannot fill the queue of an agent that a backup needs.
const MAX_PENDING_BROWSE: i64 = 8;

/// A page of a tree that a finished job listed within this time answers a new request.
const PAGE_CACHE: time::Duration = time::Duration::minutes(10);

/// The nodes of a page when the request names no limit (AC-RS-02).
const DEFAULT_PAGE: u64 = 1_000;

const MAX_FILTER_ITEMS: usize = 32;
const MAX_FILTER_CHARS: usize = 1_024;
const MAX_PATTERNS: usize = 256;
const MAX_PATH_CHARS: usize = 4_096;
const MAX_OFFSET: u64 = 100_000_000;

/// Filters of a `snapshots` job. restic applies each of them.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotsQuery {
    /// The agent that reads the repository. An absent value uses the maintenance agent.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Names of the machines whose snapshots stay in the list. Empty keeps all.
    #[serde(default)]
    pub hosts: Vec<String>,
    /// Labels that every kept snapshot carries. Empty keeps all.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Directories that a kept snapshot must have backed up. Empty keeps all.
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Query of a snapshot list read.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct SnapshotsParams {
    /// Identifier that an earlier POST returned. Without it the call returns the newest stored list.
    pub job: Option<String>,
    /// Keep the snapshots made at this time or later, in RFC 3339. The control plane applies it.
    pub from: Option<String>,
    /// Keep the snapshots made at this time or earlier, in RFC 3339. The control plane applies it.
    pub to: Option<String>,
}

/// Result of a snapshot list request, with the state of its job.
#[derive(Debug, Serialize, ToSchema)]
pub struct SnapshotsView {
    pub state: BrowseState,
    /// Null when the state is `empty`.
    pub job_id: Option<String>,
    /// The agent that read the repository. Null when the state is `empty`.
    pub agent_id: Option<String>,
    /// In RFC 3339. Null unless the state is `done`.
    pub fetched_at: Option<String>,
    /// How old the list is, in whole seconds. Null unless the state is `done`.
    pub age_seconds: Option<i64>,
    /// True when the agent dropped the oldest snapshots to fit a result.
    pub truncated: bool,
    /// The filters that the job applied. Empty lists mean no filter.
    pub hosts: Vec<String>,
    pub tags: Vec<String>,
    pub paths: Vec<String>,
    /// A stable code that the user interface translates. Set when the state is `failed`.
    pub error_code: Option<String>,
    pub snapshots: Vec<SnapshotItem>,
}

/// Window of a tree page: one directory, one offset and one limit.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LsQueryRequest {
    /// The agent that reads the repository. An absent value uses the maintenance agent.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The identifier of one snapshot: 8 to 64 lower-case hexadecimal characters. `latest` moves
    /// between two reads, so the call refuses it.
    pub snapshot_id: String,
    /// The directory in the snapshot. Empty or `/` means the root.
    #[serde(default)]
    pub path: String,
    /// The number of entries to skip. Entries are sorted by name as restic prints them.
    #[serde(default)]
    pub offset: u64,
    /// The most entries in the page, up to 5000. Zero means 1000.
    #[serde(default)]
    pub limit: u64,
}

/// Query of a tree page read.
#[derive(Debug, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct LsParams {
    /// Identifier that the POST of the page returned. The call needs it.
    pub job: Option<String>,
}

/// Result of a tree page request, with the state of its job.
#[derive(Debug, Serialize, ToSchema)]
pub struct LsView {
    pub state: BrowseState,
    pub job_id: String,
    pub snapshot_id: String,
    /// The directory. Empty for the root.
    pub path: String,
    pub offset: u64,
    pub limit: u64,
    /// In RFC 3339. Null unless the state is `done`.
    pub fetched_at: Option<String>,
    /// True when a stored page answered the request and no job ran.
    pub cached: bool,
    /// True when the directory has no entry after this page.
    pub last: bool,
    /// Start of the page that follows, to send as `offset` of the next request. Null on the last page.
    pub next_offset: Option<u64>,
    /// A stable code that the user interface translates. Set when the state is `failed`.
    pub error_code: Option<String>,
    pub nodes: Vec<NodeItem>,
}

/// The body of a restore request (FR-RS-03, FR-RS-04).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoreBody {
    /// The agent that writes the files. An absent value uses the maintenance agent. A repository
    /// with a local path needs its maintenance agent, because the data stays on that host.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// The identifier of one snapshot: 8 to 64 lower-case hexadecimal characters. `latest` moves
    /// between two reads, so the call refuses it.
    pub snapshot_id: String,
    /// Restore only the paths that match. Empty means the whole snapshot. restic accepts either
    /// this list or `excludes`, not both.
    #[serde(default)]
    pub includes: Vec<String>,
    /// Skip the paths that match. Not allowed together with `includes`.
    #[serde(default)]
    pub excludes: Vec<String>,
    /// An absolute directory on the agent that writes the files. It must lie inside one of the
    /// roots that the agent reports in its `restore_roots`, and the agent refuses its own files.
    pub target: String,
    /// What to do with a file that exists in the target.
    pub overwrite: OverwriteMode,
    /// Check the restore and write no file. The summary still counts the files.
    #[serde(default)]
    pub dry_run: bool,
    /// Read the restored files again and compare them with the snapshot. restic refuses it with
    /// `dry_run`.
    #[serde(default)]
    pub verify: bool,
}

fn parse_agent(value: Option<&str>) -> Result<Option<AgentId>, ApiError> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) => uuid::Uuid::parse_str(value)
            .map(AgentId::from_uuid)
            .map(Some)
            .map_err(|error| {
                ApiError::invalid_field_cause("agent_id", Reason::InvalidFormat, error.to_string())
            }),
    }
}

/// Validates the values of one filter of a snapshot request. A tag follows the step validation.
fn checked_list(
    field: &'static str,
    values: &[String],
    tags: bool,
) -> Result<Vec<String>, ApiError> {
    if values.len() > MAX_FILTER_ITEMS {
        return Err(ApiError::invalid_field(field, Reason::TooLong));
    }
    for value in values {
        if value.chars().count() > MAX_FILTER_CHARS {
            return Err(ApiError::invalid_field(field, Reason::TooLong));
        }
        let bad_tag = tags && (value.is_empty() || value.contains(',') || value.trim() != value);
        if has_forbidden(value) || bad_tag {
            return Err(ApiError::invalid_field(field, Reason::InvalidFormat));
        }
    }
    Ok(values.to_vec())
}

fn checked_snapshot(value: &str) -> Result<String, ApiError> {
    let valid = matches!(value.len(), 8..=64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if valid {
        Ok(value.to_owned())
    } else {
        Err(ApiError::invalid_field(
            "snapshot_id",
            Reason::InvalidFormat,
        ))
    }
}

fn checked_patterns(field: &'static str, values: &[String]) -> Result<Vec<String>, ApiError> {
    if values.len() > MAX_PATTERNS {
        return Err(ApiError::invalid_field(field, Reason::TooLong));
    }
    for value in values {
        if value.is_empty() {
            return Err(ApiError::invalid_field(field, Reason::Empty));
        }
        if value.chars().count() > MAX_PATH_CHARS {
            return Err(ApiError::invalid_field(field, Reason::TooLong));
        }
        if has_forbidden(value) {
            return Err(ApiError::invalid_field(field, Reason::InvalidCharacter));
        }
    }
    Ok(values.to_vec())
}

fn checked_ls(body: &LsQueryRequest) -> Result<LsRequest, ApiError> {
    let snapshot_id = checked_snapshot(&body.snapshot_id)?;
    let path = body.path.trim();
    if path.chars().count() > MAX_PATH_CHARS {
        return Err(ApiError::invalid_field("path", Reason::TooLong));
    }
    if has_forbidden(path) {
        return Err(ApiError::invalid_field("path", Reason::InvalidCharacter));
    }
    if body.limit > MAX_LS_LIMIT {
        return Err(ApiError::invalid_field("limit", Reason::TooLong));
    }
    if body.offset > MAX_OFFSET {
        return Err(ApiError::invalid_field("offset", Reason::TooLong));
    }
    Ok(LsRequest {
        snapshot_id,
        // The root is the empty path in the request and in the stored copy.
        path: if path == "/" {
            String::new()
        } else {
            path.to_owned()
        },
        offset: body.offset,
        limit: if body.limit == 0 {
            DEFAULT_PAGE
        } else {
            body.limit
        },
    })
}

fn checked_restore(body: &RestoreBody) -> Result<RestoreRequest, ApiError> {
    let snapshot_id = checked_snapshot(&body.snapshot_id)?;
    let target = body.target.trim();
    if target.is_empty() {
        return Err(ApiError::invalid_field("target", Reason::Empty));
    }
    if target.chars().count() > MAX_PATH_CHARS {
        return Err(ApiError::invalid_field("target", Reason::TooLong));
    }
    if has_forbidden(target) || !is_absolute_path(target) {
        return Err(ApiError::invalid_field("target", Reason::InvalidFormat));
    }
    let includes = checked_patterns("includes", &body.includes)?;
    let excludes = checked_patterns("excludes", &body.excludes)?;
    // restic refuses both lists in one restore.
    if !includes.is_empty() && !excludes.is_empty() {
        return Err(ApiError::invalid_field("excludes", Reason::InvalidBody));
    }
    // restic stops with "--dry-run and --verify are mutually exclusive".
    if body.dry_run && body.verify {
        return Err(ApiError::invalid_field("verify", Reason::InvalidBody));
    }
    Ok(RestoreRequest {
        snapshot_id,
        includes,
        excludes,
        target: target.to_owned(),
        overwrite: body.overwrite,
        dry_run: body.dry_run,
        verify: body.verify,
    })
}

/// Creates a queued job with its request for the repository, after the checks of the agent.
///
/// The repository must be initialized and the agent must be active, online and able to run the
/// step kind. The job is not sent here: the dispatcher builds, signs and sends it (ADR-0010).
async fn create_job(
    state: &AppState,
    context: &RequestContext,
    repository_id: RepositoryId,
    explicit_agent: Option<AgentId>,
    kind: JobKind,
    request: &JobRequest,
    check_cache: Option<&LsRequest>,
) -> Result<Created, ApiError> {
    let now = state.auth().clock.now();
    let job_id = JobId::new();
    let created_by = crate::secrets::actor_user(&context.actor);
    let lifetime = if kind == JobKind::Restore {
        RESTORE_LIFETIME
    } else {
        BROWSE_LIFETIME
    };

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(repository) = repo::get_for_update(&mut tx, repository_id).await? else {
        return Err(ApiError::not_found());
    };
    if !repository.initialized {
        return Err(ApiError::conflict());
    }
    if let Some(page) = check_cache
        && let Some(listing) =
            restore_repo::cached_page(&mut tx, repository_id, page, now - PAGE_CACHE).await?
    {
        tx.commit().await?;
        return Ok(Created::Cached(listing));
    }
    let agent_id = resolve_agent(&repository, explicit_agent, kind)?;
    ensure_agent_active(&mut tx, agent_id).await?;
    require_ready_agent(state, agent_id, kind)?;
    if let JobRequest::Restore(restore) = request {
        let os = agents_repo::get(&mut tx, agent_id)
            .await?
            .map(|agent| agent.os)
            .unwrap_or_default();
        check_restore_target(state, agent_id, &os, &restore.target)?;
    } else {
        check_browse_rate(state, context)?;
        restore_repo::lock_agent(&mut tx, agent_id).await?;
        if let Some(pending) = restore_repo::find_pending(
            &mut tx,
            kind.as_str(),
            agent_id,
            repository_id,
            request,
            now,
        )
        .await?
            && let Some(detail) = queue_repo::get_detail(&mut tx, pending).await?
        {
            tx.commit().await?;
            return Ok(Created::Job(Box::new(detail)));
        }
        if restore_repo::count_pending_browse(&mut tx, agent_id, now).await? >= MAX_PENDING_BROWSE {
            return Err(ApiError::rate_limited(5));
        }
    }

    job_repo::create(
        &mut tx,
        &NewJob {
            id: job_id,
            kind,
            agent_id,
            repository_id: Some(repository_id),
            plan_id: None,
            fire_time: None,
            expires_at: Some(now + lifetime),
            location_fingerprint: None,
            created_by,
        },
    )
    .await
    .map_err(map_write_error)?;
    restore_repo::insert_request(&mut tx, job_id, request).await?;
    if !matches!(request, JobRequest::Restore(_)) {
        // The listing sends the repository keys to this agent, so the record names it.
        let mut event = context.audit_event(AuditAction::BrowseStart, "job", job_id.as_uuid());
        event.diff = Some(diff(
            None,
            Some(&json!({
                "repository_id": repository_id.to_string(),
                "agent_id": agent_id.to_string(),
                "kind": kind.as_str(),
                "explicit_agent": explicit_agent.is_some(),
            })),
            &[],
        ));
        restichub_db::audit::insert(&mut tx, &event).await?;
    }
    if let JobRequest::Restore(restore) = request {
        let mut event = context.audit_event(AuditAction::RestoreStart, "job", job_id.as_uuid());
        event.diff = Some(diff(
            None,
            Some(&json!({
                "repository_id": repository_id.to_string(),
                "agent_id": agent_id.to_string(),
                "snapshot_id": restore.snapshot_id,
                "target": restore.target,
                "includes": restore.includes.len(),
                "excludes": restore.excludes.len(),
                "overwrite": restore.overwrite,
                "dry_run": restore.dry_run,
                "verify": restore.verify,
            })),
            &[],
        ));
        restichub_db::audit::insert(&mut tx, &event).await?;
    }
    let detail = queue_repo::get_detail(&mut tx, job_id).await?;
    tx.commit().await?;
    state.wake_dispatcher();
    state
        .events()
        .job_status(context.tenant_id, job_id, restichub_db::JobStatus::Queued);
    let Some(detail) = detail else {
        return Err(ApiError::internal("the job is not found after the create"));
    };
    Ok(Created::Job(Box::new(detail)))
}

/// Refuses a restore target that the agent would refuse, before a job exists (FR-RS-03).
///
/// The roots come from the live connection. The agent checks the target again, resolves links and
/// protects its own files, so this check is only the early answer.
fn check_restore_target(
    state: &AppState,
    agent: AgentId,
    os: &str,
    target: &str,
) -> Result<(), ApiError> {
    let flavor = PathFlavor::from_os(os);
    if components(flavor, target).is_none() {
        return Err(ApiError::invalid_field("target", Reason::InvalidFormat));
    }
    let Some(roots) = state.agent_registry().restore_roots(agent) else {
        return Err(ApiError::agent_offline());
    };
    if roots.iter().any(|root| is_within(flavor, target, root)) {
        Ok(())
    } else {
        Err(ApiError::restore_target_denied())
    }
}

/// Limits the snapshot list and tree page requests of one actor (ADR-0035).
fn check_browse_rate(state: &AppState, context: &RequestContext) -> Result<(), ApiError> {
    let actor = match &context.actor {
        restichub_db::audit::Actor::User(id) => format!("user:{id}"),
        restichub_db::audit::Actor::ApiToken(id) => format!("api_token:{id}"),
        other => other.actor_type().to_owned(),
    };
    let key = format!("browse:{}:{actor}", context.tenant_id);
    match state
        .auth()
        .browse_limiter
        .check(&key, std::time::Instant::now())
    {
        Ok(()) => Ok(()),
        Err(wait) => Err(ApiError::rate_limited(wait.as_secs().max(1))),
    }
}

enum Created {
    Job(Box<queue_repo::JobDetail>),
    Cached(restore_repo::Listing),
}

/// Refuses an agent that is offline, or that is too old for the step kind (FR-PR-09).
///
/// The dispatcher checks the capability again at the send, because the agent can reconnect with
/// another version in between.
fn require_ready_agent(state: &AppState, agent: AgentId, kind: JobKind) -> Result<(), ApiError> {
    let Some(capabilities) = state.agent_registry().capabilities(agent) else {
        return Err(ApiError::agent_offline());
    };
    let needs_restore = matches!(kind, JobKind::Ls | JobKind::Restore);
    if needs_restore && !capabilities.contains(&AgentCapability::RestoreV1) {
        return Err(ApiError::agent_capability_missing());
    }
    Ok(())
}

fn rfc3339(value: &str, field: &'static str) -> Result<OffsetDateTime, ApiError> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|error| {
        ApiError::invalid_field_cause(field, Reason::InvalidFormat, error.to_string())
    })
}

fn within(item: &SnapshotItem, from: Option<OffsetDateTime>, to: Option<OffsetDateTime>) -> bool {
    if from.is_none() && to.is_none() {
        return true;
    }
    let Some(time) = item
        .time
        .as_deref()
        .and_then(|text| OffsetDateTime::parse(text, &Rfc3339).ok())
    else {
        return false;
    };
    from.is_none_or(|from| time >= from) && to.is_none_or(|to| time <= to)
}

fn snapshots_view(
    job: Option<&Job>,
    request: Option<&SnapshotsRequest>,
    listing: Option<&restore_repo::Listing>,
    window: (Option<OffsetDateTime>, Option<OffsetDateTime>),
    now: OffsetDateTime,
) -> Result<SnapshotsView, ApiError> {
    let stored = listing
        .map(|listing| serde_json::from_value::<StoredSnapshots>(listing.payload.clone()))
        .transpose()
        .map_err(|error| ApiError::internal(format!("the stored list is not valid: {error}")))?;
    let state = match (job, &stored) {
        (Some(job), _) => state_of(job, stored.is_some()),
        (None, Some(_)) => BrowseState::Done,
        (None, None) => BrowseState::Empty,
    };
    let done = state == BrowseState::Done;
    let (truncated, snapshots) = stored.map_or((false, Vec::new()), |stored| {
        let kept = stored
            .snapshots
            .into_iter()
            .filter(|item| within(item, window.0, window.1))
            .collect();
        (stored.truncated, kept)
    });
    let filters = request.cloned().unwrap_or_default();
    Ok(SnapshotsView {
        state,
        job_id: listing
            .map(|listing| listing.job_id.to_string())
            .or_else(|| job.map(|job| job.id.to_string())),
        agent_id: job.map(|job| job.agent_id.to_string()),
        fetched_at: listing.filter(|_| done).map(|l| format_time(l.created_at)),
        age_seconds: listing
            .filter(|_| done)
            .map(|l| (now - l.created_at).whole_seconds().max(0)),
        truncated,
        hosts: filters.hosts,
        tags: filters.tags,
        paths: filters.paths,
        error_code: job
            .filter(|_| state == BrowseState::Failed)
            .and_then(error_code_of),
        snapshots,
    })
}

fn ls_view(
    job: Option<&Job>,
    request: &LsRequest,
    listing: Option<&restore_repo::Listing>,
    cached: bool,
) -> Result<LsView, ApiError> {
    let stored = listing
        .map(|listing| serde_json::from_value::<StoredPage>(listing.payload.clone()))
        .transpose()
        .map_err(|error| ApiError::internal(format!("the stored page is not valid: {error}")))?;
    let state = match (job, &stored) {
        (Some(job), _) => state_of(job, stored.is_some()),
        (None, Some(_)) => BrowseState::Done,
        (None, None) => BrowseState::Pending,
    };
    let (last, nodes) = stored.map_or((false, Vec::new()), |stored| (stored.last, stored.nodes));
    let next_offset = (!last && state == BrowseState::Done)
        .then(|| request.offset + u64::try_from(nodes.len()).unwrap_or(0));
    Ok(LsView {
        state,
        job_id: listing
            .map(|listing| listing.job_id.to_string())
            .or_else(|| job.map(|job| job.id.to_string()))
            .unwrap_or_default(),
        snapshot_id: request.snapshot_id.clone(),
        path: request.path.clone(),
        offset: request.offset,
        limit: request.limit,
        fetched_at: listing.map(|listing| format_time(listing.created_at)),
        cached,
        last,
        next_offset,
        error_code: job
            .filter(|_| state == BrowseState::Failed)
            .and_then(error_code_of),
        nodes,
    })
}

/// Starts a job that lists the snapshots of a repository (FR-RS-01).
#[utoipa::path(
    post,
    path = "/api/v1/repositories/{id}/snapshots",
    summary = "Lists the snapshots of a repository.",
    description = "An agent with access to the repository runs `restic snapshots` with the filters. Ask `GET /api/v1/repositories/{id}/snapshots?job=` for the result. An offline agent gives HTTP 409 `agent_offline`, and an agent without the capability gives HTTP 409 `agent_capability_missing`.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository.")),
    request_body = SnapshotsQuery,
    responses(
        (status = 202, description = "The job is queued.", body = SnapshotsView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The repository is not initialized, or the agent is offline.", body = crate::error::ProblemBody)
    )
)]
pub async fn start_snapshots(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<SnapshotsQuery>,
) -> Result<(StatusCode, Json<SnapshotsView>), ApiError> {
    context.authorize(Operation::JobsWrite)?;
    let repository_id = parse_repository_id(&id)?;
    let agent = parse_agent(body.agent_id.as_deref())?;
    let request = SnapshotsRequest {
        hosts: checked_list("hosts", &body.hosts, false)?,
        tags: checked_list("tags", &body.tags, true)?,
        paths: checked_list("paths", &body.paths, false)?,
    };
    let created = create_job(
        &state,
        &context,
        repository_id,
        agent,
        JobKind::Snapshots,
        &JobRequest::Snapshots(request.clone()),
        None,
    )
    .await?;
    let Created::Job(detail) = created else {
        return Err(ApiError::internal("a snapshot list has no cache"));
    };
    let view = snapshots_view(
        Some(&detail.job),
        Some(&request),
        None,
        (None, None),
        state.auth().clock.now(),
    )?;
    Ok((StatusCode::ACCEPTED, Json(view)))
}

/// Reads the snapshot list of a job, or the newest stored list (FR-RS-01).
#[utoipa::path(
    get,
    path = "/api/v1/repositories/{id}/snapshots",
    summary = "Reads a snapshot list.",
    description = "With `job` the call returns the state and the result of that job. Without it the call returns the newest stored list of the repository with its age. The dates `from` and `to` filter the list.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository."), SnapshotsParams),
    responses(
        (status = 200, description = "The snapshot list.", body = SnapshotsView),
        (status = 400, description = "A query parameter is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository or the job is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_snapshots(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    Query(params): Query<SnapshotsParams>,
) -> Result<Json<SnapshotsView>, ApiError> {
    context.authorize(Operation::JobsRead)?;
    let repository_id = parse_repository_id(&id)?;
    let from = params
        .from
        .as_deref()
        .map(|value| rfc3339(value, "from"))
        .transpose()?;
    let to = params
        .to
        .as_deref()
        .map(|value| rfc3339(value, "to"))
        .transpose()?;
    let job_id = params.job.as_deref().map(parse_job_id).transpose()?;
    let now = state.auth().clock.now();
    let view = context
        .tenant_tx(&state, async |tx| {
            if repo::get(tx, repository_id).await?.is_none() {
                return Err(ApiError::not_found());
            }
            let (job, listing) = match job_id {
                Some(job_id) => {
                    job_repo::mark_expired(tx, job_id, now).await?;
                    let Some(job) = job_repo::get(tx, job_id).await? else {
                        return Err(ApiError::not_found());
                    };
                    if job.repository_id != Some(repository_id) || job.kind != JobKind::Snapshots {
                        return Err(ApiError::not_found());
                    }
                    let listing = restore_repo::get_listing(tx, job_id).await?;
                    (Some(job), listing)
                }
                None => {
                    let listing = restore_repo::latest_snapshots(tx, repository_id).await?;
                    let job = match &listing {
                        Some(listing) => job_repo::get(tx, listing.job_id).await?,
                        None => None,
                    };
                    (job, listing)
                }
            };
            let request = match &job {
                Some(job) => match restore_repo::get_request(tx, job.id).await? {
                    Some(JobRequest::Snapshots(request)) => Some(request),
                    _ => None,
                },
                None => None,
            };
            snapshots_view(
                job.as_ref(),
                request.as_ref(),
                listing.as_ref(),
                (from, to),
                now,
            )
        })
        .await?;
    Ok(Json(view))
}

/// Starts a job that lists one directory of a snapshot (FR-RS-02).
#[utoipa::path(
    post,
    path = "/api/v1/repositories/{id}/ls",
    summary = "Lists one directory of a snapshot.",
    description = "An agent with access to the repository runs `restic ls` for one directory and returns one page of its entries. A page that a job listed in the last ten minutes comes from the stored copy with HTTP 200 and starts no job. Ask `GET /api/v1/repositories/{id}/ls?job=` for the result of a new job.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository.")),
    request_body = LsQueryRequest,
    responses(
        (status = 200, description = "A stored page answers the request.", body = LsView),
        (status = 202, description = "The job is queued.", body = LsView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The repository is not initialized, the agent is offline, or the agent is too old.", body = crate::error::ProblemBody)
    )
)]
pub async fn start_ls(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<LsQueryRequest>,
) -> Result<(StatusCode, Json<LsView>), ApiError> {
    context.authorize(Operation::JobsWrite)?;
    let repository_id = parse_repository_id(&id)?;
    let agent = parse_agent(body.agent_id.as_deref())?;
    let request = checked_ls(&body)?;
    let created = create_job(
        &state,
        &context,
        repository_id,
        agent,
        JobKind::Ls,
        &JobRequest::Ls(request.clone()),
        Some(&request),
    )
    .await?;
    match created {
        Created::Cached(listing) => {
            let view = ls_view(None, &request, Some(&listing), true)?;
            Ok((StatusCode::OK, Json(view)))
        }
        Created::Job(detail) => {
            let view = ls_view(Some(&detail.job), &request, None, false)?;
            Ok((StatusCode::ACCEPTED, Json(view)))
        }
    }
}

/// Reads the result of a tree page job (FR-RS-02).
#[utoipa::path(
    get,
    path = "/api/v1/repositories/{id}/ls",
    summary = "Reads a page of a snapshot tree.",
    description = "The call returns the state and the entries of the page job. Ask again while the state is `pending`.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository."), LsParams),
    responses(
        (status = 200, description = "The page.", body = LsView),
        (status = 400, description = "A query parameter is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository or the job is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_ls(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    Query(params): Query<LsParams>,
) -> Result<Json<LsView>, ApiError> {
    context.authorize(Operation::JobsWrite)?;
    let repository_id = parse_repository_id(&id)?;
    let now = state.auth().clock.now();
    let view = context
        .tenant_tx(&state, async |tx| {
            if repo::get(tx, repository_id).await?.is_none() {
                return Err(ApiError::not_found());
            }
            let job = params
                .job
                .as_deref()
                .ok_or_else(|| ApiError::invalid_field("job", Reason::Empty))?;
            let job_id = parse_job_id(job)?;
            job_repo::mark_expired(tx, job_id, now).await?;
            let Some(job) = job_repo::get(tx, job_id).await? else {
                return Err(ApiError::not_found());
            };
            if job.repository_id != Some(repository_id) || job.kind != JobKind::Ls {
                return Err(ApiError::not_found());
            }
            let Some(JobRequest::Ls(request)) = restore_repo::get_request(tx, job_id).await? else {
                return Err(ApiError::not_found());
            };
            let listing = restore_repo::get_listing(tx, job_id).await?;
            ls_view(Some(&job), &request, listing.as_ref(), false)
        })
        .await?;
    Ok(Json(view))
}

/// Starts a restore of a snapshot on a selected agent (FR-RS-03, FR-RS-04).
#[utoipa::path(
    post,
    path = "/api/v1/repositories/{id}/restore",
    summary = "Restores a snapshot.",
    description = "The target agent reads the repository directly and writes the files to the target directory. The data does not go through the control plane. The agent gets the secrets of this repository for this job only. An offline agent gives HTTP 409 `agent_offline`, and an agent without the capability `restore-v1` gives HTTP 409 `agent_capability_missing`.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository.")),
    request_body = RestoreBody,
    responses(
        (status = 202, description = "The job is queued.", body = JobView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The repository is not initialized, the agent is offline, or the agent is too old.", body = crate::error::ProblemBody)
    )
)]
pub async fn start_restore(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RestoreBody>,
) -> Result<(StatusCode, Json<JobView>), ApiError> {
    context.authorize(Operation::JobsWrite)?;
    let repository_id = parse_repository_id(&id)?;
    let agent = parse_agent(body.agent_id.as_deref())?;
    let request = checked_restore(&body)?;
    let created = create_job(
        &state,
        &context,
        repository_id,
        agent,
        JobKind::Restore,
        &JobRequest::Restore(request),
        None,
    )
    .await?;
    let Created::Job(detail) = created else {
        return Err(ApiError::internal("a restore has no cache"));
    };
    Ok((StatusCode::ACCEPTED, Json(JobView::from(&*detail))))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn restore(target: &str) -> RestoreBody {
        RestoreBody {
            agent_id: None,
            snapshot_id: "0123abcd".to_owned(),
            includes: Vec::new(),
            excludes: Vec::new(),
            target: target.to_owned(),
            overwrite: OverwriteMode::Never,
            dry_run: false,
            verify: false,
        }
    }

    #[test]
    fn fr_rs_04_a_restore_target_is_an_absolute_path() {
        assert!(checked_restore(&restore("/srv/restore")).is_ok());
        assert!(checked_restore(&restore("C:\\Restore")).is_ok());
        for bad in ["", "restore", "../etc", "-x", "/srv/\nrestore"] {
            assert!(checked_restore(&restore(bad)).is_err(), "{bad:?}");
        }
        let mut both = restore("/srv/restore");
        both.includes = vec!["/srv".to_owned()];
        both.excludes = vec!["*.log".to_owned()];
        assert!(checked_restore(&both).is_err(), "restic refuses both lists");
        let mut both_modes = restore("/srv/restore");
        both_modes.dry_run = true;
        both_modes.verify = true;
        assert!(
            checked_restore(&both_modes).is_err(),
            "restic refuses a dry run with verify"
        );
    }

    #[test]
    fn fr_pr_10_a_snapshot_identifier_is_hexadecimal_and_never_latest() {
        for good in ["0123abcd", &"a".repeat(64)] {
            assert!(checked_snapshot(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "latest",
            "0123abcd:/etc",
            "--dry-run",
            "ABCDEF12",
            "0123abc",
            &"a".repeat(65),
            "latest ",
        ] {
            assert!(checked_snapshot(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn ac_rs_02_a_page_names_a_bounded_window() {
        let body = |offset, limit| LsQueryRequest {
            agent_id: None,
            snapshot_id: "0123abcd".to_owned(),
            path: "/".to_owned(),
            offset,
            limit,
        };
        let request = checked_ls(&body(0, 0)).expect("a default page");
        assert_eq!(request.limit, DEFAULT_PAGE);
        assert_eq!(request.path, "", "the root is the empty path");
        assert!(checked_ls(&body(0, MAX_LS_LIMIT + 1)).is_err());
        assert!(checked_ls(&body(MAX_OFFSET + 1, 10)).is_err());
    }

    #[test]
    fn ac_rs_01_a_date_window_keeps_the_snapshots_inside_it() {
        let item = |time: Option<&str>| SnapshotItem {
            id: "a".repeat(64),
            short_id: "aaaaaaaa".to_owned(),
            time: time.map(str::to_owned),
            hostname: String::new(),
            username: String::new(),
            tags: Vec::new(),
            paths: Vec::new(),
            program_version: String::new(),
            total_files: None,
            total_bytes: None,
        };
        let from = rfc3339("2026-01-02T00:00:00Z", "from").ok();
        let to = rfc3339("2026-01-03T00:00:00Z", "to").ok();
        assert!(within(&item(Some("2026-01-02T12:00:00Z")), from, to));
        assert!(!within(&item(Some("2026-01-01T12:00:00Z")), from, to));
        assert!(!within(&item(Some("2026-01-04T12:00:00Z")), from, to));
        assert!(!within(&item(None), from, to));
        assert!(within(&item(None), None, None));
    }
}
