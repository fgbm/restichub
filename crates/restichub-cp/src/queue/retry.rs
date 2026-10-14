//! The retry of a job after a repository lock (FR-SC-05, ADR-0010).
//!
//! Restic exit code 11 means that another operation holds the repository lock. The control plane
//! does not give up: it ends the job and queues a new job with a new identifier and an exponential
//! delay, because the agent journal returns the stored result for a known job identifier
//! (ADR-0020). After the retry limit the job moves to the terminal `dead_letter` status and an
//! audit record raises the alert.

use restichub_db::audit::{AuditAction, AuditEvent, diff};
use restichub_db::jobs::Job;
use restichub_db::queue as queue_repo;
use restichub_db::{DbError, JobId, JobKind, TenantTx};
use serde_json::json;
use time::OffsetDateTime;

use super::QueueConfig;

const MAX_RETRY_DELAY: time::Duration = time::Duration::hours(1);

/// How long a retry waits in the queue after its delay.
const RETRY_QUEUE_LIFETIME: time::Duration = time::Duration::hours(1);

/// The stable result message of a dead-letter job. The user interface translates it (NFR-DOC-04).
pub const DEAD_LETTER_MESSAGE: &str = "lock_retries_exhausted";

/// What the control plane did with a job that reported a lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockOutcome {
    /// A new job is queued and runs at the given time.
    Retry(JobId, OffsetDateTime),
    DeadLetter,
    /// The job was not in flight any more, so nothing changed.
    Stale,
}

/// True for a kind that the queue retries after a lock.
pub fn is_retryable(kind: JobKind) -> bool {
    matches!(
        kind,
        JobKind::Backup | JobKind::Forget | JobKind::Prune | JobKind::Check | JobKind::Unlock
    )
}

/// The delay before retry number `attempt` (1 is the first retry): the base delay doubles for each
/// attempt up to one hour.
pub fn retry_delay(base: time::Duration, attempt: i32) -> time::Duration {
    let exponent = u32::try_from(attempt.saturating_sub(1))
        .unwrap_or(0)
        .min(20);
    let factor = 1_i64 << exponent;
    let delay = base.saturating_mul(i32::try_from(factor).unwrap_or(i32::MAX));
    delay.min(MAX_RETRY_DELAY)
}

/// Applies a lock result to the job in the open transaction.
///
/// The retry keeps the plan and the fire time of the job and counts one more attempt.
pub async fn apply_lock(
    tx: &mut TenantTx<'_>,
    config: &QueueConfig,
    job: &Job,
    now: OffsetDateTime,
) -> Result<LockOutcome, DbError> {
    let attempt = job.attempt.saturating_add(1);
    if u32::try_from(attempt).unwrap_or(u32::MAX) >= config.lock_retry_limit {
        queue_repo::dead_letter(tx, job.id, DEAD_LETTER_MESSAGE, attempt, now).await?;
        let mut event = AuditEvent::new(AuditAction::JobDeadLetter, "job");
        event.object_id = Some(job.id.as_uuid());
        event.diff = Some(diff(
            None,
            Some(&json!({
                "kind": job.kind.as_str(),
                "agent_id": job.agent_id.to_string(),
                "repository_id": job.repository_id.map(|id| id.to_string()),
                "plan_id": job.plan_id.map(|id| id.to_string()),
                "attempts": attempt,
                "reason": DEAD_LETTER_MESSAGE,
            })),
            &[],
        ));
        restichub_db::audit::insert(tx, &event).await?;
        return Ok(LockOutcome::DeadLetter);
    }
    let base = time::Duration::try_from(config.lock_retry_base).unwrap_or(time::Duration::MINUTE);
    let run_at = now + retry_delay(base, attempt);
    let expires_at = job
        .expires_at
        .map(|expiry| expiry.max(run_at + RETRY_QUEUE_LIFETIME));
    let created = queue_repo::retry_as_new_job(tx, job, attempt, run_at, expires_at, now).await?;
    Ok(match created {
        Some(id) => LockOutcome::Retry(id, run_at),
        None => LockOutcome::Stale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac_sc_05_the_delay_grows_with_each_attempt() {
        let base = time::Duration::seconds(30);
        let delays: Vec<_> = (1..=4).map(|attempt| retry_delay(base, attempt)).collect();
        assert_eq!(
            delays,
            vec![
                time::Duration::seconds(30),
                time::Duration::seconds(60),
                time::Duration::seconds(120),
                time::Duration::seconds(240),
            ]
        );
    }

    #[test]
    fn ac_sc_05_the_delay_has_an_upper_bound() {
        let base = time::Duration::seconds(30);
        assert_eq!(retry_delay(base, 40), MAX_RETRY_DELAY);
    }
}
