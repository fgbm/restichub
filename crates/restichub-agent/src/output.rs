//! The restic output of one step: bounded line readers and the JSON parser (FR-AG-08, ADR-0011).
//!
//! restic prints one JSON message on each line for `backup` and `check`, and one JSON document for
//! `snapshots`, `forget` and `stats`. The agent reads each pipe in a thread with a hard limit on the
//! line length, so a hostile or broken restic cannot grow the memory of the agent. The parser
//! ignores unknown fields and message types (ADR-0011). A line that is not a known message is a log
//! line.

use std::collections::HashMap;
use std::io::Read;

use prost::Message as _;
use prost_types::Timestamp;
use restichub_proto::agent::v1::{
    BackupSummary, CheckSummary, ForgetGroup, ForgetResult, ForgetSnapshot, LsPage, Node, NodeType,
    RestoreSummary, Snapshot, SnapshotList, StatsSummary, StepProgress, step, step_result,
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

/// The encoded size that a snapshot list or a forget result may reach. The rest of the step result
/// is small, so the whole message stays below [`restichub_proto::MAX_STEP_RESULT_BYTES`].
const MAX_OUTPUT_BYTES: usize = restichub_proto::MAX_STEP_RESULT_BYTES - 64 * 1024;

/// The nodes of one `ls` page when the step names no limit (AC-RS-02).
pub(crate) const DEFAULT_LS_PAGE: u64 = 1_000;

/// The depth of the line queue between the readers and the step loop for a text step.
const TEXT_QUEUE: usize = 64;

/// The depth for a step whose lines can be as long as a document. The queue holds at most this
/// many documents in memory.
pub(crate) const DOCUMENT_QUEUE: usize = 2;

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
    /// One JSON message for each node. The collector keeps one page of them (AC-RS-02).
    Ls,
    Restore,
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
            step::Kind::Ls(_) => Self::Ls,
            step::Kind::Restore(_) => Self::Restore,
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

    /// The depth of the line queue, which bounds the memory of the pending lines.
    pub(crate) fn queue_depth(self) -> usize {
        match self {
            Self::Snapshots | Self::Forget | Self::Stats => DOCUMENT_QUEUE,
            _ => TEXT_QUEUE,
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
    /// A status line without log text. restic repeats it while it waits on a backend.
    Status(String),
}

/// Turns the standard output of one step into the typed output (FR-AG-08).
pub(crate) struct Collector {
    kind: OutputKind,
    step_id: String,
    job_id: String,
    output: Option<step_result::Output>,
    document: Vec<u8>,
    document_invalid: bool,
    window: Option<LsWindow>,
    nodes: Vec<Node>,
    nodes_seen: u64,
    nodes_bytes: usize,
    more_nodes: bool,
    dry_run: bool,
}

/// Nodes of the `ls` stream that a step returns: from `skip`, at most `limit` of them (AC-RS-02).
struct LsWindow {
    /// `ls` prints the listed directory as its first node, which is not an entry of itself.
    directory: String,
    skip: u64,
    limit: u64,
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
            window: None,
            nodes: Vec::new(),
            nodes_seen: 0,
            nodes_bytes: 0,
            more_nodes: false,
            dry_run: false,
        }
    }

    /// Selects the page of an `ls` listing: the nodes from `offset`, at most `limit` of them. A
    /// zero limit gives [`DEFAULT_LS_PAGE`].
    #[must_use]
    pub(crate) fn with_window(mut self, directory: &str, offset: u64, limit: u64) -> Self {
        let limit = if limit == 0 { DEFAULT_LS_PAGE } else { limit };
        self.window = Some(LsWindow {
            directory: directory.trim_end_matches('/').to_owned(),
            skip: offset,
            limit,
        });
        self
    }

    /// Marks the restore as a dry run: restic prints no flag for it in the summary.
    #[must_use]
    pub(crate) fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// True when the page is full and more nodes follow. The step loop then stops restic, because
    /// the rest of the listing is not needed (AC-RS-02).
    pub(crate) fn page_complete(&self) -> bool {
        self.more_nodes
    }

    /// Takes one line of standard output.
    pub(crate) fn accept(&mut self, line: &str, truncated: bool) -> Accepted {
        match self.kind {
            OutputKind::Backup | OutputKind::Check | OutputKind::Restore => {
                self.accept_message(line, truncated)
            }
            OutputKind::Ls => self.accept_node(line, truncated),
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
            (OutputKind::Restore, Some("status")) => {
                Accepted::Progress(restore_progress(&message, &self.job_id, &self.step_id))
            }
            (OutputKind::Restore, Some("summary")) => {
                self.output = Some(step_result::Output::RestoreSummary(restore_summary(
                    &message,
                    self.dry_run,
                )));
                Accepted::Consumed
            }
            (OutputKind::Backup | OutputKind::Restore, Some("error")) => {
                Accepted::Log(backup_error(&message))
            }
            (_, Some("verbose_status" | "status")) => Accepted::Status(line.to_owned()),
            _ => Accepted::Log(line.to_owned()),
        }
    }

    fn accept_node(&mut self, line: &str, truncated: bool) -> Accepted {
        let message = if truncated {
            None
        } else {
            serde_json::from_str::<Value>(line).ok()
        };
        let Some(message) = message else {
            return Accepted::Log(line.to_owned());
        };
        let Some(window) = &self.window else {
            return Accepted::Consumed;
        };
        if message.get("message_type").and_then(Value::as_str) != Some("node") {
            return Accepted::Consumed;
        }
        let node = node(&message);
        if node.path.trim_end_matches('/') == window.directory {
            return Accepted::Consumed;
        }
        let index = self.nodes_seen;
        self.nodes_seen += 1;
        if index < window.skip || self.more_nodes {
            return Accepted::Consumed;
        }
        let size = node.encoded_len() + 6;
        if self.nodes.len() as u64 >= window.limit || self.nodes_bytes + size > MAX_OUTPUT_BYTES {
            self.more_nodes = true;
        } else {
            self.nodes_bytes += size;
            self.nodes.push(node);
        }
        Accepted::Consumed
    }

    /// Ends the step and returns the typed output, or `None` when restic printed none or printed
    /// output that the agent cannot parse.
    pub(crate) fn finish(mut self) -> Option<step_result::Output> {
        if self.kind == OutputKind::Ls {
            return self.window.is_some().then(|| {
                step_result::Output::LsPage(LsPage {
                    nodes: std::mem::take(&mut self.nodes),
                    last: !self.more_nodes,
                })
            });
        }
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

/// The progress of a restore: the `status` message names its fields in the restore way.
fn restore_progress(message: &Value, job_id: &str, step_id: &str) -> StepProgress {
    StepProgress {
        job_id: job_id.to_owned(),
        step_id: step_id.to_owned(),
        bytes_done: uint(message, "bytes_restored"),
        total_bytes: uint(message, "total_bytes"),
        files_done: uint(message, "files_restored"),
        total_files: uint(message, "total_files"),
        percent_done: float(message, "percent_done"),
        seconds_elapsed: float(message, "seconds_elapsed"),
        seconds_remaining: 0,
        current_files: Vec::new(),
    }
}

fn restore_summary(message: &Value, dry_run: bool) -> RestoreSummary {
    RestoreSummary {
        total_files: uint(message, "total_files"),
        files_restored: uint(message, "files_restored"),
        files_skipped: uint(message, "files_skipped"),
        files_deleted: uint(message, "files_deleted"),
        total_bytes: uint(message, "total_bytes"),
        bytes_restored: uint(message, "bytes_restored"),
        bytes_skipped: uint(message, "bytes_skipped"),
        seconds_elapsed: float(message, "seconds_elapsed"),
        dry_run,
    }
}

fn node(message: &Value) -> Node {
    let node_type = match message.get("type").and_then(Value::as_str) {
        Some("file") => NodeType::File,
        Some("dir") => NodeType::Dir,
        Some("symlink") => NodeType::Symlink,
        Some("dev") => NodeType::Device,
        Some("chardev") => NodeType::CharDevice,
        Some("fifo") => NodeType::Fifo,
        Some("socket") => NodeType::Socket,
        Some("irregular") => NodeType::Irregular,
        _ => NodeType::Unspecified,
    };
    Node {
        name: text(message, "name"),
        r#type: node_type as i32,
        path: text(message, "path"),
        uid: u32::try_from(uint(message, "uid")).unwrap_or(0),
        gid: u32::try_from(uint(message, "gid")).unwrap_or(0),
        size: i64::try_from(uint(message, "size")).unwrap_or(i64::MAX),
        mode: u32::try_from(uint(message, "mode")).unwrap_or(0),
        permissions: text(message, "permissions"),
        mtime: timestamp(message, "mtime"),
        atime: timestamp(message, "atime"),
        ctime: timestamp(message, "ctime"),
        link_target: text(message, "linktarget"),
    }
}

/// True when a page names a symbolic link, so the agent reads the tree object for the targets.
pub(crate) fn has_symlink(page: &LsPage) -> bool {
    page.nodes
        .iter()
        .any(|node| NodeType::try_from(node.r#type) == Ok(NodeType::Symlink))
}

/// Fills the target of each symbolic link of a page from the tree object of the listed directory.
///
/// `restic ls --json` names the type of a link but not its target; the tree object of the directory
/// holds the `linktarget` of each node (FR-RS-02). A node that the tree does not name keeps an empty
/// target.
pub(crate) fn fill_link_targets(page: &mut LsPage, tree: &[u8]) {
    let Ok(document) = serde_json::from_slice::<Value>(tree) else {
        return;
    };
    let Some(nodes) = document.get("nodes").and_then(Value::as_array) else {
        return;
    };
    for node in &mut page.nodes {
        if NodeType::try_from(node.r#type) != Ok(NodeType::Symlink) || !node.link_target.is_empty()
        {
            continue;
        }
        let target = nodes.iter().find(|entry| {
            entry.get("name").and_then(Value::as_str) == Some(node.name.as_str())
                && entry.get("type").and_then(Value::as_str) == Some("symlink")
        });
        if let Some(target) = target {
            node.link_target = text(target, "linktarget");
        }
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

/// The log text of a line on the standard error of restic.
///
/// With `--json` restic prints its errors as JSON messages: `error` for a failed item and
/// `exit_error` for the fatal message. The agent shows the readable message. A line that is not one
/// of them is a log line as it is (ADR-0011).
pub(crate) fn stderr_text(line: &str, truncated: bool) -> String {
    if truncated || !line.starts_with('{') {
        return line.to_owned();
    }
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return line.to_owned();
    };
    match message.get("message_type").and_then(Value::as_str) {
        Some("error") => backup_error(&message),
        Some("exit_error") => {
            let detail = message
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("error");
            match message.get("code").and_then(Value::as_i64) {
                Some(code) => format!("{detail} (exit code {code})"),
                None => detail.to_owned(),
            }
        }
        _ => line.to_owned(),
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
    let mut snapshots: Vec<Snapshot> = items.iter().skip(skip).map(snapshot).collect();
    let mut truncated = skip > 0;
    let mut size = 0;
    let mut keep = 0;
    // A snapshot costs its own size plus a tag and a length prefix.
    for snapshot in snapshots.iter().rev() {
        size += snapshot.encoded_len() + 6;
        if size > MAX_OUTPUT_BYTES {
            break;
        }
        keep += 1;
    }
    if keep < snapshots.len() {
        snapshots.drain(..snapshots.len() - keep);
        truncated = true;
    }
    SnapshotList {
        snapshots,
        truncated,
    }
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
    let mut truncated = false;
    let groups = document
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|group| {
            let reasons: HashMap<String, String> = group
                .get("reasons")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .filter_map(|reason| {
                    let id = text(reason.get("snapshot")?, "id");
                    Some((id, strings(reason, "matches").join(",")))
                })
                .collect();
            ForgetGroup {
                tags: strings(group, "tags"),
                host: text(group, "host"),
                paths: strings(group, "paths"),
                kept: forget_snapshots(group, "keep", &reasons, &mut truncated),
                removed: forget_snapshots(group, "remove", &reasons, &mut truncated),
            }
        })
        .collect();
    let mut result = ForgetResult { groups, truncated };
    fit_forget_result(&mut result);
    result
}

/// Drops snapshots, then groups, until the result fits the size limit.
fn fit_forget_result(result: &mut ForgetResult) {
    while result.encoded_len() > MAX_OUTPUT_BYTES {
        result.truncated = true;
        let longest = result
            .groups
            .iter_mut()
            .flat_map(|group| [&mut group.kept, &mut group.removed])
            .max_by_key(|list| list.len());
        match longest {
            Some(list) if !list.is_empty() => list.truncate(list.len() / 2),
            _ => {
                if result.groups.pop().is_none() {
                    return;
                }
            }
        }
    }
}

fn forget_snapshots(
    group: &Value,
    key: &str,
    reasons: &HashMap<String, String>,
    truncated: &mut bool,
) -> Vec<ForgetSnapshot> {
    let items = group
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    *truncated |= items.len() > MAX_SNAPSHOTS;
    items
        .iter()
        .take(MAX_SNAPSHOTS)
        .map(|item| {
            let id = text(item, "id");
            ForgetSnapshot {
                time: timestamp(item, "time"),
                reason: reasons.get(&id).cloned().unwrap_or_default(),
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
