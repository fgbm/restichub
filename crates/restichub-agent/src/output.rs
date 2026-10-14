//! The restic output of one step: bounded line readers and the JSON parser (FR-AG-08, ADR-0011).
//!
//! restic prints one JSON message on each line for `backup` and `check`, and one JSON document for
//! `snapshots`, `forget` and `stats`. The agent reads each pipe in a thread with a hard limit on the
//! line length, so a hostile or broken restic cannot grow the memory of the agent. The parser
//! ignores unknown fields and message types (ADR-0011). A line that is not a known message is a log
//! line.

use std::io::Read;

use prost_types::Timestamp;
use restichub_proto::agent::v1::{
    BackupSummary, CheckSummary, ForgetGroup, ForgetResult, ForgetSnapshot, Snapshot, SnapshotList,
    StatsSummary, StepProgress, step, step_result,
};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::mpsc;

/// The longest line that the agent keeps; the rest of a longer line is dropped (FR-AG-08).
pub(crate) const MAX_LINE: usize = 64 * 1024;

/// The largest JSON document that the agent parses (`snapshots`, `forget`, `stats`).
pub(crate) const MAX_DOCUMENT: usize = 8 * 1024 * 1024;

/// The most snapshots that one result carries, so a result fits in a protocol message. The agent
/// keeps the newest ones.
pub(crate) const MAX_SNAPSHOTS: usize = 2_000;

/// The most current files that one progress message carries.
const MAX_CURRENT_FILES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pipe {
    Stdout,
    Stderr,
}

/// One line of restic output, at most the reader limit long.
pub(crate) struct Line {
    pub(crate) pipe: Pipe,
    pub(crate) bytes: Vec<u8>,
    /// True when the reader dropped the end of the line.
    pub(crate) truncated: bool,
}

/// Starts a thread that reads a pipe line by line and sends the lines into the channel.
///
/// The thread ends at the end of the pipe or when the receiver is gone.
pub(crate) fn spawn_reader<R: Read + Send + 'static>(
    reader: R,
    pipe: Pipe,
    max_line: usize,
    tx: mpsc::Sender<Line>,
) {
    let spawned = std::thread::Builder::new()
        .name("restic-output".to_owned())
        .spawn(move || {
            read_lines(reader, max_line, |bytes, truncated| {
                tx.blocking_send(Line {
                    pipe,
                    bytes,
                    truncated,
                })
                .is_ok()
            });
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "cannot start the output reader");
    }
}

/// Splits a stream into lines of at most `max` bytes. `emit` returns false to stop.
pub(crate) fn read_lines<R: Read>(
    mut reader: R,
    max: usize,
    mut emit: impl FnMut(Vec<u8>, bool) -> bool,
) {
    let mut current: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        for &byte in &buffer[..read] {
            if byte == b'\n' {
                if current.last() == Some(&b'\r') {
                    current.pop();
                }
                if !emit(std::mem::take(&mut current), truncated) {
                    return;
                }
                truncated = false;
            } else if current.len() < max {
                current.push(byte);
            } else {
                truncated = true;
            }
        }
    }
    if !current.is_empty() || truncated {
        emit(current, truncated);
    }
}

/// How the agent reads the standard output of a step kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputKind {
    Backup,
    Check,
    Snapshots,
    Forget,
    Stats,
    /// A command with text output only, such as `prune` or `unlock`.
    Plain,
}

impl OutputKind {
    pub(crate) fn of(kind: &step::Kind) -> Self {
        match kind {
            step::Kind::Backup(_) => Self::Backup,
            step::Kind::Check(_) => Self::Check,
            step::Kind::Snapshots(_) => Self::Snapshots,
            step::Kind::Forget(_) => Self::Forget,
            step::Kind::Stats(_) => Self::Stats,
            _ => Self::Plain,
        }
    }

    /// The line limit of the standard output reader.
    pub(crate) fn stdout_limit(self) -> usize {
        match self {
            Self::Snapshots | Self::Forget | Self::Stats => MAX_DOCUMENT,
            _ => MAX_LINE,
        }
    }
}

/// What the agent does with one line of standard output.
#[derive(Debug, PartialEq)]
pub(crate) enum Accepted {
    Log(String),
    Progress(StepProgress),
    /// The collector keeps the line for the typed output.
    Consumed,
}

