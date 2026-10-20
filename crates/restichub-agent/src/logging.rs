//! Where the agent sends its log.
//!
//! A console command and a Linux service write to the standard error, which journald collects. The
//! Windows service has no standard error that anybody reads, so it writes the full log to a
//! size-rotated file in a protected directory next to the configuration directory and copies each
//! warning and error to the `ResticHub` event log. A sink that fails gives one warning and is opened
//! again after a pause, so the log never stops a job.

use std::path::{Path, PathBuf};

use tracing_subscriber::EnvFilter;

#[cfg(windows)]
mod event_log;
mod rotation;
#[cfg(any(windows, test))]
mod sinks;
#[cfg(test)]
mod tests;

#[cfg(windows)]
pub use event_log::{register_event_source, remove_event_source};
pub use rotation::RotatingFile;

pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

pub const MAX_FILES: usize = 5;

/// The sink of the service log, so the start repair can ask it to try its file again (ADR-0048).
#[cfg(windows)]
static SERVICE_LOG: std::sync::OnceLock<std::sync::Arc<sinks::ServiceLog>> =
    std::sync::OnceLock::new();

/// The directory of the Windows service log, `%ProgramData%\ResticHub\Agent-logs`.
///
/// It is not inside the configuration directory: a new registration replaces that directory with
/// one rename, which an open log file would block.
#[must_use]
pub fn service_log_dir(config: &Path) -> PathBuf {
    crate::identity::sibling(config, "-logs")
}

fn env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

/// Sends the log to the standard error.
pub fn init_console() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter())
        .with_writer(std::io::stderr)
        .try_init();
}

/// Sends the log of the Windows service to the rotated file and the `ResticHub` event log.
#[cfg(windows)]
pub fn init_service() {
    let dir = service_log_dir(&crate::paths::config_dir());
    let log = sinks::ServiceLog::new(
        Box::new(move || {
            crate::paths::create_protected_dir(&dir)?;
            Ok(RotatingFile::open(&dir, MAX_FILE_BYTES, MAX_FILES)?)
        }),
        Box::new(|| Ok(Box::new(event_log::EventSource::open()?) as Box<dyn sinks::EventSink>)),
        Box::new(std::time::Instant::now),
    );
    let log = std::sync::Arc::new(log);
    // The start repair changes the directory of the log, so it asks the sink to try again.
    let _ = SERVICE_LOG.set(std::sync::Arc::clone(&log));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter())
        .with_ansi(false)
        .with_writer(sinks::ServiceWriter(log))
        .try_init();
}

/// Lets the file sink open its file again before the end of its pause, so a warning of the start
/// repair reaches the file (ADR-0048).
#[cfg(windows)]
pub fn retry_log_file() {
    if let Some(log) = SERVICE_LOG.get() {
        log.retry_now();
    }
}
