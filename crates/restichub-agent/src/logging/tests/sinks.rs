#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tracing::Level;

use super::super::RotatingFile;
use super::super::sinks::{
    EventSink, REPEAT_WINDOW, RETRY_AFTER, ServiceLog, ServiceWriter, escape, truncate,
};
use super::{read, read_all, temp_dir};

type Events = Arc<Mutex<Vec<(Level, String)>>>;

struct FakeEvents {
    seen: Events,
    fail: Arc<AtomicBool>,
}

impl EventSink for FakeEvents {
    fn report(&self, level: Level, text: &str) -> std::io::Result<()> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("the event log is full"));
        }
        self.seen
            .lock()
            .expect("cannot lock")
            .push((level, text.to_owned()));
        Ok(())
    }
}

struct Sinks {
    dir: PathBuf,
    log: Arc<ServiceLog>,
    events: Events,
    events_fail: Arc<AtomicBool>,
    file_fail: Arc<AtomicBool>,
    clock: Arc<Mutex<Instant>>,
}

fn sinks(name: &str, max_bytes: u64) -> Sinks {
    sinks_with(name, max_bytes, false)
}

fn sinks_with(name: &str, max_bytes: u64, file_fails_to_open: bool) -> Sinks {
    let dir = temp_dir(name);
    let events: Events = Arc::default();
    let events_fail = Arc::new(AtomicBool::new(false));
    let file_fail = Arc::new(AtomicBool::new(file_fails_to_open));
    let clock = Arc::new(Mutex::new(Instant::now()));
    let log = ServiceLog::new(
        {
            let dir = dir.clone();
            let fail = Arc::clone(&file_fail);
            Box::new(move || {
                if fail.load(Ordering::SeqCst) {
                    anyhow::bail!("the directory is not protected");
                }
                Ok(RotatingFile::open(&dir, max_bytes, 5)?)
            })
        },
        {
            let seen = Arc::clone(&events);
            let fail = Arc::clone(&events_fail);
            Box::new(move || {
                Ok(Box::new(FakeEvents {
                    seen: Arc::clone(&seen),
                    fail: Arc::clone(&fail),
                }) as Box<dyn EventSink>)
            })
        },
        {
            let clock = Arc::clone(&clock);
            Box::new(move || *clock.lock().expect("cannot lock"))
        },
    );
    Sinks {
        dir,
        log: Arc::new(log),
        events,
        events_fail,
        file_fail,
        clock,
    }
}

impl Sinks {
    fn events(&self) -> Vec<(Level, String)> {
        self.events.lock().expect("cannot lock").clone()
    }

    fn advance(&self, by: Duration) {
        *self.clock.lock().expect("cannot lock") += by;
    }
}

