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
    AgentId, AgentStatus, DbError, JobId, JobKind, JobStatus, TenantId, TenantTx,
    agents as agents_repo, repositories as repo,
};
use restichub_proto::agent::v1::{StepResult, StepStatus, step_result};
use serde_json::json;
use time::OffsetDateTime;

use crate::job_data::WriteOutcome;
use crate::server::AppState;

/// How long the control plane waits after it loses an agent that held a lock (FR-SC-06).
pub const STALE_LOCK_WAIT: time::Duration = time::Duration::minutes(30);

/// How long a running job may outlive its maximum duration before the control plane fails it. The grace
/// covers the clock skew and the time that the agent needs to stop the job.
const CEILING_VARIABLE: &str = "RESTICHUB_JOB_HARD_CEILING_SECS";

/// Above the longest maximum duration of a profile (30 days).
const DEFAULT_CEILING: time::Duration = time::Duration::days(31);

const MIN_CEILING_SECONDS: i64 = 3600;

/// The time after which the control plane fails a job that is still `running`, whatever its
/// limits (ADR-0039). The variable overrides the default.
#[must_use]
pub fn hard_ceiling() -> time::Duration {
    ceiling_from(std::env::var(CEILING_VARIABLE).ok().as_deref())
}

fn ceiling_from(value: Option<&str>) -> time::Duration {
    value
        .and_then(|value| value.trim().parse::<i64>().ok())
        .map_or(DEFAULT_CEILING, |seconds| {
            time::Duration::seconds(seconds.max(MIN_CEILING_SECONDS))
        })
}

