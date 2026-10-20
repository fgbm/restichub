//! The sinks of the Windows service: the rotated file and the event log.

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use tracing::{Level, Metadata};
use tracing_subscriber::fmt::MakeWriter;

use super::RotatingFile;

/// The Event Log takes at most 31 839 UTF-16 code units in one string.
const MAX_EVENT_UNITS: usize = 31_000;

/// How long a failed sink waits before the service opens it again.
pub(crate) const RETRY_AFTER: Duration = Duration::from_secs(60);

/// The event log gets one copy of an identical event in this window; the file gets each one.
pub(crate) const REPEAT_WINDOW: Duration = Duration::from_secs(60);

/// A receiver of the warnings and errors, the Windows event log in production.
pub(crate) trait EventSink: Send + Sync {
    fn report(&self, level: Level, text: &str) -> std::io::Result<()>;
}

pub(crate) type OpenFile = Box<dyn Fn() -> anyhow::Result<RotatingFile> + Send + Sync>;
pub(crate) type OpenEvents = Box<dyn Fn() -> anyhow::Result<Box<dyn EventSink>> + Send + Sync>;
pub(crate) type Clock = Box<dyn Fn() -> Instant + Send + Sync>;

enum Sink<T> {
    Open(T),
    /// The events that the sink did not get since it failed.
    Down {
        retry_at: Instant,
        lost: u64,
    },
}

pub(super) struct Repeat {
    until: Instant,
    skipped: u64,
}

