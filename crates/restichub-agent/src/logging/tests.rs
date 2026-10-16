#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing::Level;

use super::RotatingFile;
use super::sinks::{EventSink, ServiceLog, ServiceWriter, event_text};

fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("restichub-agent-log-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    dir
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("cannot list the directory")
        .map(|entry| {
            entry
                .expect("cannot read an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).expect("cannot read the log file")
}

#[test]
fn a_full_file_starts_the_next_one() {
    let dir = temp_dir("size");
    let mut file = RotatingFile::open(&dir, 10, 5).expect("cannot open the log");
    file.write(b"1234\n").expect("cannot write");
    file.write(b"5678\n").expect("cannot write");
    file.write(b"abcd\n").expect("cannot write");
    assert_eq!(names(&dir), ["agent-000001.log", "agent-000002.log"]);
    assert_eq!(read(&dir, "agent-000001.log"), "1234\n5678\n");
    assert_eq!(read(&dir, "agent-000002.log"), "abcd\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_event_larger_than_a_file_stays_whole() {
    let dir = temp_dir("large");
    let mut file = RotatingFile::open(&dir, 4, 5).expect("cannot open the log");
    file.write(b"0123456789\n").expect("cannot write");
    assert_eq!(names(&dir), ["agent-000001.log"]);
    assert_eq!(read(&dir, "agent-000001.log"), "0123456789\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_oldest_files_above_the_count_are_removed() {
    let dir = temp_dir("count");
    std::fs::write(dir.join("notes.txt"), b"other").expect("cannot write");
    let mut file = RotatingFile::open(&dir, 5, 3).expect("cannot open the log");
    for line in ["one\n", "two\n", "three\n", "four\n", "five\n"] {
        file.write(line.as_bytes()).expect("cannot write");
    }
    assert_eq!(
        names(&dir),
        [
            "agent-000003.log",
            "agent-000004.log",
            "agent-000005.log",
            "notes.txt"
        ]
    );
    assert_eq!(read(&dir, "agent-000005.log"), "five\n");
    assert_eq!(file.current(), dir.join("agent-000005.log"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_restart_continues_the_newest_file_until_it_is_full() {
    let dir = temp_dir("restart");
    std::fs::write(dir.join("agent-000009.log"), b"old\n").expect("cannot write");
    std::fs::write(dir.join("agent-000010.log"), b"new\n").expect("cannot write");
    let mut file = RotatingFile::open(&dir, 9, 5).expect("cannot open the log");
    file.write(b"more\n").expect("cannot write");
    assert_eq!(read(&dir, "agent-000010.log"), "new\nmore\n");
    drop(file);
    let mut file = RotatingFile::open(&dir, 9, 5).expect("cannot open the log");
    file.write(b"next\n").expect("cannot write");
    assert_eq!(read(&dir, "agent-000011.log"), "next\n");
    assert_eq!(
        names(&dir),
        ["agent-000009.log", "agent-000010.log", "agent-000011.log"]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_name_that_is_not_a_log_file_is_left_alone() {
    let dir = temp_dir("names");
    for name in [
        "agent-.log",
        "agent-12a.log",
        "agent-000001.txt",
        "other-000001.log",
    ] {
        std::fs::write(dir.join(name), b"x").expect("cannot write");
    }
    let file = RotatingFile::open(&dir, 1, 1).expect("cannot open the log");
    assert_eq!(file.current(), dir.join("agent-000001.log"));
    assert_eq!(names(&dir).len(), 5);
    let _ = std::fs::remove_dir_all(&dir);
}

type Events = Arc<Mutex<Vec<(Level, String)>>>;

struct FakeEvents {
    seen: Events,
    fail: bool,
}

impl EventSink for FakeEvents {
    fn report(&self, level: Level, text: &str) -> std::io::Result<()> {
        if self.fail {
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
    log: ServiceLog,
    events: Events,
    stderr: Arc<Mutex<String>>,
}

fn sinks(name: &str, max_bytes: u64, events_fail: bool) -> Sinks {
    let dir = temp_dir(name);
    let events: Events = Arc::default();
    let stderr: Arc<Mutex<String>> = Arc::default();
    let captured = Arc::clone(&stderr);
    let log = ServiceLog::new(
        RotatingFile::open(&dir, max_bytes, 5).map_err(anyhow::Error::from),
        Ok(Box::new(FakeEvents {
            seen: Arc::clone(&events),
            fail: events_fail,
        })),
        Box::new(move |bytes| {
            captured
                .lock()
                .expect("cannot lock")
                .push_str(&String::from_utf8_lossy(bytes));
        }),
    );
    Sinks {
        dir,
        log,
        events,
        stderr,
    }
}

impl Sinks {
    fn events(&self) -> Vec<(Level, String)> {
        self.events.lock().expect("cannot lock").clone()
    }

    fn stderr(&self) -> String {
        self.stderr.lock().expect("cannot lock").clone()
    }
}

#[test]
fn ac_ag_03_info_goes_to_the_file_and_a_warning_or_an_error_also_to_the_event_log() {
    let sinks = sinks("select", 1024, false);
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
    assert_eq!(sinks.stderr(), "");
    let _ = std::fs::remove_dir_all(&sinks.dir);
}

#[test]
fn a_failed_file_falls_back_to_stderr_with_one_warning() {
    let sinks = sinks("file-fails", 8, false);
    sinks.log.write(Level::INFO, b"first\n");
    // A directory in the place of the next file makes the rotation fail.
    std::fs::create_dir(sinks.dir.join("agent-000002.log")).expect("cannot create the obstacle");
    sinks.log.write(Level::INFO, b"second\n");
    sinks.log.write(Level::WARN, b"third\n");
    let stderr = sinks.stderr();
    assert_eq!(
        stderr.matches("cannot write the log file").count(),
        1,
        "{stderr}"
    );
    assert!(stderr.ends_with("second\nthird\n"), "{stderr}");
    let events = sinks.events();
    assert_eq!(events.len(), 2, "{events:?}");
    assert!(
        events[0].1.contains("cannot write the log file"),
        "{events:?}"
    );
    assert_eq!(events[1], (Level::WARN, "third".to_owned()));
    let _ = std::fs::remove_dir_all(&sinks.dir);
}

#[test]
fn a_failed_event_log_falls_back_to_stderr_with_one_warning() {
    let sinks = sinks("events-fail", 1024, true);
    sinks.log.write(Level::WARN, b"one\n");
    sinks.log.write(Level::ERROR, b"two\n");
    sinks.log.write(Level::INFO, b"three\n");
    let stderr = sinks.stderr();
    assert_eq!(
        stderr.matches("cannot write to the event log").count(),
        1,
        "{stderr}"
    );
    assert!(stderr.ends_with("one\ntwo\n"), "{stderr}");
    let file = read(&sinks.dir, "agent-000001.log");
    assert_eq!(
        file.matches("cannot write to the event log").count(),
        1,
        "{file}"
    );
    assert!(file.contains("one\n") && file.contains("two\n") && file.ends_with("three\n"));
    let _ = std::fs::remove_dir_all(&sinks.dir);
}

#[test]
fn a_file_that_cannot_open_is_reported_once_and_the_log_goes_on() {
    let events: Events = Arc::default();
    let stderr: Arc<Mutex<String>> = Arc::default();
    let captured = Arc::clone(&stderr);
    let log = ServiceLog::new(
        Err(anyhow::anyhow!("the directory is not protected")),
        Ok(Box::new(FakeEvents {
            seen: Arc::clone(&events),
            fail: false,
        })),
        Box::new(move |bytes| {
            captured
                .lock()
                .expect("cannot lock")
                .push_str(&String::from_utf8_lossy(bytes));
        }),
    );
    log.write(Level::INFO, b"connected\n");
    let stderr = stderr.lock().expect("cannot lock").clone();
    assert!(
        stderr.contains("the directory is not protected"),
        "{stderr}"
    );
    assert!(stderr.ends_with("connected\n"), "{stderr}");
    let events = events.lock().expect("cannot lock").clone();
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(
        events[0].1.contains("cannot open the log file"),
        "{events:?}"
    );
}

#[test]
fn the_writer_passes_each_event_whole_with_its_level() {
    let sinks = sinks("writer", 1024, false);
    let log = Arc::new(sinks.log);
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(ServiceWriter(Arc::clone(&log)))
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
    let events = sinks.events.lock().expect("cannot lock").clone();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].0, Level::WARN);
    assert!(
        events[0].1.ends_with("the restore root is ignored"),
        "{events:?}"
    );
    assert!(!events[0].1.contains('\u{1b}'), "{events:?}");
    let _ = std::fs::remove_dir_all(&sinks.dir);
}

#[test]
fn an_event_text_loses_the_line_end_and_stays_in_the_limit() {
    assert_eq!(event_text(b"text\r\n"), "text");
    let long = "\u{44f}".repeat(40_000);
    assert_eq!(event_text(long.as_bytes()).chars().count(), 31_000);
}
