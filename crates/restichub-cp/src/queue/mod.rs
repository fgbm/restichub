//! The persistent job queue and its dispatcher (FR-SC-03 to FR-SC-09, FR-PR-03, FR-PR-06,
//! ADR-0010).
//!
//! The scheduler and the manual run write `queued` rows into the `jobs` table. The dispatcher looks
//! at the tenants one by one, selects the rows that may start, builds and signs each job and sends
//! it to the online agent over the stream. The agent never holds a schedule (ADR-0002).
//!
//! The pass of one tenant runs in one transaction under a tenant advisory lock, so two control plane
//! instances never count the limits against each other. The rows are locked with
//! `FOR UPDATE SKIP LOCKED`. The transaction builds and signs each job, marks it `sent`, stores the
//! signed bytes encrypted, starts its lease and reads the secrets; it commits before the control
//! plane sends the first byte (AC-RP-03). The pass reads the job signing key set before the
//! transaction and signs in memory: it never waits for a second pool connection while it holds the
//! lock. A job without an acknowledgement goes out again when its
//! lease expires (AC-PR-04). The redelivery sends the stored signed bytes and builds nothing, and
//! the agent journal dedupes by `job_id` (ADR-0020).
//!
//! A suspended tenant dispatches nothing, and its agents stay connected (AC-TN-03).

pub mod build;
mod compat;
pub mod maintenance;
pub mod restore;
pub mod retry;
pub mod select;
pub mod stale;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use prost::Message as _;
use restichub_db::audit::{AuditAction, AuditEvent, diff};
use restichub_db::jobs::{self as job_repo, Job, JobOutcome};
use restichub_db::queue as queue_repo;
use restichub_db::{AgentId, DbError, JobId, JobStatus, TenantId, TenantStatus, TenantTx, tenants};
use restichub_proto::agent::v1::{ControlMessage, Job as ProtoJob, SignedJob, control_message};
use serde_json::json;
use time::OffsetDateTime;
use zeroize::Zeroizing;

use crate::agent_stream::JobSend;
use crate::clock::Clock;
use crate::delivery::{self, DeliveryError};
use crate::job_result_params::{INCOMPATIBLE_CODE, IncompatibleParams};
use crate::job_signing::{JobSigningError, KeySet};
use crate::server::AppState;
use build::BuildError;
use select::Limits;

/// The largest number of waiting jobs of one agent that a tenant pass looks at.
const BATCH_PER_AGENT: i64 = 100;

/// How long the control plane waits before it judges a running job by the absence of its agent, and
/// before it judges the absence of an agent for the alert "Host offline" (AC-PR-07, AC-ID-06).
///
/// The agent repeats a stored result at every connection, so a job that still runs is not lost. The
/// agent waits at most `BACKOFF_MAX` of 60 s between two attempts (`restichub-agent`,
/// `stream::BACKOFF_MAX`), so the wait holds twice that delay.
pub const RECONNECT_GRACE: time::Duration = time::Duration::minutes(2);

/// How long the run of failed passes of a tenant may last before the control plane warns about it.
/// A pass of one tenant that keeps failing keeps every absence test of that tenant off (see
/// [`AgentView`]), so the warning tells the operator why no absence arrives. The run ends only
/// after passes that read the agents for the whole [`RECONNECT_GRACE`], and the warning repeats at
/// this interval while the run continues.
const LONG_GRACE_WARN: time::Duration = time::Duration::minutes(15);

/// The result message of a job that never got an acknowledgement.
const NOT_ACKNOWLEDGED: &str = "not_acknowledged";

/// The longest time that one statement of a tenant pass may run.
///
/// A pass that passes it fails and the dispatcher continues with the next tenant, so one slow
/// tenant does not stop every dispatch (AC-NFR-01).
const PASS_STATEMENT_TIMEOUT: time::Duration = time::Duration::seconds(10);

/// How often the warning about a pass that passed the statement timeout repeats for a tenant.
const TIMEOUT_WARN: time::Duration = time::Duration::minutes(15);

/// The result message of a job that the control plane could not sign.
const SIGNING_FAILED: &str = "signing_failed";

/// The result message of a sent job whose stored signed bytes are gone or unreadable.
const SIGNED_JOB_UNAVAILABLE: &str = "signed_job_unavailable";

