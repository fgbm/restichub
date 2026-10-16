//! The sinks of the Windows service and the fallback to the standard error.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tracing::{Level, Metadata};
use tracing_subscriber::fmt::MakeWriter;

use super::RotatingFile;

/// The Event Log takes at most 31 839 characters in one string.
const MAX_EVENT_CHARS: usize = 31_000;

/// A receiver of the warnings and errors, the Windows event log in production.
pub(crate) trait EventSink: Send + Sync {
    fn report(&self, level: Level, text: &str) -> std::io::Result<()>;
}

type Fallback = Box<dyn Fn(&[u8]) + Send + Sync>;

pub(crate) struct ServiceLog {
    file: Mutex<Option<RotatingFile>>,
    events: Option<Box<dyn EventSink>>,
    events_failed: AtomicBool,
    fallback: Fallback,
}

impl ServiceLog {
    pub(crate) fn new(
        file: anyhow::Result<RotatingFile>,
        events: anyhow::Result<Box<dyn EventSink>>,
        fallback: Fallback,
    ) -> Self {
        let (file, file_error) = match file {
            Ok(file) => (Some(file), None),
            Err(error) => (None, Some(error)),
        };
        let (events, events_error) = match events {
            Ok(events) => (Some(events), None),
            Err(error) => (None, Some(error)),
        };
        let log = Self {
            file: Mutex::new(file),
            events_failed: AtomicBool::new(events.is_none()),
            events,
            fallback,
        };
        if let Some(error) = file_error {
            log.file_failed(&format!("cannot open the log file: {error:#}"));
        }
        if let Some(error) = events_error {
            log.events_failed(&format!("cannot open the event log: {error:#}"));
        }
        log
    }

    /// Writes one formatted event. Each level goes to the file; a warning and an error also go to
    /// the event log.
    pub(crate) fn write(&self, level: Level, bytes: &[u8]) {
        let mut to_fallback = !self.write_file(bytes);
        if matches!(level, Level::ERROR | Level::WARN) && !self.report(level, bytes) {
            to_fallback = true;
        }
        if to_fallback {
            (self.fallback)(bytes);
        }
    }

    fn lock_file(&self) -> MutexGuard<'_, Option<RotatingFile>> {
        self.file.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_file(&self, bytes: &[u8]) -> bool {
        let mut guard = self.lock_file();
        let Some(file) = guard.as_mut() else {
            return false;
        };
        let Err(error) = file.write(bytes) else {
            return true;
        };
        let reason = format!(
            "cannot write the log file in {}: {error}",
            file.dir().display()
        );
        *guard = None;
        drop(guard);
        self.file_failed(&reason);
        false
    }

    fn report(&self, level: Level, bytes: &[u8]) -> bool {
        if self.events_failed.load(Ordering::Relaxed) {
            return false;
        }
        let Some(events) = &self.events else {
            return false;
        };
        let Err(error) = events.report(level, &event_text(bytes)) else {
            return true;
        };
        if !self.events_failed.swap(true, Ordering::Relaxed) {
            self.events_failed(&format!("cannot write to the event log: {error}"));
        }
        false
    }

    fn file_failed(&self, reason: &str) {
        let warning = format!("restichub-agent: {reason}; the log goes to the standard error\n");
        (self.fallback)(warning.as_bytes());
        self.report(Level::WARN, warning.as_bytes());
    }

    fn events_failed(&self, reason: &str) {
        let warning = format!(
            "restichub-agent: {reason}; the warnings and errors go to the log file and the standard error\n"
        );
        (self.fallback)(warning.as_bytes());
        self.write_file(warning.as_bytes());
    }
}

/// The text of one event without the line end and within the limit of the event log.
pub(crate) fn event_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_end();
    match text.char_indices().nth(MAX_EVENT_CHARS) {
        Some((end, _)) => text[..end].to_owned(),
        None => text.to_owned(),
    }
}

pub(crate) struct ServiceWriter(pub(crate) Arc<ServiceLog>);

impl ServiceWriter {
    fn buffer(&self, level: Level) -> EventBuffer {
        EventBuffer {
            log: Arc::clone(&self.0),
            level,
            bytes: Vec::new(),
        }
    }
}

impl<'a> MakeWriter<'a> for ServiceWriter {
    type Writer = EventBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.buffer(Level::INFO)
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Self::Writer {
        self.buffer(*meta.level())
    }
}

/// Collects one formatted event, so each sink gets the whole event in one piece.
pub(crate) struct EventBuffer {
    log: Arc<ServiceLog>,
    level: Level,
    bytes: Vec<u8>,
}

impl std::io::Write for EventBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for EventBuffer {
    fn drop(&mut self) {
        if !self.bytes.is_empty() {
            self.log.write(self.level, &self.bytes);
        }
    }
}
