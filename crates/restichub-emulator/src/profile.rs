//! Load profile of a bench S2 run (NFR-PERF-04).
//!
//! The file is JSON, like the scenario file of `restichub-fake-restic`: the
//! workspace has no YAML crate (ADR-0019 keeps `serde-saphyr` inside the control
//! plane). The runner reads the file with `--profile`; the crate README
//! documents the format. The profile drives the agent side only: the control
//! plane owns the schedule and the job rate (ADR-0002).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, bail};
use serde::Deserialize;

/// The control plane stores at most 8192 characters of one log line.
pub const MAX_LOG_LINE_BYTES: usize = 8192;

/// The agents, the rate and the failures of one run.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoadProfile {
    pub agents: usize,
    pub duration_seconds: f64,
    pub ramp_up_per_second: f64,
    pub os_mix: BTreeMap<String, f64>,
    pub job_duration: JobDuration,
    pub progress_rate: f64,
    pub failure_rate: BTreeMap<String, f64>,
    pub reconnect_storm: ReconnectStorm,
    pub clock_skew_seconds: f64,
    pub ack_delay_ms: u64,
    pub log_lines_per_second: f64,
    pub log_line_bytes: usize,
    pub hostile: Hostile,
}

impl Default for LoadProfile {
    fn default() -> Self {
        Self {
            agents: 1,
            duration_seconds: 60.0,
            ramp_up_per_second: 0.0,
            os_mix: BTreeMap::new(),
            job_duration: JobDuration::default(),
            progress_rate: 0.0,
            failure_rate: BTreeMap::new(),
            reconnect_storm: ReconnectStorm::default(),
            clock_skew_seconds: 0.0,
            ack_delay_ms: 0,
            log_lines_per_second: 0.0,
            log_line_bytes: 120,
            hostile: Hostile::default(),
        }
    }
}

/// Misbehaviour of the first `agents` agents, for the hostile-agent scenario. The control plane
/// must refuse every such message.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hostile {
    pub agents: usize,
    /// The chance that a job result is followed by a result for a job that the agent never got.
    pub forged_result_share: f64,
    /// Progress messages for unknown jobs, sent each second over each hostile stream.
    pub flood_per_second: f64,
    /// One message of this size, once for each hostile agent.
    pub oversize: Option<Oversize>,
}

/// The size of the one oversize message, relative to the protocol limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Oversize {
    AtLimit,
    AboveLimit,
}

/// Bounds of the uniform step time.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JobDuration {
    pub min_seconds: f64,
    pub max_seconds: f64,
}

impl Default for JobDuration {
    fn default() -> Self {
        Self {
            min_seconds: 0.1,
            max_seconds: 0.1,
        }
    }
}

/// A drop of a share of the streams at one time (FR-ID-08).
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReconnectStorm {
    pub at_seconds: f64,
    pub percent: f64,
}

impl LoadProfile {
    /// Reads and validates a profile file.
    ///
    /// # Errors
    ///
    /// Returns an error for a read, a parse or a validation failure.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read the profile {}", path.display()))?;
        let profile: Self = serde_json::from_str(&text)
            .with_context(|| format!("cannot parse the profile {}", path.display()))?;
        profile.validate()?;
        Ok(profile)
    }

    /// Refuses a profile that cannot drive a run.
    ///
    /// # Errors
    ///
    /// The error names the offending field and its value.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.agents == 0 {
            bail!("agents must be at least 1");
        }
        if !(self.duration_seconds.is_finite() && self.duration_seconds > 0.0) {
            bail!(
                "duration_seconds must be a positive number, got {}",
                self.duration_seconds
            );
        }
        if !(self.ramp_up_per_second.is_finite() && self.ramp_up_per_second >= 0.0) {
            bail!(
                "ramp_up_per_second must be zero or more, got {}",
                self.ramp_up_per_second
            );
        }
        for (label, weight) in &self.os_mix {
            if !(weight.is_finite() && *weight > 0.0) {
                bail!("the os_mix weight of {label} must be positive, got {weight}");
            }
        }
        if !(self.job_duration.min_seconds.is_finite() && self.job_duration.min_seconds >= 0.0) {
            bail!(
                "job_duration.min_seconds must be zero or more, got {}",
                self.job_duration.min_seconds
            );
        }
        if !(self.job_duration.max_seconds.is_finite()
            && self.job_duration.max_seconds >= self.job_duration.min_seconds)
        {
            bail!(
                "job_duration.max_seconds must be at least min_seconds, got {}",
                self.job_duration.max_seconds
            );
        }
        if !(self.progress_rate.is_finite() && self.progress_rate >= 0.0) {
            bail!(
                "progress_rate must be zero or more, got {}",
                self.progress_rate
            );
        }
        let mut total = 0.0;
        for (code, probability) in &self.failure_rate {
            if code.parse::<i32>().is_err() {
                bail!("the failure_rate key {code} is not an exit code");
            }
            if !(probability.is_finite() && *probability > 0.0 && *probability <= 1.0) {
                bail!(
                    "the failure_rate probability of {code} must be in (0, 1], got {probability}"
                );
            }
            total += probability;
        }
        if total > 1.0 + f64::EPSILON {
            bail!("the failure_rate probabilities sum to {total}, above 1");
        }
        if !(self.reconnect_storm.at_seconds.is_finite() && self.reconnect_storm.at_seconds >= 0.0)
        {
            bail!(
                "reconnect_storm.at_seconds must be zero or more, got {}",
                self.reconnect_storm.at_seconds
            );
        }
        if !(self.reconnect_storm.percent.is_finite()
            && (0.0..=100.0).contains(&self.reconnect_storm.percent))
        {
            bail!(
                "reconnect_storm.percent must be in [0, 100], got {}",
                self.reconnect_storm.percent
            );
        }
        if !self.clock_skew_seconds.is_finite() {
            bail!(
                "clock_skew_seconds must be a number, got {}",
                self.clock_skew_seconds
            );
        }
        if !(self.log_lines_per_second.is_finite() && self.log_lines_per_second >= 0.0) {
            bail!(
                "log_lines_per_second must be zero or more, got {}",
                self.log_lines_per_second
            );
        }
        if !(1..=MAX_LOG_LINE_BYTES).contains(&self.log_line_bytes) {
            bail!(
                "log_line_bytes must be in [1, {MAX_LOG_LINE_BYTES}], got {}",
                self.log_line_bytes
            );
        }
        let hostile = &self.hostile;
        if hostile.agents > self.agents {
            bail!(
                "hostile.agents must be at most agents ({}), got {}",
                self.agents,
                hostile.agents
            );
        }
        if !(hostile.forged_result_share.is_finite()
            && (0.0..=1.0).contains(&hostile.forged_result_share))
        {
            bail!(
                "hostile.forged_result_share must be in [0, 1], got {}",
                hostile.forged_result_share
            );
        }
        if !(hostile.flood_per_second.is_finite() && hostile.flood_per_second >= 0.0) {
            bail!(
                "hostile.flood_per_second must be zero or more, got {}",
                hostile.flood_per_second
            );
        }
        Ok(())
    }

    /// Length of the run.
    #[must_use]
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.duration_seconds)
    }

    /// Chooses a reporting label for one agent from `os_mix`. `None` keeps the
    /// host OS.
    #[must_use]
    pub fn os_label(&self, fraction: f64) -> Option<String> {
        if self.os_mix.is_empty() {
            return None;
        }
        let total: f64 = self.os_mix.values().sum();
        let target = fraction * total;
        let mut accumulated = 0.0;
        for (label, weight) in &self.os_mix {
            accumulated += weight;
            if target < accumulated {
                return Some(label.clone());
            }
        }
        self.os_mix.keys().next_back().cloned()
    }
}

