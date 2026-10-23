//! The failure state of a plan or a maintenance schedule (FR-SC-12).
//!
//! A pass that fails does not change the `enabled` switch of the user. The scheduler keeps a typed
//! reason on the row. A database error keeps the fire time and sets a growing pause, so the missed
//! run policy decides after the recovery (FR-SC-07). A schedule that this binary cannot compute has
//! no fire time and waits for an edit. Both transitions of the state leave an audit record of the
//! system.

use restichub_db::audit::{AuditAction, AuditEvent, diff};
use restichub_db::schedule_failure::ScheduleFailure;
use restichub_db::{
    DbError, MaintenanceTask, RepositoryId, ScheduleFailureReason, TenantTx, audit,
};
use serde::Serialize;
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::schedule::ScheduleError;

/// The pause after the first failed pass. It doubles for each following failure.
const RETRY_BASE: Duration = Duration::seconds(5);

/// The longest pause between two passes over a failed row.
const RETRY_MAX: Duration = Duration::minutes(10);

/// The pause after failed pass number `attempts` (1 is the first).
pub fn retry_pause(attempts: i32) -> Duration {
    let exponent = u32::try_from(attempts.saturating_sub(1))
        .unwrap_or(0)
        .min(20);
    RETRY_BASE
        .saturating_mul(i32::try_from(1_u32 << exponent).unwrap_or(i32::MAX))
        .min(RETRY_MAX)
}

/// A pass fails on the database or on the computation of the next fire time.
#[derive(Debug, thiserror::Error)]
pub enum PassError {
    #[error("{0}")]
    Db(#[from] DbError),
    #[error("{0}")]
    Schedule(#[from] ScheduleError),
}

impl PassError {
    pub fn reason(&self) -> ScheduleFailureReason {
        match self {
            Self::Db(_) => ScheduleFailureReason::DatabaseError,
            Self::Schedule(error) => ScheduleFailureReason::from(*error),
        }
    }

    /// Short label of the error class, stable for log queries.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Db(error) => error.code(),
            Self::Schedule(_) => "schedule",
        }
    }

    pub fn sqlstate(&self) -> Option<String> {
        match self {
            Self::Db(error) => error.sqlstate(),
            Self::Schedule(_) => None,
        }
    }
}

impl From<ScheduleError> for ScheduleFailureReason {
    fn from(error: ScheduleError) -> Self {
        match error {
            ScheduleError::Cron => Self::InvalidCron,
            ScheduleError::TimeZone => Self::UnknownTimeZone,
            ScheduleError::Search => Self::NoFireTime,
        }
    }
}

/// The failure state of a plan or a maintenance schedule in the API (FR-SC-12).
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleFailureView {
    pub reason: ScheduleFailureReason,
    /// The time of the first failed pass of this failure, in RFC 3339.
    pub since: String,
    /// The time of the next try, in RFC 3339. Null when only an edit of the schedule helps.
    pub retry_after: Option<String>,
}

impl ScheduleFailureView {
    pub fn of(failure: &ScheduleFailure) -> Self {
        Self {
            reason: failure.reason,
            since: format_time(failure.since),
            retry_after: failure.retry_after.map(format_time),
        }
    }
}

fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

/// The time at which the claim takes a row again after failed pass number `attempts`.
pub fn retry_after(now: OffsetDateTime, attempts: i32) -> OffsetDateTime {
    now + retry_pause(attempts)
}

/// The number of the failed pass that is under way for a row.
pub fn next_attempt(previous: Option<&ScheduleFailure>) -> i32 {
    previous.map_or(1, |failure| failure.attempts.saturating_add(1))
}

/// True when the failure is new for the row or has another reason, so it needs an audit record.
pub fn is_news(previous: Option<&ScheduleFailure>, reason: ScheduleFailureReason) -> bool {
    previous.is_none_or(|failure| failure.reason != reason)
}

/// The row and the extra detail of a failure audit record.
pub struct Target {
    object_type: &'static str,
    object_id: Uuid,
    detail: Value,
}

impl Target {
    pub fn plan(id: restichub_db::PlanId) -> Self {
        Self {
            object_type: "plan",
            object_id: id.as_uuid(),
            detail: json!({}),
        }
    }

    /// The record belongs to the repository, so the audit log names it and finds it by the
    /// repository.
    pub fn maintenance(repository_id: RepositoryId, task: MaintenanceTask) -> Self {
        Self {
            object_type: "repository",
            object_id: repository_id.as_uuid(),
            detail: json!({ "task": task.as_str() }),
        }
    }

    fn event(&self, action: AuditAction, reason: ScheduleFailureReason) -> AuditEvent {
        let mut detail = self.detail.clone();
        if let Value::Object(map) = &mut detail {
            map.insert("reason".to_owned(), json!(reason.as_str()));
        }
        let mut event = AuditEvent::new(action, self.object_type);
        event.object_id = Some(self.object_id);
        event.diff = Some(diff(None, Some(&detail), &[]));
        event
    }
}

/// Writes the record of a row that entered the failure state.
pub async fn record_failed(
    tx: &mut TenantTx<'_>,
    target: &Target,
    reason: ScheduleFailureReason,
) -> Result<(), DbError> {
    audit::insert(tx, &target.event(AuditAction::ScheduleFailed, reason)).await?;
    Ok(())
}

/// Writes the record of a row that left the failure state after a successful pass.
pub async fn record_recovered(
    tx: &mut TenantTx<'_>,
    target: &Target,
    previous: &ScheduleFailure,
) -> Result<(), DbError> {
    audit::insert(
        tx,
        &target.event(AuditAction::ScheduleRecovered, previous.reason),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-SC-12: the pause starts at 5 s, doubles and stops at 10 min.
    #[test]
    fn ac_sc_12_the_pause_doubles_up_to_ten_minutes() {
        let pauses: Vec<i64> = (1..=9)
            .map(|attempts| retry_pause(attempts).whole_seconds())
            .collect();
        assert_eq!(pauses, vec![5, 10, 20, 40, 80, 160, 320, 600, 600]);
        assert_eq!(retry_pause(i32::MAX).whole_seconds(), 600);
        assert_eq!(retry_pause(0).whole_seconds(), 5);
    }
}
