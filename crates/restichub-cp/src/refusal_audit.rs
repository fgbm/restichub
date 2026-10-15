//! The audit records of the messages that the control plane refuses (AC-PR-01, AC-PR-05).
//!
//! The audit log only grows, so every record passes the [`AuditLimiter`] of the state. A record
//! holds the message kind and the claimed job, never the payload.

use std::time::Instant;

use restichub_db::audit::{Actor, AuditAction, AuditEvent, diff};
use restichub_db::{AgentId, DbError, JobId, TenantId, TenantTx};
use serde_json::{Value, json};

use crate::audit_limit::{Admission, Subject};
use crate::server::AppState;

/// Why the control plane ignores a message of an agent (AC-PR-05).
#[derive(Debug, Clone, Copy)]
pub(crate) enum MessageRefusal {
    ForeignJob,
    JobNotSent,
    UnknownJob,
}

impl MessageRefusal {
    fn code(self) -> &'static str {
        match self {
            Self::ForeignJob => "foreign_job",
            Self::JobNotSent => "job_not_sent",
            Self::UnknownJob => "unknown_job",
        }
    }
}

/// The reason of a rejection that names a job which the tenant of the agent does not hold.
pub(crate) const UNKNOWN_JOB: &str = "unknown_job";

/// Audits a message that the control plane ignores. The record commits with the transaction of
/// the caller.
pub(crate) async fn refused_message(
    state: &AppState,
    tx: &mut TenantTx<'_>,
    agent: AgentId,
    job: JobId,
    kind: &str,
    refusal: MessageRefusal,
) -> Result<(), DbError> {
    let subject = Subject::Refusal(refusal.code());
    if !admit(state, tx, agent, subject).await? {
        return Ok(());
    }
    let mut event = AuditEvent::new(AuditAction::AgentMessageRefused, "job");
    event.actor = Actor::Agent(agent.as_uuid());
    let mut after = json!({ "message": kind, "reason": refusal.code() });
    if matches!(refusal, MessageRefusal::UnknownJob) {
        // The tenant of the agent holds no such job, so the record names no object.
        after["job_id"] = Value::String(job.to_string());
    } else {
        event.object_id = Some(job.as_uuid());
    }
    event.diff = Some(diff(None, Some(&after), &[]));
    restichub_db::audit::insert(tx, &event).await.map(|_| ())
}

/// Audits a `JobRejected` that the job of the agent does not carry: the signature failed, or the
/// job is unknown. `job` is the claimed job id, if the message holds a valid one.
pub(crate) async fn rejection(
    state: &AppState,
    tx: &mut TenantTx<'_>,
    agent: AgentId,
    job: Option<JobId>,
    reason: &'static str,
) -> Result<(), DbError> {
    let subject = Subject::Rejection(reason);
    if !admit(state, tx, agent, subject).await? {
        return Ok(());
    }
    let mut event = AuditEvent::new(AuditAction::JobRejected, "job");
    event.actor = Actor::Agent(agent.as_uuid());
    event.object_id = job.map(|job| job.as_uuid());
    event.diff = Some(diff(None, Some(&json!({ "reason": reason })), &[]));
    restichub_db::audit::insert(tx, &event).await.map(|_| ())
}

/// Audits a `JobRejected` for a job id that the tenant of the agent does not hold. The record
/// names no object and keeps the claimed id and the reason of the agent.
pub(crate) async fn rejection_of_unknown_job(
    state: &AppState,
    tx: &mut TenantTx<'_>,
    agent: AgentId,
    job: JobId,
    reason: &str,
) -> Result<(), DbError> {
    let subject = Subject::Rejection(UNKNOWN_JOB);
    if !admit(state, tx, agent, subject).await? {
        return Ok(());
    }
    let mut event = AuditEvent::new(AuditAction::JobRejected, "job");
    event.actor = Actor::Agent(agent.as_uuid());
    let after = json!({ "reason": UNKNOWN_JOB, "job_id": job.to_string(), "rejection": reason });
    event.diff = Some(diff(None, Some(&after), &[]));
    restichub_db::audit::insert(tx, &event).await.map(|_| ())
}

/// Writes the summary records for the counts that the limit suppressed, when the stream of the
/// agent closes.
pub async fn flush_summaries(state: &AppState, tenant: TenantId, agent: AgentId) {
    let pending = state.audit_limiter().close(agent);
    if pending.is_empty() {
        return;
    }
    let Ok(mut tx) = state.db().begin_tenant(tenant).await else {
        tracing::warn!(%agent, "the audit summary could not open a transaction");
        return;
    };
    for (subject, suppressed) in pending {
        if let Err(error) = insert_summary(&mut tx, agent, subject, suppressed).await {
            tracing::warn!(%agent, %error, "the audit summary could not be stored");
            return;
        }
    }
    if let Err(error) = tx.commit().await {
        tracing::warn!(%agent, %error, "the audit summary could not commit");
    }
}

/// `true` when the caller writes its record. A window that ended also writes its summary.
async fn admit(
    state: &AppState,
    tx: &mut TenantTx<'_>,
    agent: AgentId,
    subject: Subject,
) -> Result<bool, DbError> {
    match state.audit_limiter().admit(agent, subject, Instant::now()) {
        Admission::Skip => Ok(false),
        Admission::Write { summary } => {
            if let Some(suppressed) = summary {
                insert_summary(tx, agent, subject, suppressed).await?;
            }
            Ok(true)
        }
    }
}

async fn insert_summary(
    tx: &mut TenantTx<'_>,
    agent: AgentId,
    subject: Subject,
    suppressed: u64,
) -> Result<(), DbError> {
    let (action, reason) = match subject {
        Subject::Rejection(reason) => (AuditAction::JobRejected, reason),
        Subject::Refusal(reason) => (AuditAction::AgentMessageRefused, reason),
    };
    let mut event = AuditEvent::new(action, "job");
    event.actor = Actor::Agent(agent.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&json!({ "reason": reason, "suppressed": suppressed })),
        &[],
    ));
    restichub_db::audit::insert(tx, &event).await.map(|_| ())
}
