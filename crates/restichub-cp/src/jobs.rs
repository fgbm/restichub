//! The job model and the on-demand job dispatcher (FR-PR-01, FR-PR-03, FR-PR-05, FR-RP-02,
//! FR-RP-08, ADR-0003).
//!
//! An Admin initializes a repository or tests its access from the web API. The control plane builds
//! one job, validates it ([`ValidatedJob`], FR-PR-10), signs it with the current job signing key
//! (FR-PR-03, ADR-0030) and sends it to the agent over the open stream (FR-PR-05). The agent
//! receives the repository password and the backend credentials inside the signed bytes only
//! (ADR-0003).
//!
//! The row in the `jobs` table holds no secret. The dispatcher reads each secret with
//! [`SecretReader`], which writes the `secret_delivered` audit record in the same transaction
//! (AC-RP-03). The transaction commits before the control plane sends the value to the agent.
//!
//! An on-demand job holds no queue slot: an offline agent gives HTTP 409 `agent_offline` at once.
//! The scheduled jobs go through the queue of [`crate::queue`].
//!
//! The safety rule of an init: the job always holds a `test_access` step before the `init` step, and
//! the agent runs `init` only when `test_access` exited with restic code 10 (the repository does not
//! exist). Any other failure stops the job; a wrong password gives `config_error` and no init
//! (AC-RP-01). An initialized repository is never re-initialized: the control plane refuses the
//! action with HTTP 409.
//!
//! The agent result updates the job row. A successful `init` or a successful `test_access` sets
//! `repositories.initialized = true` (FR-RP-02), but only while the repository location still
//! matches the fingerprint stored with the job. A `not_initialized` result never clears
//! the flag: on an initialized repository it is a location mismatch, so the job fails with the typed
//! access error.
//!
//! One active job for each repository is enforced by a partial unique index ; a second job
//! gives HTTP 409. An overdue job moves to `expired` on read and in a periodic sweep.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_compiler::ValidatedJob;
use restichub_db::audit::{AuditAction, diff};
use restichub_db::jobs::{self as job_repo, Job, JobOutcome, NewJob};
use restichub_db::queue::{self as queue_repo, JobDetail};
use restichub_db::repositories::{self as repo, Repository};
use restichub_db::{
    AgentId, AgentStatus, BackendKind, JobId, JobKind, JobResultCode, JobStatus, RepositoryId,
    SecretId, TenantId, agents as agents_repo,
};
use restichub_proto::agent::v1::{
    CancelJob, ControlMessage, InitStep, Job as ProtoJob, JobAck, JobRejected, JobResult,
    JobStatus as ProtoJobStatus, LocalCredentials, RejectReason, RepositoryAccess, RestCredentials,
    S3Credentials, Step, StepStatus, TestAccessStep, control_message,
    repository_access::BackendCredentials, step,
};
use restichub_secrets::{KekProvider, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::context::{Operation, RequestContext};
use crate::delivery::{DeliveryError, SecretReader};
use crate::error::{ApiError, map_write_error};
use crate::json::ApiJson;
use crate::queue::retry;
use crate::repositories::stored_location;
use crate::server::AppState;
use crate::validation::Reason;

/// How long an on-demand job stays valid after the control plane issues it (ADR-0020).
const JOB_LIFETIME: time::Duration = time::Duration::minutes(30);

/// Timeout of one step of a repository job (FR-AG-05).
const STEP_TIMEOUT_SECONDS: u32 = 600;

/// Timeout of the full job (FR-AG-05).
pub(crate) const JOB_TIMEOUT_SECONDS: u32 = 900;

/// How long a job that a busy agent refused waits before it goes out again.
const BUSY_DELAY: time::Duration = time::Duration::seconds(10);

/// The largest accepted error code from an agent (FR-PR-08).
const MAX_RESULT_MESSAGE_LEN: usize = 64;

/// The stable error code of a location mismatch: a `not_initialized` result on an initialized
/// repository. The user interface translates it (NFR-DOC-04).
const ACCESS_ERROR_CODE: &str = "repository_missing_here";

/// The stable warning code of a `forget` step that selected no snapshot (FR-RP-08).
const FORGET_SELECTED_NOTHING: &str = "forget_selected_nothing";

/// The request body of an init or a test-access action.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct JobActionRequest {
    /// The agent that runs the job. An absent value uses the maintenance agent of the repository
    /// (FR-RP-08). A local-path repository needs an explicit or a maintenance agent.
    #[serde(default)]
    pub agent_id: Option<String>,
}

