//! The persistent job queue and its dispatcher (FR-SC-03 to FR-SC-09, FR-PR-03, FR-PR-06,
//! ADR-0010).
//!
//! The scheduler and the manual run write `queued` rows into the `jobs` table. The dispatcher looks
//! at the tenants one by one, selects the rows that may start, builds and signs each job and sends
//! it to the online agent over the stream. The agent never holds a schedule (ADR-0002).
//!
//! The pass of one tenant runs in one transaction under a tenant advisory lock, so two control plane
//! instances never count the limits against each other. The rows are locked with
//! `FOR UPDATE SKIP LOCKED`. The transaction marks each job `sent`, starts its lease and reads the
//! secrets; it commits before the control plane sends the first byte (AC-RP-03). A job without an
//! acknowledgement goes out again when its lease expires (AC-PR-04); the agent journal dedupes by
//! `job_id`.
//!
//! A suspended tenant dispatches nothing, and its agents stay connected (AC-TN-03).

pub mod build;
pub mod retry;
pub mod select;
pub mod stale;

use std::sync::Arc;
use std::time::Duration;

use restichub_db::jobs::{self as job_repo, Job, JobOutcome};
use restichub_db::queue as queue_repo;
use restichub_db::{AgentId, DbError, JobId, JobStatus, TenantId, TenantStatus, TenantTx, tenants};
use restichub_proto::agent::v1::{ControlMessage, control_message};
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::server::AppState;
use build::BuildError;
use select::Limits;

/// The largest number of waiting jobs that one tenant pass looks at.
const BATCH: i64 = 100;

/// The result message of a job that never got an acknowledgement.
const NOT_ACKNOWLEDGED: &str = "not_acknowledged";

/// The result message of a job that the control plane could not sign.
const SIGNING_FAILED: &str = "signing_failed";

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
    let tenant_list = {
        let mut tx = state.db().begin_platform().await?;
        let list = tenants::list(&mut tx).await?;
        tx.commit().await?;
        list
    };
    let online = state.agent_registry().online_agents();
    for tenant in tenant_list {
        // A suspended tenant dispatches nothing; its agents stay connected (AC-TN-03).
        if tenant.status != TenantStatus::Active {
            continue;
        }
        if let Err(error) = stale::recover_tenant(state, tenant.id, now, &online).await {
            tracing::warn!(tenant = %tenant.id, %error, "the stale lock pass of a tenant failed");
        }
        if let Err(error) = dispatch_tenant(state, tenant.id, now, &online).await {
            tracing::warn!(tenant = %tenant.id, %error, "the dispatch pass of a tenant failed");
        }
    }
    Ok(())
}

/// One job that the pass committed as `sent` and that goes to the agent.
struct Outgoing {
    id: JobId,
    agent: AgentId,
    job: restichub_compiler::ValidatedJob,
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
    if online.is_empty() || !queue_repo::try_lock_tenant(&mut tx).await? {
        return tx.commit().await;
    }
    let waiting = queue_repo::dispatchable(&mut tx, now, online, BATCH).await?;
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
        match build::build(state, &mut tx, &job, now).await {
            Ok(validated) => {
                queue_repo::mark_dispatched(&mut tx, job.id, now, now + lease).await?;
                tx.release_savepoint(SAVEPOINT).await?;
                outgoing.push(Outgoing {
                    id: job.id,
                    agent: job.agent_id,
                    job: validated,
                });
            }
            Err(BuildError::Refused(refusal)) => {
                // The rollback also removes the secret delivery records of the refused job.
                tx.rollback_to_savepoint(SAVEPOINT).await?;
                tracing::warn!(job = %job.id, reason = refusal.code(), "the queue refused a job");
                fail(&mut tx, &job, refusal.code(), refusal.result_code(), now).await?;
            }
            Err(BuildError::Transient(cause)) => {
                tx.rollback_to_savepoint(SAVEPOINT).await?;
                tracing::warn!(job = %job.id, %cause, "the queue could not build a job");
            }
        }
    }
    // The commit comes before the send: the delivery records and the lease are durable first.
    tx.commit().await?;

    for item in outgoing {
        send(state, tenant, item, now).await;
    }
    Ok(())
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

/// Signs the committed job and puts it on the stream of the agent (FR-PR-03).
async fn send(state: &AppState, tenant: TenantId, item: Outgoing, now: OffsetDateTime) {
    let Some(signing) = state.job_signing() else {
        tracing::error!("the server holds no job signing key set");
        revert(state, tenant, item.id).await;
        return;
    };
    let signed = match signing.sign_job(state.db(), item.job.job()).await {
        Ok(signed) => signed,
        Err(error) => {
            tracing::warn!(job = %item.id, %error, "the queue could not sign a job");
            finish_failed(state, tenant, item.id, SIGNING_FAILED, now).await;
            return;
        }
    };
    let sent = state.agent_registry().send_to_agent(
        item.agent,
        ControlMessage {
            payload: Some(control_message::Payload::SignedJob(signed)),
        },
    );
    if !sent {
        // The agent left between the select and the send. The job returns to the queue.
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

async fn finish_failed(
    state: &AppState,
    tenant: TenantId,
    job: JobId,
    message: &str,
    now: OffsetDateTime,
) {
    let outcome = JobOutcome {
        status: JobStatus::Failed,
        result_code: restichub_db::JobResultCode::Failed,
        result_message: Some(message.to_owned()),
    };
    let result = async {
        let mut tx = state.db().begin_tenant(tenant).await?;
        job_repo::finish(&mut tx, job, &outcome, now).await?;
        tx.commit().await
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(%job, %error, "the queue could not fail a job");
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