/// Turns the standard output of one step into the typed output (FR-AG-08).
pub(crate) struct Collector {
    kind: OutputKind,
    step_id: String,
    job_id: String,
    output: Option<step_result::Output>,
    document: Vec<u8>,
    document_invalid: bool,
}

impl Collector {
    pub(crate) fn new(kind: OutputKind, job_id: &str, step_id: &str) -> Self {
        Self {
            kind,
            step_id: step_id.to_owned(),
            job_id: job_id.to_owned(),
            output: None,
            document: Vec::new(),
            document_invalid: false,
        }
    }

    /// Takes one line of standard output.
    pub(crate) fn accept(&mut self, line: &str, truncated: bool) -> Accepted {
        match self.kind {
            OutputKind::Backup | OutputKind::Check => self.accept_message(line, truncated),
            OutputKind::Snapshots | OutputKind::Forget | OutputKind::Stats => {
                if truncated || self.document.len() + line.len() + 1 > MAX_DOCUMENT {
                    self.document_invalid = true;
                } else {
                    self.document.extend_from_slice(line.as_bytes());
                    self.document.push(b'\n');
                }
                Accepted::Consumed
            }
            OutputKind::Plain => Accepted::Log(line.to_owned()),
        }
    }

    fn accept_message(&mut self, line: &str, truncated: bool) -> Accepted {
        let message = if truncated {
            None
        } else {
            serde_json::from_str::<Value>(line).ok()
        };
        let Some(message) = message else {
            return Accepted::Log(line.to_owned());
        };
        let message_type = message.get("message_type").and_then(Value::as_str);
        match (self.kind, message_type) {
            (OutputKind::Backup, Some("status")) => {
                Accepted::Progress(progress(&message, &self.job_id, &self.step_id))
            }
            (OutputKind::Backup, Some("summary")) => {
                self.output = Some(step_result::Output::BackupSummary(backup_summary(&message)));
                Accepted::Consumed
            }
            (OutputKind::Check, Some("summary")) => {
                self.output = Some(step_result::Output::CheckSummary(CheckSummary {
                    num_errors: uint(&message, "num_errors"),
                    repair_suggested: message
                        .get("suggest_repair_index")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        || message
                            .get("suggest_repair_prune")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    blobs_checked: 0,
                    packs_checked: 0,
                }));
                Accepted::Consumed
            }
            (OutputKind::Backup, Some("error")) => Accepted::Log(backup_error(&message)),
            // A status or an unknown message type carries no log text (ADR-0011).
            (_, Some("verbose_status" | "status")) => Accepted::Consumed,
            _ => Accepted::Log(line.to_owned()),
        }
    }

    /// Ends the step and returns the typed output, or `None` when restic printed none or printed
    /// output that the agent cannot parse.
    pub(crate) fn finish(mut self) -> Option<step_result::Output> {
        if self.output.is_some() || self.document_invalid || self.document.is_empty() {
            return self.output;
        }
        let document: Value = serde_json::from_slice(&self.document).ok()?;
        match self.kind {
            OutputKind::Snapshots => {
                Some(step_result::Output::SnapshotList(snapshot_list(&document)))
            }
            OutputKind::Forget => Some(step_result::Output::ForgetResult(forget_result(&document))),
            OutputKind::Stats => Some(step_result::Output::StatsSummary(stats_summary(&document))),
            _ => self.output.take(),
        }
    }
}

