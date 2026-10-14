//! The stale lock recovery (FR-SC-06, AC-SC-06).
//!
//! A hard kill of restic, or the loss of an agent in the middle of a job, can leave a repository
//! lock behind. The control plane marks the job (`stale_lock_since`). Once the mark is at least
//! [`STALE_LOCK_WAIT`] old and no job waits or runs on the repository, the control plane enqueues an
//! `unlock` job. The step never removes locks of a running operation: `unlock_all` stays an Admin
//! action with a confirmation.

use restichub_db::audit::{AuditAction, AuditEvent, diff};
use restichub_db::jobs::{self as job_repo, NewJob};
use restichub_db::queue as queue_repo;
use restichub_db::{
    AgentId, AgentStatus, DbError, JobId, JobKind, TenantId, TenantTx, agents as agents_repo,
    repositories as repo,
};
use restichub_proto::agent::v1::StepResult;
use serde_json::json;
use time::OffsetDateTime;

use crate::server::AppState;

/// How long the control plane waits after it loses an agent that held a lock (FR-SC-06).
pub const STALE_LOCK_WAIT: time::Duration = time::Duration::minutes(30);

/// How long an unlock job stays valid in the queue.
const UNLOCK_JOB_LIFETIME: time::Duration = time::Duration::hours(24);

/// Runs one recovery pass for a tenant.
pub async fn recover_tenant(
    state: &AppState,
    tenant: TenantId,
    now: OffsetDateTime,
    online: &[AgentId],
) -> Result<(), DbError> {
    let mut tx = state.db().begin_tenant(tenant).await?;
    if !queue_repo::try_lock_tenant(&mut tx).await? {
        return Ok(());
    }
    let cutoff = now - STALE_LOCK_WAIT;

    // A running job whose agent vanished for the whole wait time holds a lock that nobody will
    // release. The job fails and its lock becomes a suspect.
    for lost in queue_repo::lost_running(&mut tx, cutoff, online).await? {
        queue_repo::mark_agent_lost(&mut tx, lost.id, lost.lost_at, now).await?;
    }

    for stale in queue_repo::stale_repositories(&mut tx, cutoff).await? {
        let Some(repository) = repo::get_for_update(&mut tx, stale.repository_id).await? else {
            queue_repo::clear_stale_locks(&mut tx, stale.repository_id).await?;
            continue;
        };
        // A later success proves that the lock does not block the repository.
        if queue_repo::has_success_since(&mut tx, stale.repository_id, stale.stale_since).await? {
            queue_repo::clear_stale_locks(&mut tx, stale.repository_id).await?;
            continue;
        }
        // An operation may still run on the repository. The mark stays and the next pass tries again.
        if queue_repo::has_any_active_for_repository(&mut tx, stale.repository_id).await? {
            continue;
        }
        let agent_id = repository.maintenance_agent_id.unwrap_or(stale.agent_id);
        if !agent_is_active(&mut tx, agent_id).await? {
            continue;
        }
        let job_id = JobId::new();
        job_repo::create(
            &mut tx,
            &NewJob {
                id: job_id,
                kind: JobKind::Unlock,
                agent_id,
                repository_id: Some(stale.repository_id),
                plan_id: None,
                fire_time: None,
                expires_at: Some(now + UNLOCK_JOB_LIFETIME),
                location_fingerprint: None,
                created_by: None,
            },
        )
        .await?;
        queue_repo::clear_stale_locks(&mut tx, stale.repository_id).await?;

        let mut event = AuditEvent::new(AuditAction::JobRun, "job");
        event.object_id = Some(job_id.as_uuid());
        event.diff = Some(diff(
            None,
            Some(&json!({
                "kind": JobKind::Unlock.as_str(),
                "repository_id": stale.repository_id.to_string(),
                "agent_id": agent_id.to_string(),
                "reason": "stale_lock",
            })),
            &[],
        ));
        restichub_db::audit::insert(&mut tx, &event).await?;
    }
    tx.commit().await
}

/// Records a `StepResult` that reports a possible stale lock after a hard kill (FR-AG-10).
///
/// The identity comes from the certificate, never from the message (FR-PR-08), so a message for a
/// job of another agent changes no row.
pub async fn handle_step_result(
    state: &AppState,
    tenant: TenantId,
    agent: AgentId,
    result: &StepResult,
) {
    if !result.possible_stale_lock {
        return;
    }
    let Some(job_id) = crate::jobs::parse_job_id_value(&result.job_id) else {
        return;
    };
    let now = state.auth().clock.now();
    let outcome = async {
        let mut tx = state.db().begin_tenant(tenant).await?;
        let Some(job) = job_repo::get_for_update(&mut tx, job_id).await? else {
            return Ok(());
        };
        if job.agent_id == agent {
            queue_repo::note_stale_lock(&mut tx, job_id, now).await?;
        }
        tx.commit().await
    }
    .await;
    if let Err(error) = outcome {
        tracing::warn!(job = %job_id, %error, "the step result could not mark a possible stale lock");
    }
}

async fn agent_is_active(tx: &mut TenantTx<'_>, agent: AgentId) -> Result<bool, DbError> {
    Ok(agents_repo::get(tx, agent)
        .await?
        .is_some_and(|row| row.status == AgentStatus::Active))
}