/// One job with its lifecycle (FR-PR-01).
#[derive(Debug, Serialize, ToSchema)]
pub struct JobView {
    pub id: String,
    pub kind: JobKind,
    /// Null for a job that touches no repository.
    pub repository_id: Option<String>,
    pub agent_id: String,
    pub status: JobStatus,
    /// Null until the job finishes.
    pub result_code: Option<JobResultCode>,
    /// A machine-readable error code from the agent. The user interface translates it (NFR-DOC-04).
    /// It never holds a secret.
    pub result_message: Option<String>,
    /// In RFC 3339.
    pub created_at: String,
    /// In RFC 3339.
    pub updated_at: String,
    /// In RFC 3339. Null until the job finishes.
    pub finished_at: Option<String>,
    /// In RFC 3339. The end of the acceptance window of the signed job: the agent refuses the job
    /// after this time. It is not a limit of the run. Null when the job holds no expiry.
    pub expires_at: Option<String>,
    /// Null for an on-demand job and for a job whose plan was deleted.
    pub plan_id: Option<String>,
    /// Null in a response that joins no display names.
    pub plan_name: Option<String>,
    /// Null in a response that joins no display names.
    pub agent_hostname: Option<String>,
    /// Null in a response that joins no display names.
    pub repository_name: Option<String>,
    /// In RFC 3339. Null until the agent starts the job.
    pub started_at: Option<String>,
    /// In RFC 3339. The earliest time at which the queue sends the job.
    pub run_at: String,
    /// Lock retries so far (restic exit code 11).
    pub attempt: i32,
}

impl From<&JobDetail> for JobView {
    fn from(detail: &JobDetail) -> Self {
        let mut view = Self::from(&detail.job);
        view.plan_name = detail.plan_name.clone();
        view.agent_hostname = detail.agent_hostname.clone();
        view.repository_name = detail.repository_name.clone();
        view
    }
}

impl From<&Job> for JobView {
    fn from(job: &Job) -> Self {
        Self {
            id: job.id.to_string(),
            kind: job.kind,
            repository_id: job.repository_id.map(|id| id.to_string()),
            agent_id: job.agent_id.to_string(),
            status: job.status,
            result_code: job.result_code,
            result_message: job.result_message.clone(),
            created_at: format_time(job.created_at),
            updated_at: format_time(job.updated_at),
            finished_at: job.finished_at.map(format_time),
            expires_at: job.expires_at.map(format_time),
            plan_id: job.plan_id.map(|id| id.to_string()),
            plan_name: None,
            agent_hostname: None,
            repository_name: None,
            started_at: job.started_at.map(format_time),
            run_at: format_time(job.run_at),
            attempt: job.attempt,
        }
    }
}

/// Initializes a repository from a selected agent (FR-RP-02).
///
/// The job runs `test_access` first and `init` after it, so the agent never re-initializes an
/// existing repository. An offline agent gives HTTP 409 `agent_offline`.
#[utoipa::path(
    post,
    path = "/api/v1/repositories/{id}/init",
    summary = "Initializes a repository from an agent.",
    description = "The control plane sends the repository password and the backend credentials with the signed job only (ADR-0003). An offline agent gives HTTP 409 `agent_offline`.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository.")),
    request_body = JobActionRequest,
    responses(
        (status = 202, description = "The job is sent to the agent.", body = JobView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The agent is offline.", body = crate::error::ProblemBody)
    )
)]
pub async fn init_repository(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<JobActionRequest>,
) -> Result<(StatusCode, Json<JobView>), ApiError> {
    context.authorize(Operation::RepositoriesWrite)?;
    let repository_id = crate::repositories::parse_repository_id(&id)?;
    let agent_id = parse_optional_agent_id(body.agent_id.as_deref())?;
    let job =
        create_repository_job(&state, &context, repository_id, agent_id, JobKind::Init).await?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

/// Tests the access to a repository from a selected agent (FR-RP-02).
///
/// The job runs `restic cat config`, which is read-only. A missing repository gives the typed result
/// `not_initialized`, not an error. A wrong password gives `config_error`.
#[utoipa::path(
    post,
    path = "/api/v1/repositories/{id}/test-access",
    summary = "Tests the access to a repository from an agent.",
    description = "The job runs `restic cat config`. A missing repository gives the typed result `not_initialized`. An offline agent gives HTTP 409 `agent_offline`.",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository.")),
    request_body = JobActionRequest,
    responses(
        (status = 202, description = "The job is sent to the agent.", body = JobView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The agent is offline.", body = crate::error::ProblemBody)
    )
)]
pub async fn test_access_repository(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<JobActionRequest>,
) -> Result<(StatusCode, Json<JobView>), ApiError> {
    context.authorize(Operation::RepositoriesWrite)?;
    let repository_id = crate::repositories::parse_repository_id(&id)?;
    let agent_id = parse_optional_agent_id(body.agent_id.as_deref())?;
    let job = create_repository_job(
        &state,
        &context,
        repository_id,
        agent_id,
        JobKind::TestAccess,
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(job)))
}

