//! Where the agent sends its log.
//!
//! A console command and a Linux service write to the standard error, which journald collects. The
//! Windows service has no standard error that anybody reads, so it writes the full log to a
//! size-rotated file in the protected `logs` directory and copies each warning and error to the
//! Application event log. A sink that fails is dropped once with one warning, and its lines go to
//! the standard error from then on, so the log never stops a job.

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

/// The directory of the service log below the configuration directory (Windows only).
pub const LOGS_DIR: &str = "logs";

pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

pub const MAX_FILES: usize = 5;

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

/// Sends the log of the Windows service to the rotated file and the Application event log.
#[cfg(windows)]
pub fn init_service() {
    let dir = crate::paths::config_dir().join(LOGS_DIR);
    let file = crate::paths::create_protected_dir(&dir)
        .and_then(|()| Ok(RotatingFile::open(&dir, MAX_FILE_BYTES, MAX_FILES)?));
    let events =
        event_log::EventSource::open().map(|source| Box::new(source) as Box<dyn sinks::EventSink>);
    let log = sinks::ServiceLog::new(file, events, Box::new(write_stderr));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter())
        .with_ansi(false)
        .with_writer(sinks::ServiceWriter(std::sync::Arc::new(log)))
        .try_init();
}

#[cfg(windows)]
fn write_stderr(bytes: &[u8]) {
    use std::io::Write as _;
    let _ = std::io::stderr().write_all(bytes);
}
