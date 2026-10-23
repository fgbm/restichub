//! The recovery point objective of a plan (FR-MN-01, FR-MN-03, ADR-0036).

use restichub_db::dashboard::RpoPlan;
use serde::Serialize;
use time::{Duration, OffsetDateTime};
use utoipa::ToSchema;

use crate::schedule::Schedule;

/// The share of the objective at which a plan counts as at risk, in percent.
pub const AT_RISK_PERCENT: i64 = 75;

/// The objective of a plan whose schedule the control plane cannot compute.
const FALLBACK: Duration = Duration::hours(26);

const MIN_MARGIN: Duration = Duration::minutes(15);

const MAX_MARGIN: Duration = Duration::hours(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RpoState {
    Ok,
    AtRisk,
    Violated,
}

/// The objective of a plan: the stored value, else the schedule interval plus a margin.
///
/// The interval is the time between the next two fire times. The margin is a tenth of the interval,
/// at least 15 minutes and at most 2 hours, so a daily plan gets 26 hours.
pub fn effective_rpo(
    cron: &str,
    time_zone: &str,
    explicit_seconds: Option<i32>,
    now: OffsetDateTime,
) -> Duration {
    if let Some(seconds) = explicit_seconds {
        return Duration::seconds(i64::from(seconds));
    }
    let interval = Schedule::parse(cron, time_zone).ok().and_then(|schedule| {
        let first = schedule.next_after(now).ok()?;
        let second = schedule.next_after(first).ok()?;
        Some(second - first)
    });
    match interval {
        Some(interval) if interval > Duration::ZERO => {
            interval + (interval / 10_i32).clamp(MIN_MARGIN, MAX_MARGIN)
        }
        _ => FALLBACK,
    }
}

pub fn state_of(age: Duration, rpo: Duration) -> RpoState {
    if age > rpo {
        RpoState::Violated
    } else if age.whole_seconds() * 100 >= rpo.whole_seconds() * AT_RISK_PERCENT {
        RpoState::AtRisk
    } else {
        RpoState::Ok
    }
}

/// A plan that the scheduler cannot run counts as at risk, even while its last backup is fresh
/// (FR-SC-12). A violated plan stays violated, so the alert of the plan keeps its rule.
pub fn with_schedule_failure(state: RpoState, failed: bool) -> RpoState {
    match state {
        RpoState::Ok if failed => RpoState::AtRisk,
        other => other,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RpoStatus {
    pub state: RpoState,
    /// The finish time of the last succeeded backup, else the creation time of the plan.
    pub reference: OffsetDateTime,
    pub has_success: bool,
    pub age: Duration,
    pub rpo: Duration,
}

pub fn evaluate(plan: &RpoPlan, now: OffsetDateTime) -> RpoStatus {
    let reference = plan.last_success_at.unwrap_or(plan.created_at);
    let age = (now - reference).max(Duration::ZERO);
    let rpo = effective_rpo(&plan.cron, &plan.time_zone, plan.rpo_seconds, now);
    RpoStatus {
        state: with_schedule_failure(state_of(age, rpo), plan.schedule_failure.is_some()),
        reference,
        has_success: plan.last_success_at.is_some(),
        age,
        rpo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    /// AC-MN-01: a daily plan gets 26 hours, an hourly plan 75 minutes.
    #[test]
    fn ac_mn_01_default_objective_follows_the_interval() {
        let now = datetime!(2026-10-15 08:00 UTC);
        assert_eq!(
            effective_rpo("0 2 * * *", "UTC", None, now),
            Duration::hours(26)
        );
        assert_eq!(
            effective_rpo("0 * * * *", "UTC", None, now),
            Duration::minutes(75)
        );
        assert_eq!(
            effective_rpo("0 2 * * 1", "UTC", None, now),
            Duration::hours(170)
        );
        assert_eq!(
            effective_rpo("0 2 * * *", "UTC", Some(86_400), now),
            Duration::hours(24)
        );
        assert_eq!(
            effective_rpo("not a cron", "UTC", None, now),
            Duration::hours(26)
        );
    }

    /// AC-MN-01
    #[test]
    fn ac_mn_01_states() {
        let rpo = Duration::hours(24);
        assert_eq!(state_of(Duration::hours(1), rpo), RpoState::Ok);
        assert_eq!(state_of(Duration::hours(18), rpo), RpoState::AtRisk);
        assert_eq!(state_of(Duration::hours(24), rpo), RpoState::AtRisk);
        assert_eq!(state_of(Duration::hours(25), rpo), RpoState::Violated);
    }

    /// AC-SC-12: a plan with a failed schedule is at risk, and a violated plan stays violated.
    #[test]
    fn ac_sc_12_a_failed_schedule_raises_ok_to_at_risk() {
        assert_eq!(with_schedule_failure(RpoState::Ok, true), RpoState::AtRisk);
        assert_eq!(with_schedule_failure(RpoState::Ok, false), RpoState::Ok);
        assert_eq!(
            with_schedule_failure(RpoState::Violated, true),
            RpoState::Violated
        );
        assert_eq!(
            with_schedule_failure(RpoState::AtRisk, true),
            RpoState::AtRisk
        );
    }
}