/// Builds, signs and sends one repository job (FR-PR-03, FR-PR-05).
async fn create_repository_job(
    state: &AppState,
    context: &RequestContext,
    repository_id: RepositoryId,
    explicit_agent: Option<AgentId>,
    kind: JobKind,
) -> Result<JobView, ApiError> {
    let now = state.auth().clock.now();
    let expires_at = now + JOB_LIFETIME;
    let job_id = JobId::new();
    let created_by = crate::secrets::actor_user(&context.actor);

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    // The read locks the row: a concurrent delete cannot remove the repository between the read and
    // the job create. A repository of another tenant gives HTTP 404 (AC-TN-01).
    let Some(repository) = repo::get_for_update(&mut tx, repository_id).await? else {
        return Err(ApiError::not_found());
    };
    // An initialized repository is never re-initialized.
    if kind == JobKind::Init && repository.initialized {
        return Err(ApiError::conflict());
    }
    let agent_id = resolve_agent(&repository, explicit_agent, kind)?;
    ensure_agent_active(&mut tx, agent_id).await?;
    // The MVP holds no queue for an on-demand job. The check comes before the secret read, so an
    // offline agent writes no delivery record.
    if !state.agent_registry().is_online(agent_id) {
        return Err(ApiError::agent_offline());
    }

    let fingerprint = repository_fingerprint(&repository);
    // One active job for each repository. The partial unique index carries the
    // invariant, so a second job gives HTTP 409.
    job_repo::create(
        &mut tx,
        &NewJob {
            id: job_id,
            kind,
            agent_id,
            repository_id: Some(repository_id),
            plan_id: None,
            fire_time: None,
            expires_at: Some(expires_at),
            location_fingerprint: Some(fingerprint),
            created_by,
        },
    )
    .await
    .map_err(map_write_error)?;

    let provider = state
        .kek()
        .ok_or_else(|| ApiError::internal("the server holds no key encryption key provider"))?;
    let access = repository_access(&mut tx, provider, &repository, agent_id, job_id).await?;
    let proto_job = ProtoJob {
        job_id: job_id.to_string(),
        tenant_id: context.tenant_id.to_string(),
        agent_id: agent_id.to_string(),
        issued_at: Some(timestamp(now)),
        expires_at: Some(timestamp(expires_at)),
        timeout_seconds: JOB_TIMEOUT_SECONDS,
        stall_timeout_seconds: 0,
        repositories: vec![access],
        steps: repository_steps(kind, &repository_id.to_string()),
    };
    // The control plane validates before it signs the job (FR-PR-10, ADR-0021).
    let validated = ValidatedJob::new(proto_job).map_err(|rejection| {
        ApiError::internal(format!(
            "the control plane built a job that does not validate: {:?}",
            rejection.reason
        ))
    })?;

    let mut event = context.audit_event(AuditAction::JobRun, "job", job_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&json!({
            "kind": kind.as_str(),
            "repository_id": repository_id.to_string(),
            "agent_id": agent_id.to_string(),
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    restichub_db::job_steps::plan_described(
        &mut tx,
        job_id,
        &planned_steps(&validated.job().steps),
    )
    .await?;
    tx.commit().await?;

    let signing = state
        .job_signing()
        .ok_or_else(|| ApiError::internal("the server holds no job signing key set"))?;
    let signed = match signing.sign_job(state.db(), validated.job()).await {
        Ok(signed) => signed,
        Err(error) => {
            // A signing failure must not leave the job queued forever.
            let mut tx = state.db().begin_tenant(context.tenant_id).await?;
            let _ = job_repo::mark_refused(&mut tx, job_id, "signing_failed", now).await;
            let _ = tx.commit().await;
            return Err(ApiError::from(error));
        }
    };

    let sent = state.agent_registry().send_to_agent(
        agent_id,
        ControlMessage {
            payload: Some(control_message::Payload::SignedJob(signed)),
        },
    );

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    if sent {
        job_repo::mark_sent(
            &mut tx,
            job_id,
            now,
            i32::try_from(JOB_TIMEOUT_SECONDS).unwrap_or(i32::MAX),
        )
        .await?;
    } else {
        job_repo::mark_refused(&mut tx, job_id, "agent_offline", now).await?;
    }
    let job = job_repo::get(&mut tx, job_id).await?;
    tx.commit().await?;

    if !sent {
        return Err(ApiError::agent_offline());
    }
    let Some(job) = job else {
        return Err(ApiError::internal("the job is not found after the send"));
    };
    Ok(JobView::from(&job))
}

/// Records a `JobAck` from the stream: the job moves to `running` (FR-PR-06).
///
/// The identity comes from the certificate, never from the message (FR-PR-08), so a message for
/// another agent changes no row.
pub async fn handle_job_ack(state: &AppState, tenant: TenantId, agent: AgentId, ack: &JobAck) {
    let Some(job_id) = parse_job_id_value(&ack.job_id) else {
        return;
    };
    let now = state.auth().clock.now();
    let Ok(mut tx) = state.db().begin_tenant(tenant).await else {
        tracing::warn!(job = %ack.job_id, "the job acknowledgement could not open a transaction");
        return;
    };
    let job = match job_repo::get_for_update(&mut tx, job_id).await {
        Ok(Some(job)) => job,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(job = %job_id, %error, "the acknowledgement could not read the job");
            return;
        }
    };
    if job.agent_id != agent {
        return;
    }
    if job.status == JobStatus::Sent
        && let Err(error) = job_repo::mark_running(&mut tx, job_id, now).await
    {
        tracing::warn!(job = %job_id, %error, "the acknowledgement could not update the job");
        return;
    }
    if let Err(error) = tx.commit().await {
        tracing::warn!(job = %job_id, %error, "the acknowledgement could not commit");
        return;
    }
    // A cancel that raced the dispatch reached the agent before the job did, so the agent ignored
    // it. The acknowledgement of the flagged job gets the cancel again, also when the job already
    // ended: the agent runs it until it gets the cancel.
    if job.cancel_requested {
        state.agent_registry().send_to_agent(
            agent,
            ControlMessage {
                payload: Some(control_message::Payload::CancelJob(CancelJob {
                    job_id: job_id.to_string(),
                    grace_period_seconds: crate::job_api::CANCEL_GRACE_SECONDS,
                })),
            },
        );
    }
}

/// Records a `JobRejected` from the stream: the job fails with the typed reason (FR-PR-03).
///
/// A rejection frees the repository, so the control plane can start the next job. A rejection that
/// names no job only leaves a log line. The identity
/// comes from the certificate, never from the message (FR-PR-08), so a rejection for another agent
/// or tenant changes no row.
pub async fn handle_job_rejected(
    state: &AppState,
    tenant: TenantId,
    agent: AgentId,
    rejected: &JobRejected,
) {
    let Some(job_id) = parse_job_id_value(&rejected.job_id) else {
        // The agent names no job when it cannot trust the signature, so this is the trace of a
        // forged or damaged job (T3).
        tracing::warn!(
            %agent,
            reason = reject_reason_code(rejected.reason),
            "the agent rejected a job that it could not identify"
        );
        return;
    };
    let now = state.auth().clock.now();
    let Ok(mut tx) = state.db().begin_tenant(tenant).await else {
        tracing::warn!(job = %rejected.job_id, "the job rejection could not open a transaction");
        return;
    };
    let job = match job_repo::get_for_update(&mut tx, job_id).await {
        Ok(Some(job)) => job,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(job = %job_id, %error, "the rejection could not read the job");
            return;
        }
    };
    if job.agent_id != agent || job.status.is_terminal() {
        return;
    }
    // A busy agent only defers the job: it stays `sent` and the control plane delivers it again
    // after a short delay. The refusal is not a delivery (ADR-0020).
    if rejected.reason == RejectReason::Busy as i32 {
        tracing::info!(job = %job_id, "the agent is busy and defers the job");
        if let Err(error) = queue_repo::defer_busy(&mut tx, job_id, now + BUSY_DELAY).await {
            tracing::warn!(job = %job_id, %error, "the busy refusal could not defer the job");
            return;
        }
        if let Err(error) = tx.commit().await {
            tracing::warn!(job = %job_id, %error, "the busy refusal could not commit");
        }
        return;
    }
    let reason = reject_reason_code(rejected.reason);
    tracing::warn!(job = %job_id, reason, "the agent rejected the job");
    let outcome = JobOutcome {
        status: JobStatus::Failed,
        result_code: JobResultCode::Failed,
        result_message: Some(reason.to_owned()),
    };
    if let Err(error) = job_repo::finish(&mut tx, job_id, &outcome, now).await {
        tracing::warn!(job = %job_id, %error, "the rejection could not update the job");
        return;
    }
    if let Err(error) = tx.commit().await {
        tracing::warn!(job = %job_id, %error, "the rejection could not commit");
        return;
    }
    state.wake_dispatcher();
}

/// The stable code of a `RejectReason`. The user interface translates it (NFR-DOC-04).
fn reject_reason_code(reason: i32) -> &'static str {
    match RejectReason::try_from(reason) {
        Ok(RejectReason::BadSignature) => "bad_signature",
        Ok(RejectReason::UnknownKey) => "unknown_key",
        Ok(RejectReason::WrongAgent) => "wrong_agent",
        Ok(RejectReason::WrongTenant) => "wrong_tenant",
        Ok(RejectReason::Expired) => "expired",
        Ok(RejectReason::Replay) => "replay",
        Ok(RejectReason::UnsupportedStep) => "unsupported_step",
        Ok(RejectReason::UnsupportedResticVersion) => "unsupported_restic_version",
        Ok(RejectReason::UnknownHook) => "unknown_hook",
        Ok(RejectReason::InvalidHookParameters) => "invalid_hook_parameters",
        Ok(RejectReason::AllowListInsecure) => "allow_list_insecure",
        Ok(RejectReason::Malformed) => "malformed",
        Ok(RejectReason::LifetimeTooLong) => "lifetime_too_long",
        Ok(RejectReason::NotYetValid) => "not_yet_valid",
        Ok(RejectReason::Busy) => "busy",
        Ok(RejectReason::Internal) => "internal_error",
        Ok(RejectReason::RestoreTargetDenied) => "restore_target_denied",
        Ok(RejectReason::ServerScriptsDisabled) => "server_scripts_disabled",
        Ok(RejectReason::InvalidScript) => "invalid_script",
        Ok(RejectReason::Unspecified) | Err(_) => "rejected",
    }
}

