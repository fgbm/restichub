//! The limits of the re-registration of a host (FR-ID-10, FR-ID-11, ADR-0041).

use time::Duration;

/// Overrides [`DEFAULT_REENROLL_TOKEN_MAX_TTL_SECS`].
pub const REENROLL_TOKEN_MAX_TTL_ENV: &str = "RESTICHUB_REENROLL_TOKEN_MAX_TTL_SECS";

/// Overrides [`DEFAULT_REPLACE_ONLINE_WINDOW_SECS`].
pub const REPLACE_ONLINE_WINDOW_ENV: &str = "RESTICHUB_REPLACE_ONLINE_WINDOW_SECS";

/// A token bound to a host takes over an identity, so it lives one hour at most.
pub const DEFAULT_REENROLL_TOKEN_MAX_TTL_SECS: i64 = 60 * 60;

/// An old record with a sign of life inside this window is not replaced.
pub const DEFAULT_REPLACE_ONLINE_WINDOW_SECS: i64 = 5 * 60;

fn seconds(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

/// The highest time to live of a token bound to a host, in seconds.
pub fn reenroll_token_max_ttl_secs() -> i64 {
    seconds(
        REENROLL_TOKEN_MAX_TTL_ENV,
        DEFAULT_REENROLL_TOKEN_MAX_TTL_SECS,
    )
}

pub fn replace_online_window() -> Duration {
    Duration::seconds(seconds(
        REPLACE_ONLINE_WINDOW_ENV,
        DEFAULT_REPLACE_ONLINE_WINDOW_SECS,
    ))
}
