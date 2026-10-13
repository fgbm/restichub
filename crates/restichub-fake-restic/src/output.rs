//! Recorded restic JSON messages (ADR-0011).
//!
//! The field names and the omitted defaults follow restic 0.19.1:
//! `internal/ui/backup/json.go`, `cmd/restic/cmd_snapshots.go`,
//! `cmd_forget.go`, `cmd_check.go`, `cmd_restore.go`, `cmd_cat.go`,
//! `cmd_version.go` and `internal/restic/config.go`.

use std::io::{self, Write};
use std::time::Duration;

use serde::Serialize;

use crate::cli::{Command, Invocation};

/// Target release of the emulated restic.
pub const RESTIC_VERSION: &str = "0.19.1";
/// Go toolchain of the restic release above.
pub const GO_VERSION: &str = "go1.26.4";

const NEWEST_START: &str = "2026-01-01T00:00:00Z";
const OLDEST_TIME: &str = "2025-12-31T00:00:00Z";
const NEWEST_END: &str = "2026-01-01T00:00:05Z";
const SNAPSHOT_ID: &str = "b2f8a1c94e7d60351a8f2b4c6d9e0f3a7c5b1d2e4f6089a3b7c1d5e9f2a406b8";
const OLD_SNAPSHOT_ID: &str = "0f3a7c5b1d2e4f6089a3b7c1d5e9f2a406b8b2f8a1c94e7d60351a8f2b4c6d9e";
const TREE_ID: &str = "5c1d2e4f6089a3b7c1d5e9f2a406b8b2f8a1c94e7d60351a8f2b4c6d9e0f3a7c";
const OLD_TREE_ID: &str = "9a3b7c1d5e9f2a406b8b2f8a1c94e7d60351a8f2b4c6d9e0f3a7c5b1d2e4f608";
const REPOSITORY_ID: &str = "c5b1d2e4f6089a3b7c1d5e9f2a406b8b2f8a1c94e7d60351a8f2b4c6d9e0f3a7";
const CHUNKER_POLYNOMIAL: &str = "25b468838dcb75";
const TOTAL_BYTES: u64 = 4000;
const TOTAL_FILES: u64 = 4;
const DEFAULT_PROGRESS_COUNT: u64 = 3;

/// Writes the output of the command to stdout.
pub fn emit(invocation: &Invocation) -> io::Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    match invocation.command {
        Command::Backup => backup(&mut out, invocation),
        Command::Forget => forget(&mut out, invocation),
        Command::Check => check(&mut out, invocation),
        Command::Snapshots => snapshots(&mut out, invocation),
        Command::Restore => restore(&mut out, invocation),
        Command::Init => init(&mut out, invocation),
        Command::Version => version(&mut out, invocation),
        Command::CatConfig => cat_config(&mut out),
        Command::Prune | Command::Unlock => Ok(()),
    }
}

fn backup<W: Write>(out: &mut W, invocation: &Invocation) -> io::Result<()> {
    if !invocation.global.json {
        return Ok(());
    }
    let count = progress_count();
    let rate = progress_rate();
    for index in 0..count {
        write_line(out, &backup_status(index, count, rate))?;
        sleep_before_next(index, count, rate);
    }
    write_line(out, &backup_summary())
}

fn restore<W: Write>(out: &mut W, invocation: &Invocation) -> io::Result<()> {
    if !invocation.global.json {
        return Ok(());
    }
    let count = progress_count();
    let rate = progress_rate();
    for index in 0..count {
        write_line(out, &restore_status(index, count, rate))?;
        sleep_before_next(index, count, rate);
    }
    write_line(out, &restore_summary())
}

fn snapshots<W: Write>(out: &mut W, invocation: &Invocation) -> io::Result<()> {
    if !invocation.global.json {
        return Ok(());
    }
    write_line(out, &vec![newest_snapshot()])
}

fn forget<W: Write>(out: &mut W, invocation: &Invocation) -> io::Result<()> {
    if !invocation.global.json {
        return Ok(());
    }
    let group = ForgetGroup {
        tags: Some(vec!["daily".to_string()]),
        host: "test-host".to_string(),
        paths: Some(vec!["/data".to_string()]),
        keep: Some(vec![newest_snapshot()]),
        remove: Some(vec![oldest_snapshot()]),
        reasons: Some(vec![KeepReason {
            snapshot: newest_snapshot(),
            matches: vec!["last snapshot".to_string()],
        }]),
    };
    write_line(out, &vec![group])
}

fn check<W: Write>(out: &mut W, invocation: &Invocation) -> io::Result<()> {
    if !invocation.global.json {
        return Ok(());
    }
    write_line(
        out,
        &CheckSummary {
            message_type: "summary",
            num_errors: 0,
            broken_packs: None,
            suggest_repair_index: false,
            suggest_prune: false,
        },
    )
}