/// The steps of a signed job with their kind, name and phase, for the job detail (FR-HK-11). The
/// name of a script is the name its author gave; no body and no value leaves with it.
pub(crate) fn planned_steps(
    steps: &[restichub_proto::agent::v1::Step],
) -> Vec<restichub_db::job_steps::PlannedStep> {
    use restichub_db::{PlannedKind, PlannedPhase};
    use restichub_proto::agent::v1::{HookPhase, step};
    let phase = |wire: i32| match HookPhase::try_from(wire) {
        Ok(HookPhase::Before) => Some(PlannedPhase::Before),
        Ok(HookPhase::After) => Some(PlannedPhase::After),
        Ok(HookPhase::OnSuccess) => Some(PlannedPhase::OnSuccess),
        Ok(HookPhase::OnError) => Some(PlannedPhase::OnError),
        Ok(HookPhase::Finally) => Some(PlannedPhase::Finally),
        _ => None,
    };
    steps
        .iter()
        .map(|planned| {
            let (kind, name, step_phase, content_sha256) = match planned.kind.as_ref() {
                Some(step::Kind::Backup(_)) => {
                    (Some(PlannedKind::Backup), String::new(), None, None)
                }
                Some(step::Kind::Hook(hook)) => (
                    Some(PlannedKind::Hook),
                    hook.hook_id.clone(),
                    phase(hook.phase),
                    None,
                ),
                Some(step::Kind::Script(script)) => (
                    Some(PlannedKind::Script),
                    script.name.clone(),
                    phase(script.phase),
                    Some(crate::scripts::body_hash(&script.body)),
                ),
                Some(_) => (Some(PlannedKind::Other), String::new(), None, None),
                None => (None, String::new(), None, None),
            };
            restichub_db::job_steps::PlannedStep {
                step_id: planned.step_id.clone(),
                kind,
                name,
                phase: step_phase,
                content_sha256,
            }
        })
        .collect()
}

