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
//! plane sends the first byte (AC-RP-03). A job without an acknowledgement goes out again when its
//! lease expires (AC-PR-04). The redelivery sends the stored signed bytes and builds nothing, and
//! the agent journal dedupes by `job_id` (ADR-0020).
//!
//! A suspended tenant dispatches nothing, and its agents stay connected (AC-TN-03).

pub mod build;
mod compat;
pub mod maintenance;
pub mod retry;
pub mod select;
pub mod stale;

use std::sync::Arc;
use std::time::Duration;

use prost::Message as _;
use restichub_db::jobs::{self as job_repo, Job, JobOutcome};
use restichub_db::queue as queue_repo;
use restichub_db::{AgentId, DbError, JobId, JobStatus, TenantId, TenantStatus, TenantTx, tenants};
use restichub_proto::agent::v1::{ControlMessage, Job as ProtoJob, SignedJob, control_message};
use time::OffsetDateTime;
use zeroize::Zeroizing;

use crate::agent_stream::JobSend;
use crate::clock::Clock;
use crate::delivery::{self, DeliveryError};
use crate::server::AppState;
use build::BuildError;
use select::Limits;

/// The largest number of waiting jobs of one agent that a tenant pass looks at.
const BATCH_PER_AGENT: i64 = 100;

/// The result message of a job that never got an acknowledgement.
const NOT_ACKNOWLEDGED: &str = "not_acknowledged";

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
        let mut tx = state.db().begin_platform().await?;
        let list = tenants::list(&mut tx).await?;
        tx.commit().await?;
        list
    };
    for tenant in tenant_list {
        // A suspended tenant dispatches nothing; its agents stay connected (AC-TN-03).
        if tenant.status != TenantStatus::Active {
            continue;
        }
        if let Err(error) = stale::recover_tenant(state, tenant.id, now, online).await {
            tracing::warn!(tenant = %tenant.id, %error, "the stale lock pass of a tenant failed");
        }
        if let Err(error) = dispatch_tenant(state, tenant.id, now, online).await {
            tracing::warn!(tenant = %tenant.id, %error, "the dispatch pass of a tenant failed");
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
    let mut tx = state.db().begin_tenant(tenant).await?;
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
            redeliver(state, &mut tx, &job, now, now + lease).await
        } else {
            first_send(state, &mut tx, &job, now, now + lease).await
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

/// What the pass does with one selected job.
enum Prepared {
    Send(SignedJob, Vec<restichub_proto::AgentCapability>),
    Fail(&'static str, restichub_db::JobResultCode),
    /// A database or a key error. The job stays as it is and the next pass tries again.
    Skip,
}

/// Builds, signs and stores the first send of a queued job (FR-PR-03).
async fn first_send(
    state: &AppState,
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
        Err(BuildError::Transient(cause)) => {
            tracing::warn!(job = %job.id, %cause, "the queue could not build a job");
            return Ok(Prepared::Skip);
        }
    };
    let (Some(signing), Some(provider)) = (state.job_signing(), state.kek()) else {
        tracing::error!("the server holds no job signing key set or key provider");
        return Ok(Prepared::Skip);
    };
    let signed = match signing.sign_job(state.db(), built.job.job()).await {
        Ok(signed) => signed,
        Err(error) => {
            tracing::warn!(job = %job.id, %error, "the queue could not sign a job");
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
        secrets: built.secrets,
    };
    let timeout = i32::try_from(build::JOB_TIMEOUT_SECONDS).unwrap_or(i32::MAX);
    queue_repo::mark_dispatched(tx, job.id, now, lease_until, timeout, &stored).await?;
    let required = compat::required_capabilities(&built.job.job().steps);
    Ok(Prepared::Send(signed, required))
}

/// Sends the stored signed bytes of a `sent` job again, without a rebuild (ADR-0020).
///
/// The job carries its secrets again, so each delivery writes the `secret_delivered` records
/// (AC-RP-03).
async fn redeliver(
    state: &AppState,
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
    if sent != JobSend::Sent && !item.redelivery {
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
    tokio::spawn(async move {
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
    })
}