fn init<W: Write>(out: &mut W, invocation: &Invocation) -> io::Result<()> {
    if !invocation.global.json {
        return Ok(());
    }
    write_line(
        out,
        &InitMessage {
            message_type: "initialized",
            id: REPOSITORY_ID,
            repository: invocation.repository.clone().unwrap_or_default(),
        },
    )
}

fn version<W: Write>(out: &mut W, invocation: &Invocation) -> io::Result<()> {
    let version = restic_version();
    if invocation.global.json {
        write_line(
            out,
            &VersionMessage {
                message_type: "version",
                version,
                go_version: GO_VERSION,
                go_os: go_os(),
                go_arch: go_arch(),
            },
        )
    } else {
        writeln!(
            out,
            "restic {version} compiled with {GO_VERSION} on {}/{}",
            go_os(),
            go_arch()
        )
    }
}

fn cat_config<W: Write>(out: &mut W) -> io::Result<()> {
    let config = RepositoryConfig {
        version: 2,
        id: REPOSITORY_ID,
        chunker_polynomial: CHUNKER_POLYNOMIAL,
    };
    let encoded = serde_json::to_string_pretty(&config).map_err(io::Error::other)?;
    writeln!(out, "{encoded}")
}

/// Release string that the `version` command prints. A test can present an
/// older release to the agent (AC-AG-09).
fn restic_version() -> String {
    std::env::var("FAKE_RESTIC_VERSION").unwrap_or_else(|_| RESTIC_VERSION.to_string())
}

/// Number of `status` messages that a backup or a restore writes.
fn progress_count() -> u64 {
    env_u64("FAKE_RESTIC_PROGRESS_COUNT", DEFAULT_PROGRESS_COUNT).max(1)
}

/// `status` messages for each second. Zero means no wait between the messages.
fn progress_rate() -> f64 {
    env_f64("FAKE_RESTIC_PROGRESS_RATE", 0.0).max(0.0)
}

fn backup_status(index: u64, count: u64, rate: f64) -> BackupStatus {
    let percent = percent_done(index, count);
    BackupStatus {
        message_type: "status",
        seconds_elapsed: nonzero(elapsed_seconds(index, rate)),
        seconds_remaining: nonzero(remaining_seconds(index, count, rate)),
        percent_done: percent,
        total_files: Some(TOTAL_FILES),
        files_done: Some(index + 1),
        total_bytes: Some(TOTAL_BYTES),
        bytes_done: Some(bytes_done(percent)),
        error_count: None,
        current_files: Some(vec![format!("/data/file{}.txt", index + 1)]),
    }
}

fn restore_status(index: u64, count: u64, rate: f64) -> RestoreStatus {
    let percent = percent_done(index, count);
    RestoreStatus {
        message_type: "status",
        seconds_elapsed: nonzero(elapsed_seconds(index, rate)),
        percent_done: percent,
        total_files: Some(TOTAL_FILES),
        files_restored: Some(index + 1),
        files_skipped: None,
        files_deleted: None,
        total_bytes: Some(TOTAL_BYTES),
        bytes_restored: Some(bytes_done(percent)),
        bytes_skipped: None,
    }
}

fn backup_summary() -> BackupSummary {
    BackupSummary {
        message_type: "summary",
        files_new: 1,
        files_changed: 0,
        files_unmodified: 3,
        dirs_new: 1,
        dirs_changed: 0,
        dirs_unmodified: 1,
        data_blobs: 1,
        tree_blobs: 1,
        data_added: 1024,
        data_added_packed: 1100,
        total_files_processed: TOTAL_FILES,
        total_bytes_processed: TOTAL_BYTES,
        total_duration: 5.0,
        backup_start: NEWEST_START,
        backup_end: NEWEST_END,
        snapshot_id: Some(SNAPSHOT_ID),
        dry_run: None,
    }
}

fn restore_summary() -> RestoreSummary {
    RestoreSummary {
        message_type: "summary",
        seconds_elapsed: None,
        total_files: Some(TOTAL_FILES),
        files_restored: Some(TOTAL_FILES),
        files_skipped: None,
        files_deleted: None,
        total_bytes: Some(TOTAL_BYTES),
        bytes_restored: Some(TOTAL_BYTES),
        bytes_skipped: None,
    }
}

fn newest_snapshot() -> Snapshot {
    Snapshot {
        time: NEWEST_START,
        parent: None,
        tree: TREE_ID,
        paths: vec!["/data".to_string()],
        hostname: Some("test-host".to_string()),
        username: Some("backup".to_string()),
        uid: Some(1000),
        gid: Some(1000),
        excludes: None,
        tags: Some(vec!["daily".to_string()]),
        original: None,
        program_version: Some(format!("restic {}", restic_version())),
        summary: Some(newest_snapshot_summary()),
        id: SNAPSHOT_ID,
        short_id: &SNAPSHOT_ID[..10],
    }
}

