//! The recovery point objective of a plan (FR-MN-01, FR-MN-03, ADR-0035).

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
        state: state_of(age, rpo),
        reference,
        has_success: plan.last_success_at.is_some(),
        age,
        rpo,
    }
}

/// A short text for hours and minutes, such as `26 h` or `45 min`.
pub fn human(duration: Duration) -> String {
    let minutes = duration.whole_minutes();
    if minutes >= 120 {
        format!("{} h", minutes / 60)
    } else {
        format!("{minutes} min")
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
}