impl Drop for Sinks {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn ac_ag_03_info_goes_to_the_file_and_a_warning_or_an_error_also_to_the_event_log() {
    let sinks = sinks("select", 1024);
    sinks.log.write(Level::INFO, b"connected\n");
    sinks.log.write(Level::WARN, b"slow\n");
    sinks.log.write(Level::ERROR, b"stopped\n");
    sinks.log.write(Level::DEBUG, b"detail\n");
    assert_eq!(
        read(&sinks.dir, "agent-000001.log"),
        "connected\nslow\nstopped\ndetail\n"
    );
    assert_eq!(
        sinks.events(),
        [
            (Level::WARN, "slow".to_owned()),
            (Level::ERROR, "stopped".to_owned())
        ]
    );
}

#[test]
fn a_failed_file_gives_one_warning_and_is_opened_again_after_the_pause() {
    let sinks = sinks("file-fails", 8);
    sinks.log.write(Level::INFO, b"first\n");
    // A directory in the place of the next file makes the rotation fail.
    let obstacle = sinks.dir.join("agent-000002.log");
    std::fs::create_dir(&obstacle).expect("cannot create the obstacle");
    sinks.log.write(Level::INFO, b"second\n");
    sinks.log.write(Level::WARN, b"t3 third\n");
    sinks.advance(RETRY_AFTER / 2);
    sinks.log.write(Level::INFO, b"still out\n");
    std::fs::remove_dir(&obstacle).expect("cannot remove the obstacle");
    sinks.advance(RETRY_AFTER);
    sinks.log.write(Level::INFO, b"fourth\n");
    sinks.log.write(Level::INFO, b"fifth\n");

    let events = sinks.events();
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(
        events[0].1.contains("cannot write the log file") && events[0].1.contains("tries again"),
        "{events:?}"
    );
    assert_eq!(events[1], (Level::WARN, "t3 third".to_owned()));
    let file = read_all(&sinks.dir);
    assert!(file.starts_with("first\n"), "{file}");
    assert!(
        file.contains("the log file works again; 3 events of the outage are not in it"),
        "{file}"
    );
    assert!(file.ends_with("fourth\nfifth\n"), "{file}");
    assert!(
        !file.contains("second") && !file.contains("still out"),
        "{file}"
    );
}

#[test]
fn a_failed_event_log_gives_one_warning_and_is_opened_again_after_the_pause() {
    let sinks = sinks("events-fail", 1024);
    sinks.events_fail.store(true, Ordering::SeqCst);
    sinks.log.write(Level::WARN, b"t1 one\n");
    sinks.log.write(Level::ERROR, b"t2 two\n");
    sinks.events_fail.store(false, Ordering::SeqCst);
    sinks.log.write(Level::ERROR, b"t3 before the pause\n");
    sinks.advance(RETRY_AFTER);
    sinks.log.write(Level::ERROR, b"t4 after the pause\n");

    let file = read(&sinks.dir, "agent-000001.log");
    assert_eq!(
        file.matches("cannot write to the event log").count(),
        1,
        "{file}"
    );
    for line in [
        "t1 one\n",
        "t2 two\n",
        "t3 before the pause\n",
        "t4 after the pause\n",
    ] {
        assert!(file.contains(line), "{file}");
    }
    assert_eq!(
        sinks.events(),
        [(Level::ERROR, "t4 after the pause".to_owned())]
    );
}

#[test]
fn a_file_that_cannot_open_is_reported_once_and_opened_again_later() {
    let sinks = sinks_with("file-opens-late", 1024, true);
    sinks.log.write(Level::INFO, b"connected\n");
    sinks.advance(RETRY_AFTER);
    sinks.log.write(Level::INFO, b"still out\n");
    sinks.file_fail.store(false, Ordering::SeqCst);
    sinks.advance(RETRY_AFTER);
    sinks.log.write(Level::INFO, b"back\n");

    let events = sinks.events();
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(
        events[0].1.contains("cannot open the log file")
            && events[0].1.contains("the directory is not protected"),
        "{events:?}"
    );
    let file = read_all(&sinks.dir);
    assert!(
        file.contains("the log file works again; 2 events of the outage are not in it"),
        "{file}"
    );
    assert!(file.ends_with("back\n"), "{file}");
}

#[test]
fn ac_ag_03_identical_events_reach_the_event_log_once_a_minute_and_the_file_each_time() {
    let sinks = sinks("repeats", 1 << 20);
    for stamp in 1..=5 {
        let line = format!("t{stamp}  WARN spooling: cannot write the log spool\n");
        sinks.log.write(Level::WARN, line.as_bytes());
    }
    sinks
        .log
        .write(Level::WARN, b"t6  WARN other: another warning\n");
    sinks.advance(REPEAT_WINDOW);
    sinks.log.write(
        Level::WARN,
        b"t7  WARN spooling: cannot write the log spool\n",
    );

    let events: Vec<String> = sinks.events().into_iter().map(|(_, text)| text).collect();
    assert_eq!(
        events,
        [
            "t1  WARN spooling: cannot write the log spool",
            "t6  WARN other: another warning",
            "t7  WARN spooling: cannot write the log spool (4 identical events after the previous one are only in the log file)",
        ]
    );
    let file = read(&sinks.dir, "agent-000001.log");
    assert_eq!(
        file.matches("cannot write the log spool").count(),
        6,
        "{file}"
    );
}

#[test]
fn the_writer_passes_each_event_whole_with_its_level() {
    let sinks = sinks("writer", 1024);
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(ServiceWriter(Arc::clone(&sinks.log)))
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(job = "j1", "the job started");
        tracing::warn!("the restore root is ignored");
    });
    let file = read(&sinks.dir, "agent-000001.log");
    assert_eq!(file.lines().count(), 2, "{file}");
    assert!(
        file.contains("INFO") && file.contains("job=\"j1\""),
        "{file}"
    );
    let events = sinks.events();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].0, Level::WARN);
    assert!(
        events[0].1.ends_with("the restore root is ignored"),
        "{events:?}"
    );
    assert!(!events[0].1.contains('\u{1b}'), "{events:?}");
}

#[test]
fn a_line_break_in_a_value_cannot_forge_a_line_in_the_file_or_the_event() {
    let sinks = sinks("forge", 4096);
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(ServiceWriter(Arc::clone(&sinks.log)))
        .finish();
    let forged = "v1\r\n2026-01-01T00:00:00Z ERROR restichub_agent: run icacls";
    tracing::subscriber::with_default(subscriber, || {
        tracing::warn!(version = %forged, "the control plane is newer");
    });
    let file = read(&sinks.dir, "agent-000001.log");
    assert_eq!(file.lines().count(), 1, "{file}");
    assert!(
        file.contains("v1\\r\\n2026-01-01T00:00:00Z ERROR"),
        "{file}"
    );
    let events = sinks.events();
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(!events[0].1.contains(['\r', '\n']), "{events:?}");
}

#[test]
fn a_control_character_is_escaped_and_a_tab_stays() {
    assert_eq!(escape("plain\ttext"), "plain\ttext");
    assert_eq!(
        escape("a\u{1b}[31mb\u{2028}c\u{85}d\n"),
        "a\\u{1b}[31mb\\u{2028}c\\u{85}d\\n"
    );
}

#[test]
fn an_event_text_stays_in_the_limit() {
    let long = "\u{44f}".repeat(40_000);
    assert_eq!(truncate(&long).chars().count(), 31_000);
    assert_eq!(truncate("short"), "short");
}

#[test]
fn an_event_text_outside_the_basic_plane_stays_in_the_utf16_limit() {
    let long = "\u{1f600}".repeat(40_000);
    let cut = truncate(&long);
    assert_eq!(cut.encode_utf16().count(), 31_000);
    assert!(cut.chars().all(|c| c == '\u{1f600}'));
    let odd = format!("a{long}");
    assert_eq!(truncate(&odd).encode_utf16().count(), 30_999);
}

#[test]
fn ac_ag_03_the_repeat_table_forgets_old_events() {
    let sinks = sinks("repeats-expire", 1 << 20);
    for job in 1..=50 {
        let line = format!("t1  WARN jobs: job {job} cannot start\n");
        sinks.log.write(Level::WARN, line.as_bytes());
        sinks.log.write(Level::WARN, line.as_bytes());
    }
    let repeats = || sinks.log.repeats.lock().expect("cannot lock").len();
    assert_eq!(repeats(), 50);
    sinks.advance(REPEAT_WINDOW * 2);
    sinks
        .log
        .write(Level::WARN, b"t2  WARN jobs: another warning\n");
    assert_eq!(repeats(), 1);
}