fn oldest_snapshot() -> Snapshot {
    Snapshot {
        time: OLDEST_TIME,
        parent: None,
        tree: OLD_TREE_ID,
        paths: vec!["/data".to_string()],
        hostname: None,
        username: None,
        uid: None,
        gid: None,
        excludes: None,
        tags: Some(vec!["weekly".to_string()]),
        original: None,
        program_version: Some(format!("restic {}", restic_version())),
        summary: None,
        id: OLD_SNAPSHOT_ID,
        short_id: &OLD_SNAPSHOT_ID[..10],
    }
}

fn newest_snapshot_summary() -> SnapshotSummary {
    SnapshotSummary {
        backup_start: NEWEST_START,
        backup_end: NEWEST_END,
        files_new: 1,
        files_changed: 0,
        files_unmodified: 3,
        dirs_new: 1,
        dirs_changed: 0,
        dirs_unmodified: 1,
        data_blobs: 1,
        tree_blobs: 1,
        data_added: 1024,
        data_added_packed: 1100,
        total_files_processed: TOTAL_FILES,
        total_bytes_processed: TOTAL_BYTES,
    }
}

fn percent_done(index: u64, count: u64) -> f64 {
    (index + 1) as f64 / (count + 1) as f64
}

fn bytes_done(percent: f64) -> u64 {
    (percent * TOTAL_BYTES as f64).round() as u64
}

fn elapsed_seconds(index: u64, rate: f64) -> u64 {
    if rate > 0.0 {
        (index as f64 / rate).floor() as u64
    } else {
        0
    }
}

fn remaining_seconds(index: u64, count: u64, rate: f64) -> u64 {
    if rate > 0.0 {
        ((count - 1 - index) as f64 / rate).floor() as u64
    } else {
        0
    }
}

fn sleep_before_next(index: u64, count: u64, rate: f64) {
    if rate > 0.0 && index + 1 < count {
        std::thread::sleep(Duration::from_secs_f64(1.0 / rate));
    }
}

fn nonzero(value: u64) -> Option<u64> {
    (value > 0).then_some(value)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn go_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    }
}

fn go_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    }
}

fn write_line<W: Write, T: Serialize>(out: &mut W, value: &T) -> io::Result<()> {
    serde_json::to_writer(&mut *out, value).map_err(io::Error::other)?;
    out.write_all(b"\n")
}

/// The restic exit codes of FR-AG-09. Any other code is a generic failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResticExit {
    Success,
    Failure,
    Warning,
    NoRepository,
    Locked,
    WrongPassword,
    Cancelled,
    Other(u8),
}

impl ResticExit {
    pub fn from_byte(code: u8) -> Self {
        match code {
            0 => Self::Success,
            1 => Self::Failure,
            3 => Self::Warning,
            10 => Self::NoRepository,
            11 => Self::Locked,
            12 => Self::WrongPassword,
            130 => Self::Cancelled,
            other => Self::Other(other),
        }
    }

    pub fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }

    pub fn code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failure => 1,
            Self::Warning => 3,
            Self::NoRepository => 10,
            Self::Locked => 11,
            Self::WrongPassword => 12,
            Self::Cancelled => 130,
            Self::Other(code) => code,
        }
    }

    /// The `exit_error` JSON line that restic writes to stderr before it exits.
    pub fn error_message(self) -> String {
        let error = ExitError {
            message_type: "exit_error",
            code: self.code(),
            message: self.reason(),
        };
        serde_json::to_string(&error).unwrap_or_else(|_| String::from("{}"))
    }

    fn reason(self) -> &'static str {
        match self {
            Self::Success => "the command was successful",
            Self::Failure => "the command failed",
            Self::Warning => {
                "some source data could not be read, or `forget` could not remove one or more snapshots"
            }
            Self::NoRepository => "the repository does not exist",
            Self::Locked => {
                "the repository is already locked, the `unlock` command can be used to remove stale locks"
            }
            Self::WrongPassword => "the password is incorrect or no key was found",
            Self::Cancelled => "the command was cancelled",
            Self::Other(_) => "the command failed",
        }
    }
}

#[derive(Serialize)]
struct ExitError {
    message_type: &'static str,
    code: u8,
    message: &'static str,
}

#[derive(Serialize)]
struct BackupStatus {
    message_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    seconds_elapsed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seconds_remaining: Option<u64>,
    percent_done: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_files: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files_done: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes_done: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_files: Option<Vec<String>>,
}

