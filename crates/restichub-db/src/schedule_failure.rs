//! The failure state that a plan row and a maintenance schedule row share (FR-SC-12).
//!
//! The scheduler records a failed pass on the row. The columns are `schedule_failure`,
//! `failed_since`, `retry_after` and `failure_attempts`; the migration 0060 explains them.

use time::OffsetDateTime;

use crate::enums::ScheduleFailureReason;

/// The failure of the scheduler pass of one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleFailure {
    pub reason: ScheduleFailureReason,
    /// The time of the first failed pass of this failure.
    pub since: OffsetDateTime,
    /// The claim skips the row until this time. `None` for a failure that needs an edit.
    pub retry_after: Option<OffsetDateTime>,
    pub attempts: i32,
}

impl ScheduleFailure {
    /// A row without a reason or without a start time holds no failure.
    pub(crate) fn from_columns(
        reason: Option<ScheduleFailureReason>,
        since: Option<OffsetDateTime>,
        retry_after: Option<OffsetDateTime>,
        attempts: i32,
    ) -> Option<Self> {
        Some(Self {
            reason: reason?,
            since: since?,
            retry_after,
            attempts,
        })
    }
}
