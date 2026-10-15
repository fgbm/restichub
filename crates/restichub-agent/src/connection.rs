//! The connection status file that the service writes for `status` and `register` (FR-AG-01,
//! FR-IN-07).
//!
//! The file holds times, the control plane URL and the last error text. It never holds a key, a
//! token or a certificate (ADR-0032), and it lives in the protected configuration directory.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub const STATUS_FILE: &str = "status.json";

const MAX_ERROR_CHARS: usize = 500;

/// What the service last knew about its connection to the control plane.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionStatus {
    /// True while the stream is open. A stopped service cannot clear it, so a reader checks the
    /// service state too.
    pub connected: bool,
    pub cp_url: String,
    /// Unix seconds of the start of the last open stream.
    pub connected_at: Option<i64>,
    pub last_error: Option<String>,
    /// Unix seconds of the last error.
    pub last_error_at: Option<i64>,
}

impl ConnectionStatus {
    /// Reads the status file, or `None` when the service never wrote one.
    ///
    /// # Errors
    ///
    /// Returns an error when the file exists and cannot be read or parsed.
    pub fn read(dir: &Path) -> Result<Option<Self>> {
        let path = dir.join(STATUS_FILE);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", path.display()));
            }
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .with_context(|| format!("bad {}", path.display()))
    }

    fn write(&self, dir: &Path) -> Result<()> {
        let json = serde_json::to_vec_pretty(self).context("cannot serialize the status")?;
        crate::paths::replace_file(&dir.join(STATUS_FILE), &json)
    }

    #[must_use]
    pub fn connected_time(&self) -> Option<OffsetDateTime> {
        self.connected_at
            .and_then(|seconds| OffsetDateTime::from_unix_timestamp(seconds).ok())
    }

    #[must_use]
    pub fn error_time(&self) -> Option<OffsetDateTime> {
        self.last_error_at
            .and_then(|seconds| OffsetDateTime::from_unix_timestamp(seconds).ok())
    }
}

/// Records that the stream is open. A write failure is logged and never stops the agent.
pub fn record_connected(dir: &Path, cp_url: &str) {
    update(dir, cp_url, |status| {
        status.connected = true;
        status.connected_at = Some(OffsetDateTime::now_utc().unix_timestamp());
        status.last_error = None;
        status.last_error_at = None;
    });
}

/// Clears the connected flag when the stream ends cleanly; the last error stays.
pub fn record_closed(dir: &Path, cp_url: &str) {
    update(dir, cp_url, |status| status.connected = false);
}

/// Records why the agent is not connected.
pub fn record_failure(dir: &Path, cp_url: &str, error: &str) {
    let text: String = error.chars().take(MAX_ERROR_CHARS).collect();
    update(dir, cp_url, |status| {
        status.connected = false;
        status.last_error = Some(text);
        status.last_error_at = Some(OffsetDateTime::now_utc().unix_timestamp());
    });
}

fn update(dir: &Path, cp_url: &str, change: impl FnOnce(&mut ConnectionStatus)) {
    // A damaged file is replaced, because the next write is the only way to repair it.
    let mut status = ConnectionStatus::read(dir)
        .ok()
        .flatten()
        .unwrap_or_default();
    if !cp_url.is_empty() {
        cp_url.clone_into(&mut status.cp_url);
    }
    change(&mut status);
    if let Err(error) = status.write(dir) {
        tracing::warn!(%error, "cannot write the connection status file");
    }
}

/// The outcome of the wait; a timeout carries the last status the service wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wait {
    Connected(ConnectionStatus),
    /// The time ran out. The status holds the last error, when the service wrote one.
    TimedOut(Option<ConnectionStatus>),
}

/// Waits until the service reports a stream that opened at or after `since`.
///
/// A status from before `since` belongs to an earlier run of the service, so it does not count.
#[must_use]
pub fn wait_for_connection(
    dir: &Path,
    since: OffsetDateTime,
    timeout: Duration,
    poll: Duration,
) -> Wait {
    let deadline = Instant::now() + timeout;
    loop {
        let status = ConnectionStatus::read(dir).ok().flatten();
        if let Some(status) = &status
            && status.connected
            && status
                .connected_time()
                .is_some_and(|at| at.unix_timestamp() >= since.unix_timestamp())
        {
            return Wait::Connected(status.clone());
        }
        if Instant::now() >= deadline {
            return Wait::TimedOut(status.filter(|status| {
                status
                    .error_time()
                    .is_some_and(|at| at.unix_timestamp() >= since.unix_timestamp())
            }));
        }
        std::thread::sleep(poll);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "restichub-agent-status-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        dir
    }

    #[test]
    fn fr_ag_01_the_status_file_round_trips() {
        let dir = temp_dir("round-trip");
        assert_eq!(ConnectionStatus::read(&dir).expect("read"), None);
        record_failure(&dir, "https://cp.example", "the handshake failed");
        let failed = ConnectionStatus::read(&dir).expect("read").expect("file");
        assert!(!failed.connected);
        assert_eq!(failed.last_error.as_deref(), Some("the handshake failed"));
        record_connected(&dir, "https://cp.example");
        let connected = ConnectionStatus::read(&dir).expect("read").expect("file");
        assert!(connected.connected);
        assert!(connected.connected_at.is_some());
        assert_eq!(connected.last_error, None);
        assert_eq!(connected.cp_url, "https://cp.example");
        record_closed(&dir, "https://cp.example");
        let closed = ConnectionStatus::read(&dir).expect("read").expect("file");
        assert!(!closed.connected);
        assert_eq!(closed.connected_at, connected.connected_at);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn adr_0032_the_status_file_holds_only_the_known_fields() {
        let dir = temp_dir("fields");
        record_connected(&dir, "https://cp.example");
        let text = std::fs::read_to_string(dir.join(STATUS_FILE)).expect("file");
        let value: serde_json::Value = serde_json::from_str(&text).expect("json");
        let mut keys: Vec<&str> = value
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "connected",
                "connected_at",
                "cp_url",
                "last_error",
                "last_error_at"
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fr_ag_01_a_long_error_is_cut() {
        let dir = temp_dir("long-error");
        record_failure(&dir, "https://cp.example", &"x".repeat(5000));
        let status = ConnectionStatus::read(&dir).expect("read").expect("file");
        assert_eq!(status.last_error.expect("error").len(), MAX_ERROR_CHARS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fr_in_07_a_connection_from_before_the_start_does_not_count() {
        let dir = temp_dir("stale");
        record_connected(&dir, "https://cp.example");
        let later = OffsetDateTime::now_utc() + Duration::from_secs(60);
        let wait = wait_for_connection(
            &dir,
            later,
            Duration::from_millis(50),
            Duration::from_millis(10),
        );
        assert!(matches!(wait, Wait::TimedOut(None)));
        let earlier = OffsetDateTime::now_utc() - Duration::from_secs(60);
        let wait = wait_for_connection(
            &dir,
            earlier,
            Duration::from_millis(50),
            Duration::from_millis(10),
        );
        assert!(matches!(wait, Wait::Connected(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fr_in_07_a_timeout_reports_the_error_of_this_run() {
        let dir = temp_dir("timeout-error");
        record_failure(&dir, "https://cp.example", "certificate not yet valid");
        let since = OffsetDateTime::now_utc() - Duration::from_secs(5);
        let wait = wait_for_connection(
            &dir,
            since,
            Duration::from_millis(50),
            Duration::from_millis(10),
        );
        let Wait::TimedOut(Some(status)) = wait else {
            panic!("expected a timeout with the error");
        };
        assert_eq!(
            status.last_error.as_deref(),
            Some("certificate not yet valid")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