/// Records a `JobResult` from the stream: it updates the job and the repository state (FR-RP-02).
///
/// A successful `init` or a successful `test_access` sets `repositories.initialized = true`, but
/// only while the repository location still matches the job. A `not_initialized` result
/// never clears the flag : on an initialized repository it is a location mismatch, so the
/// job fails with the typed access error. The identity comes from the certificate (FR-PR-08).
pub async fn handle_job_result(
    state: &AppState,
    tenant: TenantId,
    agent: AgentId,
    result: &JobResult,
) {
    let Some(job_id) = parse_job_id_value(&result.job_id) else {
        return;
    };
    let now = state.auth().clock.now();
    let Ok(mut tx) = state.db().begin_tenant(tenant).await else {
        tracing::warn!(job = %result.job_id, "the job result could not open a transaction");
        return;
    };
    let job = match job_repo::get_for_update(&mut tx, job_id).await {
        Ok(Some(job)) => job,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(job = %job_id, %error, "the job result could not read the job");
            return;
        }
    };
    if job.agent_id != agent || job.status.is_terminal() {
        return;
    }
    let Some(outcome) = outcome_of(result) else {
        tracing::warn!(job = %job_id, "the job result holds an unknown outcome");
        return;
    };
    // Restic exit code 11 is a held lock, not a failure: the job returns to the queue as a new job
    // or, after the last retry, to the dead-letter status (FR-SC-05). The agent reports the lock on
    // the step, and the job result of a stopped job has the status `failed`, so the step result
    // marks the row (see `queue::stale::handle_step_result`).
    let lock_held = outcome.result_code == JobResultCode::RepositoryLocked
        || (job.lock_reported && outcome.result_code == JobResultCode::Failed)
        || result.status == StepStatus::Retry as i32;
    // A cancel ends the retry chain: a flagged job never returns to the queue.
    if lock_held && !job.cancel_requested && retry::is_retryable(job.kind) {
        if let Err(error) = retry::apply_lock(&mut tx, state.queue_config(), &job, now).await {
            tracing::warn!(job = %job_id, %error, "the lock result could not update the job");
            return;
        }
        if let Err(error) = tx.commit().await {
            tracing::warn!(job = %job_id, %error, "the lock result could not commit");
            return;
        }
        state.wake_dispatcher();
        return;
    }
    let outcome = if lock_held && job.cancel_requested {
        JobOutcome {
            status: JobStatus::Failed,
            result_code: JobResultCode::Cancelled,
            result_message: None,
        }
    } else {
        outcome
    };
    // A `not_initialized` result on an initialized repository is a location mismatch, not a normal
    // result: the job fails with the typed access error and the flag stays.
    let outcome = match (outcome.result_code, job.repository_id) {
        (JobResultCode::NotInitialized, Some(repository_id)) => {
            match repo::get(&mut tx, repository_id).await {
                Ok(Some(repository)) if repository.initialized => JobOutcome {
                    status: JobStatus::Failed,
                    result_code: JobResultCode::AccessError,
                    result_message: Some(ACCESS_ERROR_CODE.to_owned()),
                },
                Ok(_) => outcome,
                Err(error) => {
                    tracing::warn!(job = %job_id, %error, "the job result could not read the repository");
                    return;
                }
            }
        }
        _ => outcome,
    };
    // A `forget` that selects no snapshot may hide a wrong filter, so the job succeeds with a warning
    // (FR-RP-08).
    let outcome = if job.empty_forget_reported && outcome.result_code == JobResultCode::Succeeded {
        JobOutcome {
            status: JobStatus::Succeeded,
            result_code: JobResultCode::SucceededWithWarnings,
            result_message: Some(FORGET_SELECTED_NOTHING.to_owned()),
        }
    } else {
        outcome
    };
    if let Err(error) = job_repo::finish(&mut tx, job_id, &outcome, now).await {
        tracing::warn!(job = %job_id, %error, "the job result could not update the job");
        return;
    }
    if matches!(
        outcome.result_code,
        JobResultCode::Succeeded | JobResultCode::SucceededWithWarnings
    ) && let Some(repository_id) = job.repository_id
    {
        apply_initialized(
            &mut tx,
            job_id,
            repository_id,
            job.location_fingerprint.as_deref(),
        )
        .await;
    }
    if let Err(error) = tx.commit().await {
        tracing::warn!(job = %job_id, %error, "the job result could not commit");
        return;
    }
    // A finished job frees its slot, so the queue may start the next job.
    state.wake_dispatcher();
}