const SAVEPOINT: &str = "queue_job";

/// The settings of the queue (FR-SC-03, FR-SC-05, FR-SC-08).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueConfig {
    /// How often the dispatcher looks at the queue. A manual run wakes it at once.
    pub interval: Duration,
    /// How long the control plane waits for the acknowledgement before it sends the job again.
    pub lease: Duration,
    /// The largest number of jobs that run on one agent.
    pub agent_limit: usize,
    /// The largest number of backups that run on one repository.
    pub repository_limit: usize,
    /// The largest number of jobs that run in one organization.
    pub global_limit: usize,
    /// The job goes to the dead-letter status when it reaches this many lock results.
    pub lock_retry_limit: u32,
    /// The delay before the first lock retry. It doubles for each retry.
    pub lock_retry_base: Duration,
    /// The job fails when the control plane sent it this many times without an acknowledgement.
    pub max_deliveries: u32,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(2),
            lease: Duration::from_secs(60),
            agent_limit: 2,
            repository_limit: 4,
            global_limit: 100,
            lock_retry_limit: 5,
            lock_retry_base: Duration::from_secs(30),
            max_deliveries: 5,
        }
    }
}

impl QueueConfig {
    fn limits(&self) -> Limits {
        Limits {
            agent: self.agent_limit.max(1),
            repository: self.repository_limit.max(1),
            global: self.global_limit.max(1),
        }
    }
}

/// When the control plane may judge an agent by its absence (AC-PR-07, AC-ID-06).
///
/// The registry of the online agents starts empty, and a database outage hides every agent, so a
/// running job looks lost and a host looks silent while its agent is still on its way back. The
/// wait starts again with the first pass that read the agents after the start of the process or
/// after a pass that could not read them.
///
/// The wait is kept for each tenant. A pass that fails in the tenant of one organization must not
/// hide a host of another organization for as long as it fails. The dispatcher, the stale lock
/// recovery and the alert evaluation read the wait of the tenant; the alert evaluation holds no
/// timer of its own.
#[derive(Debug, Default)]
pub struct AgentView {
    tenants: std::sync::Mutex<HashMap<TenantId, TenantGrace>>,
}

/// The wait of one tenant, and the run of failed passes that keeps it running.
#[derive(Debug, Default, Clone, Copy)]
struct TenantGrace {
    /// The first pass that read the agents again, while the wait runs.
    reading_since: Option<OffsetDateTime>,
    /// The first failed pass of the run that the current wait belongs to.
    failing_since: Option<OffsetDateTime>,
    /// When the warning about the current run of failures last went out.
    warned_at: Option<OffsetDateTime>,
    /// When the warning about a statement timeout last went out.
    timeout_warned_at: Option<OffsetDateTime>,
}

impl TenantGrace {
    fn passed(&mut self, now: OffsetDateTime) {
        let since = *self.reading_since.get_or_insert(now);
        if now < since + RECONNECT_GRACE {
            return;
        }
        // The agents were readable for the whole wait, so the run of failures has ended.
        self.failing_since = None;
        self.warned_at = None;
    }

    fn failed(&mut self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        self.reading_since = None;
        let failing_since = *self.failing_since.get_or_insert(now);
        let warn = now >= failing_since + LONG_GRACE_WARN
            && self
                .warned_at
                .is_none_or(|warned| now >= warned + LONG_GRACE_WARN);
        if !warn {
            return None;
        }
        self.warned_at = Some(now);
        Some(failing_since)
    }

    /// Warns about a pass that a statement timeout ended, at most once per `TIMEOUT_WARN`, so a
    /// tenant that stays slow does not fill the log.
    fn timed_out(&mut self, tenant: TenantId, now: OffsetDateTime) {
        let warn = self
            .timeout_warned_at
            .is_none_or(|warned| now >= warned + TIMEOUT_WARN);
        if warn {
            tracing::warn!(
                tenant = %tenant,
                seconds = PASS_STATEMENT_TIMEOUT.whole_seconds(),
                "the pass of a tenant passed the statement timeout, so its agent_lost, timed_out \
                 and Host offline checks stay off until a pass succeeds"
            );
            self.timeout_warned_at = Some(now);
        }
    }
}