fn uint(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn float(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn progress(message: &Value, job_id: &str, step_id: &str) -> StepProgress {
    let current_files = message
        .get("current_files")
        .and_then(Value::as_array)
        .map(|files| {
            files
                .iter()
                .filter_map(Value::as_str)
                .take(MAX_CURRENT_FILES)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    StepProgress {
        job_id: job_id.to_owned(),
        step_id: step_id.to_owned(),
        bytes_done: uint(message, "bytes_done"),
        total_bytes: uint(message, "total_bytes"),
        files_done: uint(message, "files_done"),
        total_files: uint(message, "total_files"),
        percent_done: float(message, "percent_done"),
        seconds_elapsed: float(message, "seconds_elapsed"),
        seconds_remaining: uint(message, "seconds_remaining"),
        current_files,
    }
}

fn backup_summary(message: &Value) -> BackupSummary {
    BackupSummary {
        files_new: uint(message, "files_new"),
        files_changed: uint(message, "files_changed"),
        files_unmodified: uint(message, "files_unmodified"),
        dirs_new: uint(message, "dirs_new"),
        dirs_changed: uint(message, "dirs_changed"),
        dirs_unmodified: uint(message, "dirs_unmodified"),
        data_blobs: uint(message, "data_blobs"),
        tree_blobs: uint(message, "tree_blobs"),
        data_added: uint(message, "data_added"),
        data_added_packed: uint(message, "data_added_packed"),
        total_files_processed: uint(message, "total_files_processed"),
        total_bytes_processed: uint(message, "total_bytes_processed"),
        total_duration_seconds: float(message, "total_duration"),
        snapshot_id: text(message, "snapshot_id"),
        dry_run: message
            .get("dry_run")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

/// The log text of a restic `error` message of a backup.
fn backup_error(message: &Value) -> String {
    let detail = message
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("error");
    match (
        message.get("during").and_then(Value::as_str),
        message.get("item").and_then(Value::as_str),
    ) {
        (Some(during), Some(item)) => format!("error during {during} of {item}: {detail}"),
        (Some(during), None) => format!("error during {during}: {detail}"),
        _ => format!("error: {detail}"),
    }
}

fn timestamp(value: &Value, key: &str) -> Option<Timestamp> {
    let parsed = OffsetDateTime::parse(value.get(key)?.as_str()?, &Rfc3339).ok()?;
    Some(Timestamp {
        seconds: parsed.unix_timestamp(),
        nanos: i32::try_from(parsed.nanosecond()).unwrap_or(0),
    })
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn snapshot_list(document: &Value) -> SnapshotList {
    let items = document.as_array().map(Vec::as_slice).unwrap_or_default();
    let skip = items.len().saturating_sub(MAX_SNAPSHOTS);
    let snapshots = items.iter().skip(skip).map(snapshot).collect();
    SnapshotList { snapshots }
}

fn snapshot(item: &Value) -> Snapshot {
    let id = text(item, "id");
    let summary = item.get("summary").map(|summary| {
        let mut summary = backup_summary(summary);
        summary.snapshot_id.clone_from(&id);
        summary
    });
    Snapshot {
        short_id: text(item, "short_id"),
        time: timestamp(item, "time"),
        tree: text(item, "tree"),
        paths: strings(item, "paths"),
        hostname: text(item, "hostname"),
        username: text(item, "username"),
        tags: strings(item, "tags"),
        program_version: text(item, "program_version"),
        summary,
        id,
    }
}

fn forget_result(document: &Value) -> ForgetResult {
    let groups = document
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|group| {
            let reasons = group
                .get("reasons")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            ForgetGroup {
                tags: strings(group, "tags"),
                host: text(group, "host"),
                paths: strings(group, "paths"),
                kept: forget_snapshots(group, "keep", reasons),
                removed: forget_snapshots(group, "remove", reasons),
            }
        })
        .collect();
    ForgetResult { groups }
}

fn forget_snapshots(group: &Value, key: &str, reasons: &[Value]) -> Vec<ForgetSnapshot> {
    group
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .take(MAX_SNAPSHOTS)
        .map(|item| {
            let id = text(item, "id");
            let reason = reasons
                .iter()
                .find(|reason| reason.get("snapshot").is_some_and(|s| text(s, "id") == id))
                .map(|reason| strings(reason, "matches").join(","))
                .unwrap_or_default();
            ForgetSnapshot {
                time: timestamp(item, "time"),
                reason,
                id,
            }
        })
        .collect()
}

fn stats_summary(document: &Value) -> StatsSummary {
    StatsSummary {
        total_size: uint(document, "total_size"),
        total_uncompressed_size: uint(document, "total_uncompressed_size"),
        total_file_count: uint(document, "total_file_count"),
        snapshots_count: uint(document, "snapshots_count"),
        compression_ratio: float(document, "compression_ratio"),
    }
}

#[cfg(test)]
mod tests;