/// Sets `repositories.initialized = true` while the location still matches the job.
async fn apply_initialized(
    tx: &mut restichub_db::TenantTx<'_>,
    job_id: JobId,
    repository_id: RepositoryId,
    fingerprint: Option<&str>,
) {
    let repository = match repo::get(tx, repository_id).await {
        Ok(Some(repository)) => repository,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(job = %job_id, %error, "the initialized flag could not read the repository");
            return;
        }
    };
    if Some(repository_fingerprint(&repository).as_str()) != fingerprint {
        // The location changed between the job and its result: the flag belongs to another
        // location, so the control plane does not set it.
        tracing::warn!(
            job = %job_id,
            "the repository location changed since the job started; the initialized flag stays"
        );
        return;
    }
    if let Err(error) = repo::set_initialized(tx, repository_id, true).await {
        tracing::warn!(job = %job_id, %error, "the initialized flag could not be set");
    }
}

/// Resolves the target agent (FR-RP-08).
///
/// An init and a local-path test-access run only on the maintenance agent: an init belongs to the
/// host that owns the repository, and a local path lives on one host. A test-access of an S3 or
/// rest repository defaults to the maintenance agent and accepts another explicit agent.
pub(crate) fn resolve_agent(
    repository: &Repository,
    explicit: Option<AgentId>,
    kind: JobKind,
) -> Result<AgentId, ApiError> {
    let maintenance_only = kind == JobKind::Init || repository.backend == BackendKind::Local;
    if maintenance_only {
        let maintenance = repository
            .maintenance_agent_id
            .ok_or_else(|| ApiError::invalid_field("agent_id", Reason::Empty))?;
        if explicit.is_some_and(|agent| agent != maintenance) {
            return Err(ApiError::invalid_field("agent_id", Reason::Unknown));
        }
        return Ok(maintenance);
    }
    explicit
        .or(repository.maintenance_agent_id)
        .ok_or_else(|| ApiError::invalid_field("agent_id", Reason::Empty))
}