fn warn_long_grace(tenant: TenantId, failing_since: OffsetDateTime, now: OffsetDateTime) {
    tracing::warn!(
        tenant = %tenant,
        minutes = (now - failing_since).whole_minutes(),
        "the pass of a tenant keeps failing, so the control plane judges no absent agent there"
    );
}

impl AgentView {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<TenantId, TenantGrace>> {
        self.tenants
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Records a pass that read the agents of the tenant. The run of failed passes of the tenant
    /// ends only after a pass that found the agents readable for the whole wait.
    pub fn passed(&self, tenant: TenantId, now: OffsetDateTime) {
        self.lock().entry(tenant).or_default().passed(now);
    }

    /// Records that a pass of the tenant could not read or write. The wait of that tenant starts
    /// again with its next pass, and a warning goes out when the tenant stays in it.
    pub fn failed(&self, tenant: TenantId, now: OffsetDateTime) {
        if let Some(since) = self.lock().entry(tenant).or_default().failed(now) {
            warn_long_grace(tenant, since, now);
        }
    }

    /// Records that a pass could not read the list of the tenants. The wait of every tenant starts
    /// again with its next pass.
    pub fn all_failed(&self, now: OffsetDateTime) {
        for (tenant, grace) in self.lock().iter_mut() {
            if let Some(since) = grace.failed(now) {
                warn_long_grace(*tenant, since, now);
            }
        }
    }

    /// Records a pass that a statement timeout ended, and warns at most once per `TIMEOUT_WARN`
    /// for the tenant.
    pub fn timed_out(&self, tenant: TenantId, now: OffsetDateTime) {
        self.lock()
            .entry(tenant)
            .or_default()
            .timed_out(tenant, now);
    }