#[derive(Serialize)]
struct BackupSummary {
    message_type: &'static str,
    files_new: u64,
    files_changed: u64,
    files_unmodified: u64,
    dirs_new: u64,
    dirs_changed: u64,
    dirs_unmodified: u64,
    data_blobs: i64,
    tree_blobs: i64,
    data_added: u64,
    data_added_packed: u64,
    total_files_processed: u64,
    total_bytes_processed: u64,
    total_duration: f64,
    backup_start: &'static str,
    backup_end: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    snapshot_id: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
}

#[derive(Serialize, Clone)]
struct SnapshotSummary {
    backup_start: &'static str,
    backup_end: &'static str,
    files_new: u64,
    files_changed: u64,
    files_unmodified: u64,
    dirs_new: u64,
    dirs_changed: u64,
    dirs_unmodified: u64,
    data_blobs: i64,
    tree_blobs: i64,
    data_added: u64,
    data_added_packed: u64,
    total_files_processed: u64,
    total_bytes_processed: u64,
}

#[derive(Serialize, Clone)]
struct Snapshot {
    time: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<&'static str>,
    tree: &'static str,
    paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    excludes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    program_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<SnapshotSummary>,
    id: &'static str,
    short_id: &'static str,
}

#[derive(Serialize)]
struct KeepReason {
    snapshot: Snapshot,
    matches: Vec<String>,
}

#[derive(Serialize)]
struct ForgetGroup {
    // The restic field has no omitempty, so a missing list is null.
    tags: Option<Vec<String>>,
    host: String,
    paths: Option<Vec<String>>,
    keep: Option<Vec<Snapshot>>,
    remove: Option<Vec<Snapshot>>,
    reasons: Option<Vec<KeepReason>>,
}

#[derive(Serialize)]
struct CheckSummary {
    message_type: &'static str,
    num_errors: i64,
    broken_packs: Option<Vec<String>>,
    suggest_repair_index: bool,
    suggest_prune: bool,
}

#[derive(Serialize)]
struct InitMessage {
    message_type: &'static str,
    id: &'static str,
    repository: String,
}

#[derive(Serialize)]
struct VersionMessage {
    message_type: &'static str,
    version: String,
    go_version: &'static str,
    go_os: &'static str,
    go_arch: &'static str,
}

#[derive(Serialize)]
struct RepositoryConfig {
    version: u64,
    id: &'static str,
    chunker_polynomial: &'static str,
}

#[derive(Serialize)]
struct RestoreStatus {
    message_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    seconds_elapsed: Option<u64>,
    percent_done: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_files: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files_restored: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files_skipped: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files_deleted: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes_restored: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes_skipped: Option<u64>,
}

#[derive(Serialize)]
struct RestoreSummary {
    message_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    seconds_elapsed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_files: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files_restored: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files_skipped: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files_deleted: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes_restored: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes_skipped: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{
        CHUNKER_POLYNOMIAL, OLD_SNAPSHOT_ID, OLD_TREE_ID, REPOSITORY_ID, RESTIC_VERSION,
        ResticExit, SNAPSHOT_ID, TREE_ID,
    };

    fn assert_hex_id(value: &str) {
        assert_eq!(value.len(), 64, "wrong id length: {value}");
        assert!(
            value
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "not lowercase hex: {value}"
        );
    }

    #[test]
    fn recorded_ids_are_full_lowercase_hex() {
        for id in [
            SNAPSHOT_ID,
            OLD_SNAPSHOT_ID,
            TREE_ID,
            OLD_TREE_ID,
            REPOSITORY_ID,
        ] {
            assert_hex_id(id);
        }
        assert!(
            CHUNKER_POLYNOMIAL.chars().all(|c| c.is_ascii_hexdigit()),
            "not hex: {CHUNKER_POLYNOMIAL}"
        );
    }

    #[test]
    fn ac_ag_05_exit_code_mapping() {
        let cases = [
            (0, ResticExit::Success),
            (1, ResticExit::Failure),
            (3, ResticExit::Warning),
            (10, ResticExit::NoRepository),
            (11, ResticExit::Locked),
            (12, ResticExit::WrongPassword),
            (130, ResticExit::Cancelled),
            (99, ResticExit::Other(99)),
        ];
        for (code, expected) in cases {
            assert_eq!(ResticExit::from_byte(code), expected, "exit code {code}");
            assert_eq!(expected.code(), code, "round trip {code}");
        }
    }

    #[test]
    fn exit_error_message_names_the_code() {
        let message = ResticExit::Locked.error_message();
        assert!(message.contains("\"message_type\":\"exit_error\""));
        assert!(message.contains("\"code\":11"));
    }

    #[test]
    fn version_is_the_pinned_restic_release() {
        assert_eq!(RESTIC_VERSION, "0.19.1");
    }
}
