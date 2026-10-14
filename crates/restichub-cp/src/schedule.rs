//! The cron schedule of a plan: the cron expression, the time zone and the daylight saving rule
//! (FR-SC-01, ADR-0010).
//!
//! The module computes the fire times of a plan with `croner` and the IANA time zone of the plan
//! (ADR-0010). A fixed-time expression that falls in a daylight saving gap resolves to the first
//! instant after the clock change, and one that falls in an overlap resolves to its first
//! occurrence, which is the rule of the accepted decision. An interval expression such as
//! `0 * * * *` names no wall-clock time, so it fires in both halves of an overlap.

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

    /// Every unexpired fire time in `[next_run_at, now]`, the latest `cap` (FR-SC-07).
    ///
    /// A fire time at or before `now - max_age` is expired: a job for it would outlive the deadline
    /// at once, so the walk skips it and returns none of those. The result holds at most `cap` fire
    /// times, the latest ones, so a plan that missed thousands of fire times creates no more than
    /// `cap` jobs in one pass and the older fire times are dropped.
    pub fn fire_times_up_to(
        &self,
        next_run_at: OffsetDateTime,
        now: OffsetDateTime,
        cap: usize,
        max_age: time::Duration,
    ) -> Result<Vec<OffsetDateTime>, ScheduleError> {
        let mut times = Vec::new();
        if next_run_at > now || cap == 0 {
            return Ok(times);
        }
        // The first fire time that a job still outlives. `next_run_at` is itself a fire time, so it
        // is the start unless it is already expired.
        let floor = now - max_age;
        let mut cursor = if next_run_at > floor {
            times.push(next_run_at);
            next_run_at
        } else {
            floor
        };
        // Walk the unexpired fire times up to `now`, keeping only the latest `cap`. The walk is
        // bounded by the fire times within `max_age`, so a long downtime is not walked from its
        // start.
        loop {
            let fire = self.next_from(cursor)?;
            if fire > now {
                break;
            }
            times.push(fire);
            if times.len() > cap {
                times.remove(0);
            }
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
    /// A cron expression that names no instant, such as 31 February, is refused at save time
    /// (FR-SC-01). The parser accepts the expression, so the search for the first fire time is the
    /// check.
    #[test]
    fn an_impossible_cron_yields_no_fire_time() {
        let schedule = Schedule::parse("0 0 31 2 *", "UTC").expect("the parser accepts the fields");
        assert_eq!(
            schedule.next_after(utc(2025, Month::January, 1, 0, 0)),
            Err(ScheduleError::Search)
        );
    }

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
        let day = time::Duration::hours(24);
        // An inclusive start plus three further hours gives four fire times.
        let times = schedule
            .fire_times_up_to(
                utc(2025, Month::January, 1, 0, 0),
                utc(2025, Month::January, 1, 3, 0),
                100,
                day,
            )
            .expect("the fire times");
        assert_eq!(times.len(), 4);
        assert_eq!(times[0], utc(2025, Month::January, 1, 0, 0));
        assert_eq!(times[3], utc(2025, Month::January, 1, 3, 0));

        // The cap keeps the latest fire times of one pass.
        let capped = schedule
            .fire_times_up_to(
                utc(2025, Month::January, 1, 0, 0),
                utc(2025, Month::January, 2, 0, 0),
                10,
                day,
            )
            .expect("the fire times");
        assert_eq!(capped.len(), 10);
        assert_eq!(capped[0], utc(2025, Month::January, 1, 15, 0));
        assert_eq!(capped[9], utc(2025, Month::January, 2, 0, 0));

        // A fire time that a job would outlive at once is expired and creates none. The next fire
        // time is itself expired, so the walk starts after it.
        let unexpired = schedule
            .fire_times_up_to(
                utc(2025, Month::January, 1, 0, 0),
                utc(2025, Month::January, 2, 0, 0),
                100,
                time::Duration::hours(3),
            )
            .expect("the fire times");
        assert_eq!(
            unexpired.first(),
            Some(&utc(2025, Month::January, 1, 22, 0))
        );
        assert_eq!(unexpired.last(), Some(&utc(2025, Month::January, 2, 0, 0)));
        assert_eq!(unexpired.len(), 3);
    }

    /// The autumn change puts 02:00 to 03:00 on 2025-10-26 into an overlap. A fixed-time expression
    /// runs once, at the first occurrence; an interval expression names no wall-clock time, so it
    /// runs in both halves of the overlap (FR-SC-01). The two occurrences of 02:00 local are
    /// 00:00 UTC (02:00 CEST) and 01:00 UTC (02:00 CET).
    #[test]
    fn ac_sc_01_an_interval_expression_runs_in_both_halves_of_the_overlap() {
        let schedule = Schedule::parse("0 * * * *", "Europe/Berlin").expect("the schedule parses");
        let runs = schedule
            .next_runs(utc(2025, Month::October, 25, 22, 30), 5)
            .expect("the runs");
        assert_eq!(
            runs,
            vec![
                utc(2025, Month::October, 25, 23, 0),
                utc(2025, Month::October, 26, 0, 0),
                utc(2025, Month::October, 26, 1, 0),
                utc(2025, Month::October, 26, 2, 0),
                utc(2025, Month::October, 26, 3, 0),
            ]
        );
        assert!(
            runs.contains(&utc(2025, Month::October, 26, 0, 0))
                && runs.contains(&utc(2025, Month::October, 26, 1, 0)),
            "both halves of the overlap run"
        );
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