    /// True when the control plane read the agents of the tenant for at least [`RECONNECT_GRACE`].
    pub fn judges_absence(&self, tenant: TenantId, now: OffsetDateTime) -> bool {
        self.lock()
            .get(&tenant)
            .and_then(|grace| grace.reading_since)
            .is_some_and(|since| now >= since + RECONNECT_GRACE)
    }
}

/// Runs one dispatcher pass over every active tenant.
///
/// The clock is injectable, so a test drives the lease, the backoff and the stale lock wait without
/// a sleep.
pub async fn run_once(state: &AppState, now: OffsetDateTime) -> Result<(), DbError> {
    let online = state.agent_registry().online_agents();
    run_with_online(state, now, &online).await
}

/// Like [`run_once`], with the set of online agents that the caller took earlier. An agent can
/// leave between the snapshot and the build of its job (FR-PR-09).
pub async fn run_with_online(
    state: &AppState,
    now: OffsetDateTime,
    online: &[AgentId],
) -> Result<(), DbError> {
    let tenant_list = {
        let read = async {
            let mut tx = state.db().begin_platform().await?;
            let list = tenants::list(&mut tx).await?;
            tx.commit().await?;
            Ok::<_, DbError>(list)
        }
        .await;
        match read {
            Ok(list) => list,
            Err(error) => {
                state.agent_view().all_failed(now);
                return Err(error);
            }
        }
    };
    for tenant in tenant_list {
        // A suspended tenant dispatches nothing; its agents stay connected (AC-TN-03).
        if tenant.status != TenantStatus::Active {
            continue;
        }
        state.agent_view().passed(tenant.id, now);
        let recovered = stale::recover_tenant(state, tenant.id, now, online).await;
        if let Err(error) = &recovered {
            if error.is_statement_timeout() {
                state.agent_view().timed_out(tenant.id, now);
            } else {
                tracing::warn!(tenant = %tenant.id, %error, "the stale lock pass of a tenant failed");
            }
        }
        let dispatched = dispatch_tenant(state, tenant.id, now, online).await;
        if let Err(error) = &dispatched {
            if error.is_statement_timeout() {
                state.agent_view().timed_out(tenant.id, now);
            } else {
                tracing::warn!(tenant = %tenant.id, %error, "the dispatch pass of a tenant failed");
            }
        }
        if recovered.is_err() || dispatched.is_err() {
            state.agent_view().failed(tenant.id, now);
        }
    }
    Ok(())
}

/// One job that the pass committed as `sent` and that goes to the agent.
struct Outgoing {
    id: JobId,
    agent: AgentId,
    signed: SignedJob,
    /// The capabilities that the live connection must hold at the moment of the send (FR-PR-09).
    required: Vec<restichub_proto::AgentCapability>,
    /// A redelivery keeps its lease when the send fails; the next lease expiry sends it again.
    redelivery: bool,
}

async fn dispatch_tenant(
    state: &AppState,
    tenant: TenantId,
    now: OffsetDateTime,
    online: &[AgentId],
) -> Result<(), DbError> {
    let config = state.queue_config();
    let signer = if online.is_empty() {
        Signer::Retry
    } else {
        load_signer(state).await
    };
    let mut tx = state.db().begin_tenant(tenant).await?;
    tx.statement_timeout(PASS_STATEMENT_TIMEOUT).await?;
    tx.force_custom_plan().await?;
    queue_repo::fail_for_revoked_agents(&mut tx, now).await?;
    if !queue_repo::try_lock_tenant(&mut tx).await? {
        return tx.commit().await;
    }
    // A cancelled job that never got an acknowledgement ends with its lease, also while no agent
    // is online.
    queue_repo::cancel_unacknowledged(&mut tx, now).await?;
    if online.is_empty() {
        return tx.commit().await;
    }
    let waiting = queue_repo::dispatchable(&mut tx, now, online, BATCH_PER_AGENT).await?;
    if waiting.is_empty() {
        return tx.commit().await;
    }
    let in_flight = queue_repo::in_flight(&mut tx).await?;
    let chosen: Vec<Job> = select::select(&waiting, &in_flight, &config.limits())
        .into_iter()
        .cloned()
        .collect();

    let lease = time::Duration::try_from(config.lease).unwrap_or(time::Duration::MINUTE);
    let mut outgoing = Vec::new();
    for job in chosen {
        // A job that went out too often without an acknowledgement fails.
        if job.status == JobStatus::Sent
            && u32::try_from(job.deliveries).unwrap_or(u32::MAX) >= config.max_deliveries
        {
            fail(
                &mut tx,
                &job,
                NOT_ACKNOWLEDGED,
                restichub_db::JobResultCode::Failed,
                now,
            )
            .await?;
            continue;
        }
        tx.savepoint(SAVEPOINT).await?;
        let prepared = if job.status == JobStatus::Sent {
            redeliver(state, &signer, &mut tx, &job, now, now + lease).await
        } else {
            first_send(state, &signer, &mut tx, &job, now, now + lease).await
        };
        match prepared? {
            Prepared::Send(signed, required) => {
                tx.release_savepoint(SAVEPOINT).await?;
                outgoing.push(Outgoing {
                    id: job.id,
                    agent: job.agent_id,
                    signed,
                    required,
                    redelivery: job.status == JobStatus::Sent,
                });
            }
            Prepared::Fail(code, result_code) => {
                // The rollback also removes the secret delivery records of the failed job.
                tx.rollback_to_savepoint(SAVEPOINT).await?;
                tracing::warn!(job = %job.id, reason = code, "the queue fails a job");
                fail(&mut tx, &job, code, result_code, now).await?;
            }
            Prepared::Incompatible(params) => {
                tx.rollback_to_savepoint(SAVEPOINT).await?;
                tracing::warn!(job = %job.id, ?params, "the queue fails an incompatible job");
                let outcome = JobOutcome {
                    status: JobStatus::Failed,
                    result_code: restichub_db::JobResultCode::Incompatible,
                    result_message: Some(INCOMPATIBLE_CODE.to_owned()),
                    result_params: Some(params.to_value()),
                };
                job_repo::finish(&mut tx, job.id, &outcome, now).await?;
            }
            Prepared::Skip => {
                tx.rollback_to_savepoint(SAVEPOINT).await?;
            }
        }
    }
    // The commit comes before the send: the delivery records and the lease are durable first.
    tx.commit().await?;

    for item in outgoing {
        send(state, tenant, item).await;
    }
    Ok(())
}

enum Signer {
    Ready(Arc<KeySet>),
    /// The key set could not be read now. The jobs stay as they are and the next pass tries again.
    Retry,
    /// The installation holds no valid key set. A job that needs a new signature fails.
    Failed,
}

async fn load_signer(state: &AppState) -> Signer {
    let Some(signing) = state.job_signing() else {
        tracing::error!("the server holds no job signing key set");
        return Signer::Retry;
    };
    match signing.key_set(state.db()).await {
        Ok(set) => Signer::Ready(set),
        Err(JobSigningError::Database(error)) => {
            tracing::warn!(%error, "the queue could not read the job signing key set");
            Signer::Retry
        }
        Err(error) => {
            tracing::warn!(%error, "the queue could not load the job signing key set");
            Signer::Failed
        }
    }
}

/// What the pass does with one selected job.
enum Prepared {
    Send(SignedJob, Vec<restichub_proto::AgentCapability>),
    Fail(&'static str, restichub_db::JobResultCode),
    /// An option of the profile needs a newer restic than the agent reports (AC-AG-09).
    Incompatible(IncompatibleParams),
    /// A database or a key error. The job stays as it is and the next pass tries again.
    Skip,
}

/// Builds, signs and stores the first send of a queued job (FR-PR-03).
async fn first_send(
    state: &AppState,
    signer: &Signer,
    tx: &mut TenantTx<'_>,
    job: &Job,
    now: OffsetDateTime,
    lease_until: OffsetDateTime,
) -> Result<Prepared, DbError> {
    let built = match build::build(state, tx, job, now).await {
        Ok(built) => built,
        Err(BuildError::Refused(refusal)) => {
            return Ok(Prepared::Fail(refusal.code(), refusal.result_code()));
        }
        Err(BuildError::Incompatible(params)) => return Ok(Prepared::Incompatible(params)),
        Err(BuildError::Transient(cause)) => {
            tracing::warn!(job = %job.id, %cause, "the queue could not build a job");
            return Ok(Prepared::Skip);
        }
    };
    let Some(provider) = state.kek() else {
        tracing::error!("the server holds no key provider");
        return Ok(Prepared::Skip);
    };
    let signed = match signer {
        Signer::Ready(set) => set.sign_job(built.job.job()),
        Signer::Retry => return Ok(Prepared::Skip),
        Signer::Failed => {
            tracing::warn!(job = %job.id, "the queue could not sign a job");
            return Ok(Prepared::Fail(
                SIGNING_FAILED,
                restichub_db::JobResultCode::Failed,
            ));
        }
    };
    let bytes = Zeroizing::new(signed.encode_to_vec());
    let sealed = match delivery::seal_signed_job(tx, provider, job.id, &bytes).await {
        Ok(sealed) => sealed,
        Err(DeliveryError::Database(error)) => return Err(error),
        Err(error) => {
            tracing::warn!(job = %job.id, %error, "the queue could not store a signed job");
            return Ok(Prepared::Skip);
        }
    };
    let stored = queue_repo::StoredSignedJob {
        key_version: sealed.key_version,
        nonce: sealed.nonce,
        ciphertext: sealed.ciphertext,
        signer_key_id: Some(signed.signer_key_id.clone()),
        secrets: built.secrets,
    };
    let timeout = Some(i32::try_from(built.job.job().timeout_seconds).unwrap_or(i32::MAX))
        .filter(|seconds| *seconds > 0);
    let stall = Some(i32::try_from(built.job.job().stall_timeout_seconds).unwrap_or(i32::MAX))
        .filter(|seconds| *seconds > 0);
    // The row can change between the select and this update, for example when the scheduler
    // replaces the job. The rollback of the savepoint removes the secret delivery records.
    if !queue_repo::mark_dispatched(tx, job.id, now, lease_until, timeout, stall, &stored).await? {
        return Ok(Prepared::Skip);
    }
    let required = compat::required_capabilities(&built.job.job().steps);
    restichub_db::job_steps::plan_described(
        tx,
        job.id,
        &crate::jobs::planned_steps(&built.job.job().steps),
    )
    .await?;
    Ok(Prepared::Send(signed, required))
}

/// Sends the stored signed bytes of a `sent` job again, without a rebuild (ADR-0020).
///
/// The job carries its secrets again, so each delivery writes the `secret_delivered` records
/// (AC-RP-03).
async fn redeliver(
    state: &AppState,
    signer: &Signer,
    tx: &mut TenantTx<'_>,
    job: &Job,
    now: OffsetDateTime,
    lease_until: OffsetDateTime,
) -> Result<Prepared, DbError> {
    let Some(provider) = state.kek() else {
        tracing::error!("the server holds no key provider");
        return Ok(Prepared::Skip);
    };
    let Some(stored) = queue_repo::signed_job(tx, job.id).await? else {
        return Ok(Prepared::Fail(
            SIGNED_JOB_UNAVAILABLE,
            restichub_db::JobResultCode::Failed,
        ));
    };
    let bytes = match delivery::open_signed_job(tx, provider, job.id, &stored).await {
        Ok(bytes) => bytes,
        Err(DeliveryError::Database(error)) => return Err(error),
        Err(error) => {
            tracing::warn!(job = %job.id, %error, "the queue could not open a stored signed job");
            return Ok(Prepared::Fail(
                SIGNED_JOB_UNAVAILABLE,
                restichub_db::JobResultCode::Failed,
            ));
        }
    };
    let signed = match SignedJob::decode(bytes.as_slice()) {
        Ok(signed) => signed,
        Err(error) => {
            tracing::warn!(job = %job.id, %error, "a stored signed job does not decode");
            return Ok(Prepared::Fail(
                SIGNED_JOB_UNAVAILABLE,
                restichub_db::JobResultCode::Failed,
            ));
        }
    };
    let signed = match resign_if_retired(state, signer, tx, job, signed, &stored).await? {
        Resigned::Current(signed) => signed,
        Resigned::Skip => return Ok(Prepared::Skip),
    };
    // The stored job may carry a field that only a newer agent reads. The agent may have come back
    // from an older release since the first send (FR-PR-09).
    let required = match ProtoJob::decode(signed.job.as_slice()) {
        Ok(stored_job) => compat::required_capabilities(&stored_job.steps),
        Err(error) => {
            tracing::warn!(job = %job.id, %error, "a stored job does not decode");
            return Ok(Prepared::Fail(
                SIGNED_JOB_UNAVAILABLE,
                restichub_db::JobResultCode::Failed,
            ));
        }
    };
    // The agent went offline after the dispatcher took its snapshot. The lease stays and the next
    // pass decides.
    let Some(live) = state.agent_registry().capabilities(job.agent_id) else {
        return Ok(Prepared::Skip);
    };
    if required.iter().any(|need| !live.contains(need)) {
        let refusal = build::Refusal::AgentCapabilityMissing;
        return Ok(Prepared::Fail(refusal.code(), refusal.result_code()));
    }
    if let Some(repository) = job.repository_id {
        for secret in &stored.secrets {
            restichub_db::audit::insert_secret_delivered(
                tx,
                *secret,
                job.agent_id.as_uuid(),
                job.id.as_uuid(),
                repository.as_uuid(),
            )
            .await?;
        }
    }
    queue_repo::mark_redelivered(tx, job.id, now, lease_until).await?;
    Ok(Prepared::Send(signed, required))
}

enum Resigned {
    Current(SignedJob),
    Skip,
}

/// Signs a stored job again when its signer key is retired, and stores the new bytes. A redelivery
/// never sends a blob of a retired or a compromised key (ADR-0030).
async fn resign_if_retired(
    state: &AppState,
    signer: &Signer,
    tx: &mut TenantTx<'_>,
    job: &Job,
    signed: SignedJob,
    stored: &queue_repo::StoredSignedJob,
) -> Result<Resigned, DbError> {
    let Some(provider) = state.kek() else {
        tracing::error!("the server holds no key provider");
        return Ok(Resigned::Skip);
    };
    // Without a readable key set the retired keys are unknown, so the job waits for the next pass.
    let Signer::Ready(set) = signer else {
        return Ok(Resigned::Skip);
    };
    if !set.revoked_key_ids().contains(&signed.signer_key_id) {
        return Ok(Resigned::Current(signed));
    }
    let fresh = set.sign_job_bytes(signed.job.clone());
    let bytes = Zeroizing::new(fresh.encode_to_vec());
    let sealed = match delivery::seal_signed_job(tx, provider, job.id, &bytes).await {
        Ok(sealed) => sealed,
        Err(DeliveryError::Database(error)) => return Err(error),
        Err(error) => {
            tracing::warn!(job = %job.id, %error, "the queue could not store a signed job again");
            return Ok(Resigned::Skip);
        }
    };
    let replacement = queue_repo::StoredSignedJob {
        key_version: sealed.key_version,
        nonce: sealed.nonce,
        ciphertext: sealed.ciphertext,
        signer_key_id: Some(fresh.signer_key_id.clone()),
        secrets: stored.secrets.clone(),
    };
    queue_repo::replace_signed_job(tx, job.id, &replacement).await?;
    let mut event = AuditEvent::new(AuditAction::JobResigned, "job");
    event.object_id = Some(job.id.as_uuid());
    // The audit diff masks a field name with "key"; a signer identifier is public.
    event.diff = Some(diff(
        None,
        Some(&json!({
            "old_signer": signed.signer_key_id,
            "new_signer": fresh.signer_key_id,
        })),
        &[],
    ));
    restichub_db::audit::insert(tx, &event).await?;
    tracing::info!(job = %job.id, "the queue signed a stored job again with the current key");
    Ok(Resigned::Current(fresh))
}

async fn fail(
    tx: &mut TenantTx<'_>,
    job: &Job,
    message: &str,
    result_code: restichub_db::JobResultCode,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    let outcome = JobOutcome {
        status: JobStatus::Failed,
        result_code,
        result_message: Some(message.to_owned()),
        result_params: None,
    };
    job_repo::finish(tx, job.id, &outcome, now).await?;
    Ok(())
}

/// Puts the committed job on the stream of the agent (FR-PR-03).
async fn send(state: &AppState, tenant: TenantId, item: Outgoing) {
    let sent = state.agent_registry().send_requiring(
        item.agent,
        ControlMessage {
            payload: Some(control_message::Payload::SignedJob(item.signed)),
        },
        &item.required,
    );
    // The agent left between the select and the send, or it came back with fewer capabilities. A
    // first send returns to the queue and the next pass builds it again; a redelivery keeps its
    // lease and the next lease expiry checks it again.
    if sent == JobSend::Sent {
        state.events().job_status(tenant, item.id, JobStatus::Sent);
    } else if !item.redelivery {
        revert(state, tenant, item.id).await;
    }
}

async fn revert(state: &AppState, tenant: TenantId, job: JobId) {
    let result = async {
        let mut tx = state.db().begin_tenant(tenant).await?;
        queue_repo::revert_dispatch(&mut tx, job).await?;
        tx.commit().await
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(%job, %error, "the queue could not return a job after a failed send");
    }
}

/// Starts the dispatcher loop. A wake from [`AppState::wake_dispatcher`] runs a pass at once.
pub fn spawn(state: AppState, clock: Arc<dyn Clock>) -> tokio::task::JoinHandle<()> {
    let supervisor = state.supervisor().clone();
    supervisor.spawn_loop("dispatcher", move |_| {
        let (state, clock) = (state.clone(), clock.clone());
        async move {
            let wake = state.dispatch_wake();
            let mut ticker = tokio::time::interval(state.queue_config().interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = ticker.tick() => {}
                    _ = wake.notified() => {}
                }
                if let Err(error) = run_once(&state, clock.now()).await {
                    tracing::warn!(%error, "the dispatcher pass failed");
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at_start() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_800_000_000).expect("a valid time")
    }

    fn tenant() -> TenantId {
        TenantId::new()
    }

    #[test]
    fn ac_pr_07_the_absence_of_an_agent_counts_only_after_the_reconnect_grace() {
        let view = AgentView::default();
        let start = at_start();
        let tenant = tenant();
        assert!(
            !view.judges_absence(tenant, start + time::Duration::hours(1)),
            "no pass has read the agents of the tenant"
        );
        view.passed(tenant, start);
        assert!(!view.judges_absence(tenant, start), "the grace runs");
        assert!(!view.judges_absence(tenant, start + RECONNECT_GRACE - time::Duration::SECOND));
        assert!(view.judges_absence(tenant, start + RECONNECT_GRACE));
        assert!(
            view.judges_absence(tenant, start + time::Duration::hours(3)),
            "a later pass keeps the start"
        );
    }

    #[test]
    fn ac_pr_07_a_failed_pass_starts_the_reconnect_grace_again() {
        let view = AgentView::default();
        let start = at_start();
        let tenant = tenant();
        view.passed(tenant, start);
        let back = start + time::Duration::hours(1);
        assert!(view.judges_absence(tenant, back));

        view.failed(tenant, back);
        assert!(
            !view.judges_absence(tenant, back),
            "the view is not reliable again"
        );
        view.passed(tenant, back);
        assert!(
            !view.judges_absence(tenant, back),
            "the grace counts from here"
        );
        assert!(view.judges_absence(tenant, back + RECONNECT_GRACE));
    }

    #[test]
    fn ac_pr_07_a_failed_pass_of_one_tenant_keeps_the_grace_of_another() {
        let view = AgentView::default();
        let start = at_start();
        let (first, second) = (tenant(), tenant());
        view.passed(first, start);
        view.passed(second, start);
        let back = start + time::Duration::hours(1);
        assert!(view.judges_absence(first, back));
        assert!(view.judges_absence(second, back));

        view.failed(second, back);
        assert!(!view.judges_absence(second, back));
        assert!(
            view.judges_absence(first, back),
            "the failure of one tenant does not hide the agents of another"
        );

        let later = back + time::Duration::hours(3);
        view.failed(second, later);
        assert!(!view.judges_absence(second, later));
        view.passed(second, later);
        assert!(
            !view.judges_absence(second, later),
            "the next pass that reads the agents starts the grace again"
        );
        assert!(view.judges_absence(second, later + RECONNECT_GRACE));
    }

    /// Runs the dispatcher passes of one tenant in their production order, and returns the number
    /// of warnings that the control plane wrote.
    fn passes(
        grace: &mut TenantGrace,
        from: OffsetDateTime,
        to: OffsetDateTime,
        step: time::Duration,
    ) -> u32 {
        let mut warnings = 0;
        let mut now = from;
        while now < to {
            grace.passed(now);
            if grace.failed(now).is_some() {
                warnings += 1;
            }
            now += step;
        }
        warnings
    }

    #[test]
    fn ac_pr_07_a_tenant_in_the_grace_warns_once_after_the_long_grace() {
        let mut grace = TenantGrace::default();
        let start = at_start();
        let warnings = passes(
            &mut grace,
            start,
            start + time::Duration::minutes(16),
            time::Duration::seconds(2),
        );
        assert_eq!(
            warnings, 1,
            "the pass of the tenant failed at every turn for 16 minutes"
        );
    }

    #[test]
    fn ac_pr_07_a_tenant_that_fails_once_each_90_seconds_warns_after_the_long_grace() {
        // The first pass fails, and the passes in between read the agents. Each read is shorter
        // than the grace, so the run of failures does not end and the warning still arrives.
        let mut grace = TenantGrace::default();
        let start = at_start();
        let step = time::Duration::seconds(2);
        let mut since_failure = time::Duration::seconds(90);
        let mut warnings = 0;
        let mut now = start;
        while now < start + time::Duration::minutes(16) {
            grace.passed(now);
            if since_failure >= time::Duration::seconds(90) {
                since_failure = time::Duration::ZERO;
                if grace.failed(now).is_some() {
                    warnings += 1;
                }
            }
            since_failure += step;
            now += step;
        }
        assert_eq!(warnings, 1, "the grace of the tenant never ends");
    }

    #[test]
    fn ac_pr_07_a_clean_grace_ends_the_run_of_failures_of_the_tenant() {
        let mut grace = TenantGrace::default();
        let start = at_start();
        let step = time::Duration::seconds(2);
        grace.passed(start);
        assert!(
            grace.failed(start).is_none(),
            "a first failure is too young to warn"
        );

        let later = start + LONG_GRACE_WARN;
        let mut now = start + step;
        while now < later {
            grace.passed(now);
            now += step;
        }
        assert!(
            grace.failed(later).is_none(),
            "no absence test of the tenant stayed off for 15 minutes, so there is nothing to warn about"
        );
    }

    #[test]
    fn ac_id_06_a_failed_read_of_the_tenant_list_starts_every_grace_again() {
        let view = AgentView::default();
        let start = at_start();
        let (first, second) = (tenant(), tenant());
        view.passed(first, start);
        view.passed(second, start);
        let back = start + time::Duration::hours(1);
        assert!(view.judges_absence(first, back));

        view.all_failed(back);
        assert!(!view.judges_absence(first, back));
        assert!(!view.judges_absence(second, back));

        view.passed(first, back);
        assert!(
            !view.judges_absence(first, back),
            "the grace counts from here"
        );
        assert!(view.judges_absence(first, back + RECONNECT_GRACE));
    }
}