pub const TIMEOUT_GRACE: time::Duration = time::Duration::minutes(10);

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

    // The registry is empty after the start and after a database outage that hid the agents. A job
    // of an agent that reconnects then is not lost, so neither sweep runs before the agent had the
    // time to connect again (AC-PR-07).
    if state.agent_view().judges_absence(now) {
        // The agent enforces the maximum duration of a job that carries one. A running job far past
        // it is gone, also when the agent is online again: it restarted and lost the job. Its lock
        // becomes a suspect. A job without a maximum duration has no wall-clock limit (ADR-0039).
        for id in queue_repo::timed_out_running(&mut tx, now, TIMEOUT_GRACE, hard_ceiling()).await?
        {
            queue_repo::mark_timed_out(&mut tx, id, now).await?;
        }

        // A running job whose agent vanished for the whole wait time holds a lock that nobody will
        // release. The job fails and its lock becomes a suspect.
        for lost in queue_repo::lost_running(&mut tx, cutoff, online).await? {
            queue_repo::mark_agent_lost(&mut tx, lost.id, lost.lost_at, now).await?;
        }
    }

    for stale in queue_repo::stale_repositories(&mut tx, cutoff).await? {
        let Some(repository) = repo::get_for_update(&mut tx, stale.repository_id).await? else {
            queue_repo::clear_stale_locks(&mut tx, stale.repository_id).await?;
            continue;
        };
        // A success of a kind that takes the exclusive lock of restic, or that removes the locks,
        // proves that no lock blocks the repository. A backup runs next to the orphaned lock
        // (FR-SC-04).
        if queue_repo::has_success_since(&mut tx, stale.repository_id, stale.stale_since).await? {
            queue_repo::clear_stale_locks(&mut tx, stale.repository_id).await?;
            continue;
        }
        // An operation may still run on the repository. The mark stays and the next pass tries again.
        if queue_repo::has_any_active_for_repository(&mut tx, stale.repository_id).await? {
            continue;
        }
        let preferred = repository.maintenance_agent_id.unwrap_or(stale.agent_id);
        let agent_id = if agent_is_active(&mut tx, preferred).await? {
            preferred
        } else if let Some(other) =
            queue_repo::unlock_fallback_agent(&mut tx, stale.repository_id).await?
        {
            other
        } else {
            // The mark stays, so the dashboard keeps showing the repository as lock suspect.
            tracing::warn!(
                repository = %stale.repository_id,
                agent = %preferred,
                "no active agent can run the unlock of the repository"
            );
            continue;
        };
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

/// The most job identifiers that the control plane reads from one hello.
const MAX_LISTED_JOBS: usize = 4096 + 64;

/// Fails the running jobs that a connecting agent does not list (ADR-0039).
///
/// The agent restarted and lost the job, so the job can never report. Its lock becomes a suspect.
/// No grace applies: the agent lists a job before it acknowledges it, so a running job that the
/// hello omits is gone.
pub async fn fail_unlisted_running(
    state: &AppState,
    tenant: TenantId,
    agent: AgentId,
    listed: &[String],
) {
    let listed: Vec<uuid::Uuid> = listed
        .iter()
        .take(MAX_LISTED_JOBS)
        .filter_map(|id| crate::jobs::parse_job_id_value(id))
        .map(|id| id.as_uuid())
        .collect();
    let now = state.auth().clock.now();
    let outcome = async {
        let mut tx = state.db().begin_tenant(tenant).await?;
        for id in queue_repo::running_unlisted(&mut tx, agent, &listed).await? {
            queue_repo::mark_agent_lost(&mut tx, id, now, now).await?;
        }
        tx.commit().await
    }
    .await;
    if let Err(error) = outcome {
        tracing::warn!(%agent, %error, "cannot fail the jobs that the agent lost");
    }
}

/// Records what a `StepResult` says about the repository lock of a job (FR-AG-10, FR-SC-05).
///
/// The retry status (restic exit code 11) marks the job, because the job result of a stopped job
/// has the status `failed` and carries no step status. A hard kill marks a possible stale lock.
/// Only a job that is `sent` or `running` takes a mark. The identity comes from the certificate,
/// never from the message (FR-PR-08), so a message for a job of another agent changes no row.
pub async fn handle_step_result(
    state: &AppState,
    tenant: TenantId,
    agent: AgentId,
    result: &StepResult,
) -> WriteOutcome {
    let lock_held = result.status == StepStatus::Retry as i32;
    let empty_forget = selected_nothing(result);
    if !result.possible_stale_lock && !lock_held && !empty_forget {
        return WriteOutcome::Ignored;
    }
    let Some(job_id) = crate::jobs::parse_job_id_value(&result.job_id) else {
        return WriteOutcome::Ignored;
    };
    let now = state.auth().clock.now();
    let outcome = async {
        let mut tx = state.db().begin_tenant(tenant).await?;
        let Some(job) = job_repo::get_for_update(&mut tx, job_id).await? else {
            return Ok(false);
        };
        let mut marked = false;
        if job.agent_id == agent && matches!(job.status, JobStatus::Sent | JobStatus::Running) {
            if lock_held {
                marked |= queue_repo::note_lock_reported(&mut tx, job_id).await?;
            }
            if result.possible_stale_lock {
                marked |= queue_repo::note_stale_lock(&mut tx, job_id, now).await?;
            }
            if empty_forget {
                marked |= queue_repo::note_empty_forget(&mut tx, job_id).await?;
            }
        }
        tx.commit().await?;
        Ok::<_, DbError>(marked)
    }
    .await;
    match outcome {
        Ok(true) => WriteOutcome::Stored,
        Ok(false) => WriteOutcome::Ignored,
        Err(error) => {
            tracing::warn!(job = %job_id, %error, "the step result could not mark the job");
            WriteOutcome::Transient
        }
    }
}

/// True for a successful `forget` step whose result holds no snapshot. A truncated result and a
/// step without a result make no claim (FR-RP-08).
fn selected_nothing(result: &StepResult) -> bool {
    result.status == StepStatus::Success as i32
        && match &result.output {
            Some(step_result::Output::ForgetResult(forget)) => {
                !forget.truncated
                    && forget
                        .groups
                        .iter()
                        .all(|group| group.kept.is_empty() && group.removed.is_empty())
            }
            _ => false,
        }
}

async fn agent_is_active(tx: &mut TenantTx<'_>, agent: AgentId) -> Result<bool, DbError> {
    Ok(agents_repo::get(tx, agent)
        .await?
        .is_some_and(|row| row.status == AgentStatus::Active))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac_sc_10_the_ceiling_variable_overrides_the_default_with_a_lower_bound() {
        assert_eq!(ceiling_from(None), DEFAULT_CEILING);
        assert_eq!(ceiling_from(Some("junk")), DEFAULT_CEILING);
        assert_eq!(ceiling_from(Some("7200")), time::Duration::seconds(7200));
        assert_eq!(ceiling_from(Some("5")), time::Duration::seconds(3600));
    }
}