/// Start delay of agent `index` for a connect rate of `per_second` agents.
///
/// A rate of zero or less connects every agent at once.
#[must_use]
pub fn ramp_delay(index: usize, per_second: f64) -> Duration {
    if per_second <= 0.0 {
        return Duration::ZERO;
    }
    Duration::from_secs_f64(index as f64 / per_second)
}

/// Reports whether agent `index` drops its stream in a storm of `percent`.
///
/// The selection is by index, so a run is reproducible.
#[must_use]
pub fn storm_selects(index: usize, agents: usize, percent: f64) -> bool {
    if percent <= 0.0 || agents == 0 {
        return false;
    }
    let selected = (percent / 100.0 * agents as f64).round() as usize;
    index < selected
}

#[cfg(test)]
mod tests {
    use super::{LoadProfile, ramp_delay, storm_selects};
    use std::time::Duration;

    #[test]
    fn the_ramp_delay_follows_the_connect_rate() {
        assert_eq!(ramp_delay(0, 10.0), Duration::ZERO);
        assert_eq!(ramp_delay(10, 10.0), Duration::from_secs(1));
        assert_eq!(ramp_delay(25, 10.0), Duration::from_millis(2_500));
    }

    #[test]
    fn a_zero_rate_connects_every_agent_at_once() {
        assert_eq!(ramp_delay(50, 0.0), Duration::ZERO);
    }

    #[test]
    fn the_storm_selects_the_configured_share() {
        assert_eq!((0..50).filter(|i| storm_selects(*i, 50, 20.0)).count(), 10);
        assert_eq!((0..50).filter(|i| storm_selects(*i, 50, 0.0)).count(), 0);
        assert_eq!((0..50).filter(|i| storm_selects(*i, 50, 100.0)).count(), 50);
    }

    #[test]
    fn the_log_and_hostile_fields_are_read_and_checked() {
        let profile: LoadProfile = serde_json::from_str(
            r#"{"agents": 4, "log_lines_per_second": 50, "log_line_bytes": 200,
                "hostile": {"agents": 1, "forged_result_share": 0.5,
                            "flood_per_second": 1000, "oversize": "above_limit"}}"#,
        )
        .expect("profile");
        profile.validate().expect("valid profile");
        assert_eq!(profile.hostile.oversize, Some(super::Oversize::AboveLimit));

        let mut bad = profile.clone();
        bad.hostile.agents = 5;
        assert!(bad.validate().is_err());
        let mut bad = profile.clone();
        bad.hostile.forged_result_share = 1.5;
        assert!(bad.validate().is_err());
        let mut bad = profile;
        bad.log_line_bytes = 0;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn an_unknown_field_is_refused() {
        let error = serde_json::from_str::<LoadProfile>(r#"{"agent": 3}"#).unwrap_err();

        assert!(error.to_string().contains("agent"), "{error}");
    }
}
