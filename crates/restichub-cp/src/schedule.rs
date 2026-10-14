//! The cron schedule of a plan: the cron expression, the time zone and the daylight saving rule
//! (FR-SC-01, ADR-0010).
//!
//! The module computes the fire times of a plan with `croner` and the IANA time zone of the plan
//! (ADR-0010). A fire time that falls in a daylight saving gap resolves to the first instant after
//! the clock change, and a fire time that falls in an overlap resolves to its first occurrence,
//! which is the rule of the accepted decision.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use croner::Cron;
use time::OffsetDateTime;

/// A cron expression with the time zone of a plan.
#[derive(Debug, Clone)]
pub struct Schedule {
    cron: Cron,
    time_zone: Tz,
}

/// Why the control plane refused a schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScheduleError {
    #[error("the cron expression is not valid")]
    Cron,
    #[error("the time zone is not a known IANA name")]
    TimeZone,
    #[error("the search for a fire time did not finish")]
    Search,
}

impl Schedule {
    /// Parses a five-field cron expression and an IANA time zone name.
    ///
    /// The cron expression is checked with the compiler, so a plan and a profile agree on what an
    /// expression is (FR-PF-05).
    pub fn parse(expression: &str, time_zone: &str) -> Result<Self, ScheduleError> {
        restichub_compiler::validate_cron(expression).map_err(|_| ScheduleError::Cron)?;
        let cron = Cron::from_str(expression).map_err(|_| ScheduleError::Cron)?;
        let time_zone = Tz::from_str(time_zone).map_err(|_| ScheduleError::TimeZone)?;
        Ok(Self { cron, time_zone })
    }

    /// The fire time that follows the given instant, in UTC.
    pub fn next_after(&self, after: OffsetDateTime) -> Result<OffsetDateTime, ScheduleError> {
        self.next_from(after)
    }

    /// The next `count` fire times strictly after `after`, in UTC (FR-SC-01).
    pub fn next_runs(
        &self,
        after: OffsetDateTime,
        count: usize,
    ) -> Result<Vec<OffsetDateTime>, ScheduleError> {
        let mut runs = Vec::with_capacity(count);
        let mut cursor = after;
        for _ in 0..count {
            let fire = self.next_from(cursor)?;
            runs.push(fire);
            cursor = fire;
        }
        Ok(runs)
    }

    /// Every fire time in `[next_run_at, now]`, at most `cap` (FR-SC-07).
    ///
    /// `next_run_at` is itself a fire time of the plan, so the result starts with it. The next fire
    /// times follow, each strictly after the one before, until `now` or the cap stops the walk. The
    /// cap bounds the work after a long downtime, so a plan that missed thousands of fire times
    /// creates no more than `cap` jobs in one pass; the fire times beyond the cap are dropped.
    pub fn fire_times_up_to(
        &self,
        next_run_at: OffsetDateTime,
        now: OffsetDateTime,
        cap: usize,
    ) -> Result<Vec<OffsetDateTime>, ScheduleError> {
        let mut times = Vec::new();
        if next_run_at > now || cap == 0 {
            return Ok(times);
        }
        times.push(next_run_at);
        let mut cursor = next_run_at;
        while times.len() < cap {
            let fire = self.next_from(cursor)?;
            if fire > now {
                break;
            }
            times.push(fire);
            cursor = fire;
        }
        Ok(times)
    }

    fn next_from(&self, after: OffsetDateTime) -> Result<OffsetDateTime, ScheduleError> {
        let local = to_chrono(after).with_timezone(&self.time_zone);
        let next = self
            .cron
            .find_next_occurrence(&local, false)
            .map_err(|error| {
                tracing::warn!(%error, "the schedule could not compute the next fire time");
                ScheduleError::Search
            })?;
        Ok(from_chrono(next.with_timezone(&Utc)))
    }
}

fn to_chrono(at: OffsetDateTime) -> DateTime<Utc> {
    DateTime::from_timestamp(at.unix_timestamp(), at.nanosecond()).unwrap_or_default()
}