pub(crate) struct ServiceLog {
    file: Mutex<Sink<RotatingFile>>,
    events: Mutex<Sink<Box<dyn EventSink>>>,
    pub(super) repeats: Mutex<HashMap<u64, Repeat>>,
    open_file: OpenFile,
    open_events: OpenEvents,
    now: Clock,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ServiceLog {
    pub(crate) fn new(open_file: OpenFile, open_events: OpenEvents, now: Clock) -> Self {
        let retry_at = now() + RETRY_AFTER;
        let (file, file_error) = match open_file() {
            Ok(file) => (Sink::Open(file), None),
            Err(error) => (Sink::Down { retry_at, lost: 0 }, Some(error)),
        };
        let (events, events_error) = match open_events() {
            Ok(events) => (Sink::Open(events), None),
            Err(error) => (Sink::Down { retry_at, lost: 0 }, Some(error)),
        };
        let log = Self {
            file: Mutex::new(file),
            events: Mutex::new(events),
            repeats: Mutex::default(),
            open_file,
            open_events,
            now,
        };
        if let Some(error) = file_error {
            log.file_failed(&format!("cannot open the log file: {error:#}"));
        }
        if let Some(error) = events_error {
            log.events_failed(&format!("cannot open the event log: {error:#}"));
        }
        log
    }

    /// Lets the next line open the file sink again before its pause ends, so the warning of a start
    /// repair reaches the file (ADR-0048).
    pub(crate) fn retry_now(&self) {
        let mut sink = lock(&self.file);
        if let Sink::Down { retry_at, .. } = &mut *sink {
            *retry_at = (self.now)();
        }
    }

    /// Writes one formatted event. Each level goes to the file; a warning and an error also go to
    /// the event log.
    pub(crate) fn write(&self, level: Level, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let line = escape(text.trim_end_matches(['\r', '\n']));
        self.write_file(&format!("{line}\n"));
        if matches!(level, Level::ERROR | Level::WARN)
            && let Some(text) = self.admit(&line)
        {
            self.report(level, &text);
        }
    }

    fn write_file(&self, line: &str) {
        let now = (self.now)();
        let mut sink = lock(&self.file);
        let mut note = None;
        if let Sink::Down { retry_at, lost } = &mut *sink {
            if now < *retry_at {
                *lost += 1;
                return;
            }
            match (self.open_file)() {
                Ok(file) => {
                    if *lost > 0 {
                        note = Some(own_line(
                            Level::WARN,
                            &format!(
                                "the log file works again; {lost} events of the outage are not in it"
                            ),
                        ));
                    }
                    *sink = Sink::Open(file);
                }
                Err(_) => {
                    *retry_at = now + RETRY_AFTER;
                    *lost += 1;
                    return;
                }
            }
        }
        let Sink::Open(file) = &mut *sink else {
            return;
        };
        let written = match note {
            Some(note) => file.write(note.as_bytes()),
            None => Ok(()),
        }
        .and_then(|()| file.write(line.as_bytes()));
        let Err(error) = written else {
            return;
        };
        let reason = format!(
            "cannot write the log file in {}: {error}",
            file.dir().display()
        );
        *sink = Sink::Down {
            retry_at: now + RETRY_AFTER,
            lost: 1,
        };
        drop(sink);
        self.file_failed(&reason);
    }

    /// Lets the first of identical events through and counts the rest until the window ends.
    fn admit(&self, line: &str) -> Option<String> {
        let mut hasher = DefaultHasher::new();
        repeat_key(line).hash(&mut hasher);
        let key = hasher.finish();
        let now = (self.now)();
        let mut repeats = lock(&self.repeats);
        // A count that no later identical event reports is dropped one window later, so events
        // with varying values cannot grow the table without bound.
        repeats.retain(|_, repeat| now < repeat.until + REPEAT_WINDOW);
        let skipped = match repeats.get_mut(&key) {
            Some(repeat) if now < repeat.until => {
                repeat.skipped += 1;
                return None;
            }
            Some(repeat) => {
                std::mem::replace(
                    repeat,
                    Repeat {
                        until: now + REPEAT_WINDOW,
                        skipped: 0,
                    },
                )
                .skipped
            }
            None => {
                repeats.insert(
                    key,
                    Repeat {
                        until: now + REPEAT_WINDOW,
                        skipped: 0,
                    },
                );
                0
            }
        };
        Some(if skipped > 0 {
            format!(
                "{line} ({skipped} identical events after the previous one are only in the log file)"
            )
        } else {
            line.to_owned()
        })
    }

    fn report(&self, level: Level, text: &str) {
        let now = (self.now)();
        let mut sink = lock(&self.events);
        if let Sink::Down { retry_at, lost } = &mut *sink {
            if now < *retry_at {
                *lost += 1;
                return;
            }
            match (self.open_events)() {
                Ok(events) => *sink = Sink::Open(events),
                Err(_) => {
                    *retry_at = now + RETRY_AFTER;
                    *lost += 1;
                    return;
                }
            }
        }
        let Sink::Open(events) = &*sink else {
            return;
        };
        let Err(error) = events.report(level, truncate(text)) else {
            return;
        };
        *sink = Sink::Down {
            retry_at: now + RETRY_AFTER,
            lost: 1,
        };
        drop(sink);
        self.events_failed(&format!("cannot write to the event log: {error}"));
    }

    fn file_failed(&self, reason: &str) {
        self.report(
            Level::WARN,
            &escape(&format!(
                "{reason}; the service tries again every {} s, and the warnings and errors still go to the event log",
                RETRY_AFTER.as_secs()
            )),
        );
    }

    fn events_failed(&self, reason: &str) {
        self.write_file(&own_line(
            Level::WARN,
            &format!(
                "{reason}; the service tries again every {} s, and the full log still goes to the log file",
                RETRY_AFTER.as_secs()
            ),
        ));
    }
}

/// A line of the sinks themselves, in the shape of a formatted event.
fn own_line(level: Level, message: &str) -> String {
    let stamp = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    format!(
        "{stamp} {:>5} restichub_agent::logging: {}\n",
        level.as_str(),
        escape(message)
    )
}

/// The event without its time stamp, which differs between otherwise identical events.
fn repeat_key(line: &str) -> &str {
    line.split_once(' ').map_or(line, |(_, rest)| rest)
}

/// A line break or a control character in a value would start a forged line in the file.
pub(crate) fn escape(text: &str) -> Cow<'_, str> {
    let unsafe_char =
        |c: char| (c.is_control() && c != '\t') || matches!(c, '\u{2028}' | '\u{2029}');
    if !text.chars().any(unsafe_char) {
        return Cow::Borrowed(text);
    }
    let mut escaped = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        match c {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            c if unsafe_char(c) => escaped.push_str(&format!("\\u{{{:x}}}", u32::from(c))),
            c => escaped.push(c),
        }
    }
    Cow::Owned(escaped)
}

/// Cuts `text` to the length that the event log takes in one string.
pub(crate) fn truncate(text: &str) -> &str {
    let mut units = 0;
    for (end, c) in text.char_indices() {
        units += c.len_utf16();
        if units > MAX_EVENT_UNITS {
            return &text[..end];
        }
    }
    text
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