/// A hex SHA-256 of the repository location. It holds no secret.
fn location_fingerprint(location: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    let digest = Sha256::digest(location.as_bytes());
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// The fingerprint of the repository location and, for a local backend, of its maintenance agent
/// (ADR-0021, FR-RP-08).
///
/// A local path lives on one host, so the maintenance agent is part of the location. A change of
/// either gives a different fingerprint, and the control plane then refuses to set `initialized`.
fn repository_fingerprint(repository: &Repository) -> String {
    let mut material = stored_location(repository);
    if repository.backend == BackendKind::Local {
        material.push('\u{1f}');
        match repository.maintenance_agent_id {
            Some(agent) => material.push_str(&agent.to_string()),
            None => material.push_str("none"),
        }
    }
    location_fingerprint(&material)
}

/// Refuses an agent that is absent, of another tenant, or revoked (FR-RP-08).
pub(crate) async fn ensure_agent_active(
    tx: &mut restichub_db::TenantTx<'_>,
    agent: AgentId,
) -> Result<(), ApiError> {
    let Some(row) = agents_repo::get(tx, agent).await? else {
        return Err(ApiError::invalid_field("agent_id", Reason::Unknown));
    };
    if row.status != AgentStatus::Active {
        return Err(ApiError::invalid_field("agent_id", Reason::Unknown));
    }
    Ok(())
}

/// Builds the repository access of the job and records each secret delivery (FR-PR-05, AC-RP-03).
pub(crate) async fn repository_access(
    tx: &mut restichub_db::TenantTx<'_>,
    provider: &dyn KekProvider,
    repository: &Repository,
    agent_id: AgentId,
    job_id: JobId,
) -> Result<RepositoryAccess, DeliveryError> {
    let reader = SecretReader::new(provider);
    let agent = agent_id.as_uuid();
    let job = job_id.as_uuid();
    let repository_uuid = repository.id.as_uuid();
    let password = read_secret(
        tx,
        &reader,
        Some(repository.password_secret_id),
        agent,
        job,
        repository_uuid,
    )
    .await?
    .unwrap_or_default();
    let mut access = RepositoryAccess {
        repository_id: repository.id.to_string(),
        location: stored_location(repository),
        password,
        backend_credentials: None,
    };
    match repository.backend {
        BackendKind::S3 => {
            let access_key = read_secret(
                tx,
                &reader,
                repository.s3_access_key_secret_id,
                agent,
                job,
                repository_uuid,
            )
            .await?
            .unwrap_or_default();
            let secret_key = read_secret(
                tx,
                &reader,
                repository.s3_secret_key_secret_id,
                agent,
                job,
                repository_uuid,
            )
            .await?
            .unwrap_or_default();
            access.backend_credentials = Some(BackendCredentials::S3(S3Credentials {
                access_key_id: access_key,
                secret_access_key: secret_key,
                session_token: String::new(),
                region: repository.s3_region.clone().unwrap_or_default(),
            }));
        }
        BackendKind::Rest => {
            let rest_password = read_secret(
                tx,
                &reader,
                repository.rest_password_secret_id,
                agent,
                job,
                repository_uuid,
            )
            .await?
            .unwrap_or_default();
            access.backend_credentials = Some(BackendCredentials::Rest(RestCredentials {
                username: String::new(),
                password: rest_password,
            }));
        }
        BackendKind::Local => {
            access.backend_credentials = Some(BackendCredentials::Local(LocalCredentials {}));
        }
    }
    Ok(access)
}

/// The secrets that [`repository_access`] reads for the repository, in the order of the reads.
pub(crate) fn delivered_secrets(repository: &Repository) -> Vec<SecretId> {
    let mut secrets = vec![repository.password_secret_id];
    match repository.backend {
        BackendKind::S3 => {
            secrets.extend(repository.s3_access_key_secret_id);
            secrets.extend(repository.s3_secret_key_secret_id);
        }
        BackendKind::Rest => secrets.extend(repository.rest_password_secret_id),
        BackendKind::Local => {}
    }
    secrets
}

/// Reads one secret value for delivery, or `None` when the repository binds no such secret.
async fn read_secret(
    tx: &mut restichub_db::TenantTx<'_>,
    reader: &SecretReader<'_>,
    secret: Option<SecretId>,
    agent: Uuid,
    job: Uuid,
    repository: Uuid,
) -> Result<Option<String>, DeliveryError> {
    let Some(secret) = secret else {
        return Ok(None);
    };
    let value: SecretString = reader
        .read_for_delivery(tx, secret, agent, job, repository)
        .await?;
    Ok(Some(value.expose_secret().to_owned()))
}

/// The ordered steps that the agent runs for one repository job (FR-RP-02).
///
/// A `test_access` step always comes first. An `init` job adds the `init` step after it, so the
/// agent runs `init` only when the repository does not exist. A bare init never leaves the control
/// plane.
fn repository_steps(kind: JobKind, repository_id: &str) -> Vec<Step> {
    let mut steps = vec![Step {
        step_id: "test-access".to_owned(),
        timeout_seconds: STEP_TIMEOUT_SECONDS,
        kind: Some(step::Kind::TestAccess(TestAccessStep {
            repository_id: repository_id.to_owned(),
        })),
    }];
    if kind == JobKind::Init {
        steps.push(Step {
            step_id: "init".to_owned(),
            timeout_seconds: STEP_TIMEOUT_SECONDS,
            kind: Some(step::Kind::Init(InitStep {
                repository_id: repository_id.to_owned(),
            })),
        });
    }
    steps
}

/// Maps the typed outcome of a `JobResult` to the stored outcome (FR-AG-09).
///
/// The error code from the agent is bounded and sanitized, so a malicious agent cannot store a huge
/// value or a control character (FR-PR-08).
fn outcome_of(result: &JobResult) -> Option<JobOutcome> {
    let outcome = ProtoJobStatus::try_from(result.outcome).ok()?;
    let result_code = match outcome {
        ProtoJobStatus::Succeeded => JobResultCode::Succeeded,
        ProtoJobStatus::Failed => JobResultCode::Failed,
        ProtoJobStatus::ConfigError => JobResultCode::ConfigError,
        ProtoJobStatus::NotInitialized => JobResultCode::NotInitialized,
        ProtoJobStatus::Cancelled => JobResultCode::Cancelled,
        ProtoJobStatus::SucceededWithWarnings => JobResultCode::SucceededWithWarnings,
        ProtoJobStatus::RepositoryLocked => JobResultCode::RepositoryLocked,
        ProtoJobStatus::Unspecified => return None,
    };
    let status = match result_code {
        JobResultCode::Succeeded | JobResultCode::SucceededWithWarnings => JobStatus::Succeeded,
        JobResultCode::Failed
        | JobResultCode::RepositoryLocked
        | JobResultCode::ConfigError
        | JobResultCode::NotInitialized
        | JobResultCode::AccessError
        | JobResultCode::Cancelled => JobStatus::Failed,
    };
    // The typed code says all: the agent repeats it as the error code of the job.
    let typed = matches!(
        result_code,
        JobResultCode::SucceededWithWarnings | JobResultCode::RepositoryLocked
    );
    let result_message = result
        .error
        .as_ref()
        .filter(|_| !typed)
        .map(|error| error.code.as_str())
        .filter(|code| {
            !code.is_empty()
                && code.len() <= MAX_RESULT_MESSAGE_LEN
                && !restichub_proto::validate::has_forbidden(code)
        })
        .map(str::to_owned);
    Some(JobOutcome {
        status,
        result_code,
        result_message,
    })
}

/// An absent value uses the maintenance agent; an anonymous identifier is not useful.
fn parse_optional_agent_id(value: Option<&str>) -> Result<Option<AgentId>, ApiError> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) => Uuid::parse_str(value)
            .map(AgentId::from_uuid)
            .map(Some)
            .map_err(|error| {
                ApiError::invalid_field_cause("agent_id", Reason::InvalidFormat, error.to_string())
            }),
    }
}