fn from_chrono(at: DateTime<Utc>) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(
        i128::from(at.timestamp()) * 1_000_000_000 + i128::from(at.timestamp_subsec_nanos()),
    )
    .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use time::{Date, Month, OffsetDateTime, Time};

    use super::{Schedule, ScheduleError};

    /// Builds an instant of the tests without the `time` macros feature.
    fn utc(year: i32, month: Month, day: u8, hour: u8, minute: u8) -> OffsetDateTime {
        let date = Date::from_calendar_date(year, month, day).expect("a valid date");
        let time = Time::from_hms(hour, minute, 0).expect("a valid time");
        date.with_time(time).assume_utc()
    }

    #[test]
    fn a_wrong_cron_or_time_zone_is_refused() {
        assert_eq!(
            Schedule::parse("* * * *", "UTC").unwrap_err(),
            ScheduleError::Cron
        );
        assert_eq!(
            Schedule::parse("* * * * *", "Mars/Olympus").unwrap_err(),
            ScheduleError::TimeZone
        );
        assert!(Schedule::parse("0 2 * * *", "Europe/Moscow").is_ok());
    }

    /// The spring change in Europe/Berlin puts 02:00 to 03:00 on 2025-03-30 into a gap. The run of
    /// `30 2 * * *` occurs once, at the first instant after the change (FR-SC-01).
    #[test]
    fn ac_sc_01_a_fire_time_in_the_spring_gap_runs_after_the_clock_change() {
        let schedule = Schedule::parse("30 2 * * *", "Europe/Berlin").expect("the schedule parses");
        let next = schedule
            .next_after(utc(2025, Month::March, 29, 12, 0))
            .expect("a next fire time");
        assert_eq!(next, utc(2025, Month::March, 30, 1, 0));
    }

    /// The autumn change puts 02:00 to 03:00 on 2025-10-26 into an overlap. The run occurs once, at
    /// the first occurrence (FR-SC-01).
    #[test]
    fn ac_sc_01_a_fire_time_in_the_autumn_overlap_runs_once_at_the_first_occurrence() {
        let schedule = Schedule::parse("30 2 * * *", "Europe/Berlin").expect("the schedule parses");
        let next = schedule
            .next_after(utc(2025, Month::October, 25, 12, 0))
            .expect("a next fire time");
        assert_eq!(next, utc(2025, Month::October, 26, 0, 30));
    }

    /// Europe/Moscow does not observe daylight saving time, so 02:00 local is 23:00 UTC of the day
    /// before (AC-SC-01).
    #[test]
    fn ac_sc_01_moscow_runs_at_the_local_wall_clock() {
        let schedule = Schedule::parse("0 2 * * *", "Europe/Moscow").expect("the schedule parses");
        let next = schedule
            .next_after(utc(2025, Month::June, 1, 0, 0))
            .expect("a next fire time");
        assert_eq!(next, utc(2025, Month::June, 1, 23, 0));
    }

    #[test]
    fn the_missed_fire_times_stop_at_now_and_at_the_cap() {
        let schedule = Schedule::parse("0 * * * *", "UTC").expect("the schedule parses");
        // An inclusive start plus three further hours gives four fire times.
        let times = schedule
            .fire_times_up_to(
                utc(2025, Month::January, 1, 0, 0),
                utc(2025, Month::January, 1, 3, 0),
                100,
            )
            .expect("the fire times");
        assert_eq!(times.len(), 4);
        assert_eq!(times[0], utc(2025, Month::January, 1, 0, 0));
        assert_eq!(times[3], utc(2025, Month::January, 1, 3, 0));

        // The cap keeps one pass short after a long downtime.
        let capped = schedule
            .fire_times_up_to(
                utc(2025, Month::January, 1, 0, 0),
                utc(2025, Month::January, 2, 0, 0),
                10,
            )
            .expect("the fire times");
        assert_eq!(capped.len(), 10);
    }

    #[test]
    fn the_next_runs_preview_is_in_the_plan_time_zone() {
        let schedule = Schedule::parse("0 2 * * *", "Europe/Moscow").expect("the schedule parses");
        let runs = schedule
            .next_runs(utc(2025, Month::June, 1, 0, 0), 3)
            .expect("the next runs");
        assert_eq!(
            runs,
            vec![
                utc(2025, Month::June, 1, 23, 0),
                utc(2025, Month::June, 2, 23, 0),
                utc(2025, Month::June, 3, 23, 0),
            ]
        );
    }
}
