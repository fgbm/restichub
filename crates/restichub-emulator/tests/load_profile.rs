//! Load-profile parsing and validation (NFR-PERF-04).
#![allow(clippy::expect_used)]

use std::path::PathBuf;

use restichub_emulator::profile::{LoadProfile, ramp_delay, storm_selects};

fn parse(json: &str) -> LoadProfile {
    serde_json::from_str(json).expect("parse the profile")
}

fn write_temp(name: &str, body: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "restichub-emulator-profile-{}-{name}.json",
        std::process::id()
    ));
    std::fs::write(&path, body).expect("write the profile");
    path
}

#[test]
fn a_full_profile_parses_into_its_fields() {
    let profile = parse(
        r#"{
            "agents": 500,
            "duration_seconds": 14400,
            "ramp_up_per_second": 25,
            "os_mix": {"linux": 0.9, "windows": 0.1},
            "job_duration": {"min_seconds": 5, "max_seconds": 30},
            "progress_rate": 2,
            "failure_rate": {"1": 0.02, "3": 0.05},
            "reconnect_storm": {"at_seconds": 60, "percent": 20},
            "clock_skew_seconds": 1.5,
            "ack_delay_ms": 250
        }"#,
    );
    profile.validate().expect("a valid profile");

    assert_eq!(profile.agents, 500);
    assert_eq!(profile.duration_seconds, 14_400.0);
    assert_eq!(profile.ramp_up_per_second, 25.0);
    assert_eq!(profile.os_mix.get("linux"), Some(&0.9));
    assert_eq!(profile.job_duration.min_seconds, 5.0);
    assert_eq!(profile.job_duration.max_seconds, 30.0);
    assert_eq!(profile.failure_rate.get("3"), Some(&0.05));
    assert_eq!(profile.reconnect_storm.percent, 20.0);
    assert_eq!(profile.clock_skew_seconds, 1.5);
    assert_eq!(profile.ack_delay_ms, 250);
}

#[test]
fn a_missing_field_takes_the_default() {
    let profile = parse("{}");
    profile.validate().expect("the default profile");

    assert_eq!(profile.agents, 1);
    assert!(profile.os_mix.is_empty());
    assert_eq!(profile.job_duration.min_seconds, 0.1);
}

#[test]
fn load_reads_and_validates_a_file() {
    let path = write_temp("valid", r#"{"agents": 50, "duration_seconds": 10}"#);
    let profile = LoadProfile::load(&path).expect("load the profile");
    let _ = std::fs::remove_file(&path);

    assert_eq!(profile.agents, 50);
    assert_eq!(profile.duration_seconds, 10.0);
}

#[test]
fn a_profile_with_no_agent_is_refused() {
    let error = parse(r#"{"agents": 0}"#).validate().unwrap_err();

    assert!(error.to_string().contains("agents"), "{error}");
}

#[test]
fn a_non_positive_duration_is_refused() {
    let error = parse(r#"{"duration_seconds": 0}"#).validate().unwrap_err();

    assert!(error.to_string().contains("duration_seconds"), "{error}");
}

#[test]
fn a_negative_ramp_is_refused() {
    let error = parse(r#"{"ramp_up_per_second": -1}"#)
        .validate()
        .unwrap_err();

    assert!(error.to_string().contains("ramp_up_per_second"), "{error}");
}

#[test]
fn a_non_positive_os_weight_is_refused() {
    let error = parse(r#"{"os_mix": {"linux": 0}}"#).validate().unwrap_err();

    assert!(error.to_string().contains("os_mix"), "{error}");
}

#[test]
fn a_reversed_job_duration_is_refused() {
    let error = parse(r#"{"job_duration": {"min_seconds": 10, "max_seconds": 5}}"#)
        .validate()
        .unwrap_err();

    assert!(error.to_string().contains("job_duration"), "{error}");
}

#[test]
fn a_bad_failure_key_is_refused() {
    let error = parse(r#"{"failure_rate": {"nope": 0.1}}"#)
        .validate()
        .unwrap_err();

    assert!(error.to_string().contains("failure_rate"), "{error}");
}

#[test]
fn a_failure_probability_above_one_is_refused() {
    let error = parse(r#"{"failure_rate": {"1": 1.5}}"#)
        .validate()
        .unwrap_err();

    assert!(error.to_string().contains("failure_rate"), "{error}");
}

#[test]
fn failure_probabilities_above_one_in_total_are_refused() {
    let error = parse(r#"{"failure_rate": {"1": 0.6, "3": 0.6}}"#)
        .validate()
        .unwrap_err();

    assert!(error.to_string().contains("failure_rate"), "{error}");
}

#[test]
fn a_storm_percent_above_one_hundred_is_refused() {
    let error = parse(r#"{"reconnect_storm": {"at_seconds": 5, "percent": 120}}"#)
        .validate()
        .unwrap_err();

    assert!(error.to_string().contains("reconnect_storm"), "{error}");
}

#[test]
fn an_unknown_field_is_refused() {
    let error = serde_json::from_str::<LoadProfile>(r#"{"agents": 1, "typo": 2}"#).unwrap_err();

    assert!(error.to_string().contains("typo"), "{error}");
}

#[test]
fn the_ramp_up_rate_is_respected() {
    assert_eq!(ramp_delay(0, 20.0).as_secs_f64(), 0.0);
    assert_eq!(ramp_delay(20, 20.0).as_secs_f64(), 1.0);
    assert_eq!(ramp_delay(50, 20.0).as_secs_f64(), 2.5);
    assert_eq!(ramp_delay(50, 0.0), std::time::Duration::ZERO);
}

#[test]
fn the_storm_selects_the_configured_share() {
    let selected = (0..500)
        .filter(|index| storm_selects(*index, 500, 20.0))
        .count();
    assert_eq!(selected, 100);
    assert_eq!(
        (0..500)
            .filter(|index| storm_selects(*index, 500, 0.0))
            .count(),
        0
    );
    assert_eq!(
        (0..500)
            .filter(|index| storm_selects(*index, 500, 100.0))
            .count(),
        500
    );
}