pub(crate) fn parse_job_id(value: &str) -> Result<JobId, ApiError> {
    Uuid::parse_str(value)
        .map(JobId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause("id", Reason::InvalidFormat, error.to_string())
        })
}

/// Parses a job identifier from a stream message. `None` on a malformed value.
pub(crate) fn parse_job_id_value(value: &str) -> Option<JobId> {
    Uuid::parse_str(value).ok().map(JobId::from_uuid)
}

pub(crate) fn timestamp(time: OffsetDateTime) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: time.unix_timestamp(),
        nanos: time.nanosecond() as i32,
    }
}

pub(crate) fn format_time(value: OffsetDateTime) -> String {
    use time::format_description::well_known::Rfc3339;
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use restichub_db::{JobResultCode, JobStatus};
    use restichub_proto::agent::v1::{JobResult, JobStatus as ProtoJobStatus};

    use super::outcome_of;

    fn result(outcome: ProtoJobStatus) -> JobResult {
        JobResult {
            job_id: "job-1".to_owned(),
            outcome: outcome as i32,
            ..JobResult::default()
        }
    }

    #[test]
    fn fr_ag_09_a_job_with_warnings_is_not_a_plain_success() {
        let outcome = outcome_of(&result(ProtoJobStatus::SucceededWithWarnings))
            .expect("the outcome must map");
        assert_eq!(outcome.status, JobStatus::Succeeded);
        assert_eq!(outcome.result_code, JobResultCode::SucceededWithWarnings);
        assert_eq!(outcome.result_message, None);
        let plain = outcome_of(&result(ProtoJobStatus::Succeeded)).expect("the outcome must map");
        assert_eq!(plain.result_code, JobResultCode::Succeeded);
        assert_eq!(plain.result_message, None);
    }

    #[test]
    fn fr_sc_05_a_locked_repository_is_a_typed_failure() {
        let outcome =
            outcome_of(&result(ProtoJobStatus::RepositoryLocked)).expect("the outcome must map");
        assert_eq!(outcome.status, JobStatus::Failed);
        assert_eq!(outcome.result_code, JobResultCode::RepositoryLocked);
        assert_eq!(outcome.result_message, None);
    }
}
