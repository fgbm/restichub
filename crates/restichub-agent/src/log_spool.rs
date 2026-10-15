//! The spool of unacknowledged job log chunks (FR-LG-07).
//!
//! The agent keeps each log chunk of a job until the control plane acknowledges it with a `LogAck`.
//! After a reconnect or a restart of the agent, it sends every chunk that is still in the spool, in
//! the order of the original writes, and the control plane stores a chunk once (AC-PR-07).
//!
//! Each job has one protected file in the `log-spool` directory of the configuration directory.
//! The file is a sequence of records: a four-byte big-endian length and one encoded `LogChunk`.
//! The chunks have passed the scrubber already, so the file holds no secret (NFR-SEC-05). The
//! spool also keeps the chunks in memory for the send path; the memory use has the same bound as
//! the disk use.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result};
use prost::Message as _;
use restichub_proto::agent::v1::{LogAck, LogChunk};

use crate::paths::{create_protected_file, replace_file};

/// The directory of the spool files, in the configuration directory.
pub const SPOOL_DIR: &str = "log-spool";

/// The default cap for all spooled chunks of all jobs together.
pub const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// The environment variable that overrides [`DEFAULT_MAX_BYTES`].
pub const MAX_BYTES_VARIABLE: &str = "RESTICHUB_LOG_SPOOL_MAX_BYTES";

const FILE_EXTENSION: &str = "spool";

/// Bytes that a loss marker record takes at most.
const MARKER_RESERVE: u64 = 160;

const PREFIX: u64 = 4;

#[derive(Debug, PartialEq, Eq)]
pub enum Stored {
    Kept,
    /// The cap is reached. The chunk is not in the spool.
    Full,
}

struct JobSpool {
    path: PathBuf,
    chunks: Vec<LogChunk>,
    /// Bytes of the records that the file holds, acknowledged ones included.
    file_bytes: u64,
    /// Bytes of the chunks that are still unacknowledged.
    live_bytes: u64,
    /// How many leading chunks the current connection already got.
    sent: usize,
    /// The job has ended, so no chunk comes any more.
    finished: bool,
}

#[derive(Default)]
struct State {
    jobs: HashMap<String, JobSpool>,
    total: u64,
}

pub struct LogSpool {
    dir: PathBuf,
    max_bytes: u64,
    state: Mutex<State>,
}

fn record_size(chunk: &LogChunk) -> u64 {
    PREFIX + chunk.encoded_len() as u64
}

/// Only identifiers that cannot leave the directory give a file name.
fn file_name(job_id: &str) -> Option<String> {
    let plain = !job_id.is_empty()
        && job_id.len() <= 64
        && job_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    plain.then(|| format!("{job_id}.{FILE_EXTENSION}"))
}

fn encode(chunk: &LogChunk) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(record_size(chunk) as usize);
    bytes.extend_from_slice(&(chunk.encoded_len() as u32).to_be_bytes());
    chunk.encode_raw(&mut bytes);
    bytes
}

/// Reads the records of a spool file. A torn record at the end, left by a crash, ends the list.
fn decode_all(bytes: &[u8]) -> Vec<LogChunk> {
    let mut chunks = Vec::new();
    let mut rest = bytes;
    while let Some((prefix, tail)) = rest.split_first_chunk::<4>() {
        let length = u32::from_be_bytes(*prefix) as usize;
        let Some(record) = tail.get(..length) else {
            break;
        };
        let Ok(chunk) = LogChunk::decode(record) else {
            break;
        };
        chunks.push(chunk);
        rest = &tail[length..];
    }
    chunks
}

fn lines_in(chunk: &LogChunk) -> u64 {
    chunk.data.iter().filter(|byte| **byte == b'\n').count() as u64
}

/// Number of log lines in `chunk`, for the loss marker.
#[must_use]
pub fn line_count(chunk: &LogChunk) -> u64 {
    lines_in(chunk)
}

impl LogSpool {
    /// Opens the spool in `dir` and loads the chunks that an earlier process left.
    ///
    /// A file that cannot be read is left alone: the next sweep removes it when no job needs it.
    #[must_use]
    pub fn open(dir: &Path, max_bytes: u64) -> Self {
        let spool = Self {
            dir: dir.join(SPOOL_DIR),
            max_bytes,
            state: Mutex::new(State::default()),
        };
        spool.load();
        spool
    }

    /// The cap from the environment, or the default.
    #[must_use]
    pub fn max_bytes_from_env() -> u64 {
        std::env::var(MAX_BYTES_VARIABLE)
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or(DEFAULT_MAX_BYTES)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn load(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut state = self.lock();
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(job_id) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(&format!(".{FILE_EXTENSION}")))
                .map(str::to_owned)
            else {
                continue;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let chunks = decode_all(&bytes);
            let live: u64 = chunks.iter().map(record_size).sum();
            if state.total + live > self.max_bytes {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            state.total += live;
            state.jobs.insert(
                job_id,
                JobSpool {
                    path,
                    chunks,
                    file_bytes: bytes.len() as u64,
                    live_bytes: live,
                    sent: 0,
                    finished: true,
                },
            );
        }
    }

    /// Stores a chunk of a running job. The chunk is on disk when the call returns.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be written; the chunk is then not in the spool.
    pub fn append(&self, job_id: &str, chunk: &LogChunk) -> Result<Stored> {
        self.append_within(job_id, chunk, false)
    }

    /// Like [`Self::append`], and ignores the cap. Only the loss marker uses it: one short line
    /// per job, so the log never ends without the report of what it lost (FR-LG-07).
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be written.
    pub fn append_marker(&self, job_id: &str, chunk: &LogChunk) -> Result<Stored> {
        self.append_within(job_id, chunk, true)
    }

    fn append_within(&self, job_id: &str, chunk: &LogChunk, force: bool) -> Result<Stored> {
        let Some(name) = file_name(job_id) else {
            return Ok(Stored::Full);
        };
        let size = record_size(chunk);
        let mut state = self.lock();
        if !force && state.total + size > self.max_bytes {
            return Ok(Stored::Full);
        }
        let path = self.dir.join(name);
        let job = state
            .jobs
            .entry(job_id.to_owned())
            .or_insert_with(|| JobSpool {
                path,
                chunks: Vec::new(),
                file_bytes: 0,
                live_bytes: 0,
                sent: 0,
                finished: false,
            });
        job.finished = false;
        write_record(job, chunk)?;
        job.chunks.push(chunk.clone());
        job.file_bytes += size;
        job.live_bytes += size;
        state.total += size;
        Ok(Stored::Kept)
    }

    /// True when the cap leaves room for `chunk` and for a loss marker before it.
    #[must_use]
    pub fn fits_with_marker(&self, chunk: &LogChunk) -> bool {
        self.lock().total + record_size(chunk) + MARKER_RESERVE <= self.max_bytes
    }

    /// The chunks that the current connection did not get yet. They count as sent from now on.
    #[must_use]
    pub fn claim_unsent(&self, job_id: &str) -> Vec<LogChunk> {
        let mut state = self.lock();
        let Some(job) = state.jobs.get_mut(job_id) else {
            return Vec::new();
        };
        let batch = job.chunks[job.sent.min(job.chunks.len())..].to_vec();
        job.sent = job.chunks.len();
        batch
    }

    /// Takes `count` chunks back from the sent mark after a failed send.
    pub fn unclaim(&self, job_id: &str, count: usize) {
        if let Some(job) = self.lock().jobs.get_mut(job_id) {
            job.sent = job.sent.saturating_sub(count);
        }
    }

    /// Marks every chunk as not sent: a new connection starts from the first unacknowledged chunk.
    pub fn rewind_all(&self) {
        for job in self.lock().jobs.values_mut() {
            job.sent = 0;
        }
    }

    #[must_use]
    pub fn pending_jobs(&self) -> Vec<String> {
        self.lock()
            .jobs
            .iter()
            .filter(|(_, job)| !job.chunks.is_empty())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// True when the job still has chunks that the control plane did not acknowledge.
    #[must_use]
    pub fn holds(&self, job_id: &str) -> bool {
        self.lock()
            .jobs
            .get(job_id)
            .is_some_and(|job| !job.chunks.is_empty())
    }

    /// Removes the chunks that the control plane acknowledged and shrinks the file.
    pub fn acknowledge(&self, ack: &LogAck) {
        let mut state = self.lock();
        let state = &mut *state;
        let Some(job) = state.jobs.get_mut(&ack.job_id) else {
            return;
        };
        let covered = |chunk: &LogChunk| {
            ack.streams
                .iter()
                .any(|stream| stream.stream == chunk.stream && chunk.sequence <= stream.sequence)
        };
        let mut index = 0;
        let mut removed_sent = 0;
        let mut removed_bytes = 0;
        job.chunks.retain(|chunk| {
            let drop = covered(chunk);
            if drop {
                removed_bytes += record_size(chunk);
                if index < job.sent {
                    removed_sent += 1;
                }
            }
            index += 1;
            !drop
        });
        job.sent -= removed_sent;
        job.live_bytes -= removed_bytes;
        state.total -= removed_bytes;
        compact(job);
        if job.chunks.is_empty() && job.finished {
            state.jobs.remove(&ack.job_id);
        }
    }

    /// Tells the spool that no chunk of the job comes any more.
    pub fn finish(&self, job_id: &str) {
        let mut state = self.lock();
        let state = &mut *state;
        if let Some(job) = state.jobs.get_mut(job_id) {
            job.finished = true;
            if job.chunks.is_empty() {
                compact(job);
                state.jobs.remove(job_id);
            }
        }
    }

    /// Removes the chunks of every job outside `keep`, and the files that no job owns.
    pub fn sweep(&self, keep: &HashSet<String>) {
        let mut state = self.lock();
        let state = &mut *state;
        let stale: Vec<String> = state
            .jobs
            .keys()
            .filter(|id| !keep.contains(*id))
            .cloned()
            .collect();
        for id in stale {
            if let Some(job) = state.jobs.remove(&id) {
                state.total -= job.live_bytes;
                let _ = std::fs::remove_file(&job.path);
            }
        }
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let known = state.jobs.values().any(|job| job.path == path);
            if !known {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// Bytes of all unacknowledged chunks.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.lock().total
    }
}

fn write_record(job: &mut JobSpool, chunk: &LogChunk) -> Result<()> {
    let record = encode(chunk);
    if job.file_bytes == 0 {
        if let Some(parent) = job.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let _ = std::fs::remove_file(&job.path);
        create_protected_file(&job.path, &record)?;
        return Ok(());
    }
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&job.path)
        .with_context(|| format!("cannot open {}", job.path.display()))?;
    file.write_all(&record)
        .with_context(|| format!("cannot write {}", job.path.display()))
}

/// Rewrites the file without the acknowledged records, or removes it when none is left. The
/// file shrinks only when the acknowledged part is at least half of it, so a busy job does not
/// rewrite the file at every acknowledgement.
fn compact(job: &mut JobSpool) {
    if job.chunks.is_empty() {
        if job.file_bytes > 0 {
            let _ = std::fs::remove_file(&job.path);
            job.file_bytes = 0;
        }
        return;
    }
    if job.file_bytes - job.live_bytes < job.file_bytes / 2 {
        return;
    }
    let bytes: Vec<u8> = job.chunks.iter().flat_map(encode).collect();
    match replace_file(&job.path, &bytes) {
        Ok(()) => job.file_bytes = bytes.len() as u64,
        Err(error) => tracing::warn!(%error, "cannot shrink the log spool file"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use restichub_proto::agent::v1::{LogStream, LogStreamAck};

    use super::*;

    fn chunk(stream: LogStream, sequence: u64, text: &str) -> LogChunk {
        LogChunk {
            job_id: "job-1".to_owned(),
            step_id: "step-1".to_owned(),
            stream: stream as i32,
            sequence,
            data: text.as_bytes().to_vec(),
        }
    }

    fn ack(sequence: u64) -> LogAck {
        LogAck {
            job_id: "job-1".to_owned(),
            streams: vec![LogStreamAck {
                stream: LogStream::Stdout as i32,
                sequence,
            }],
        }
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rh-spool-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cannot create the test directory");
        dir
    }

    #[test]
    fn fr_lg_07_a_restart_loads_the_unacknowledged_chunks_in_order() {
        let dir = dir("restart");
        let spool = LogSpool::open(&dir, DEFAULT_MAX_BYTES);
        for sequence in 1..=3 {
            spool
                .append("job-1", &chunk(LogStream::Stdout, sequence, "line\n"))
                .expect("append");
        }
        spool.acknowledge(&ack(1));
        drop(spool);

        let reopened = LogSpool::open(&dir, DEFAULT_MAX_BYTES);
        let sequences: Vec<u64> = reopened
            .claim_unsent("job-1")
            .iter()
            .map(|chunk| chunk.sequence)
            .collect();
        assert_eq!(sequences, [2, 3]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fr_lg_07_the_cap_refuses_a_chunk_that_does_not_fit() {
        let dir = dir("cap");
        let one = chunk(LogStream::Stdout, 1, "0123456789\n");
        let spool = LogSpool::open(&dir, record_size(&one) * 2);
        assert_eq!(spool.append("job-1", &one).expect("append"), Stored::Kept);
        let two = chunk(LogStream::Stdout, 2, "0123456789\n");
        assert_eq!(spool.append("job-1", &two).expect("append"), Stored::Kept);
        let three = chunk(LogStream::Stdout, 3, "0123456789\n");
        assert_eq!(spool.append("job-1", &three).expect("append"), Stored::Full);
        assert_eq!(spool.claim_unsent("job-1").len(), 2);
        spool.acknowledge(&ack(2));
        assert_eq!(spool.append("job-1", &three).expect("append"), Stored::Kept);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fr_lg_07_acknowledged_data_leaves_the_disk() {
        let dir = dir("ack");
        let spool = LogSpool::open(&dir, DEFAULT_MAX_BYTES);
        for sequence in 1..=4 {
            spool
                .append("job-1", &chunk(LogStream::Stdout, sequence, "line\n"))
                .expect("append");
        }
        let path = dir.join(SPOOL_DIR).join("job-1.spool");
        let full = std::fs::metadata(&path).expect("the file exists").len();
        spool.acknowledge(&ack(3));
        let smaller = std::fs::metadata(&path).expect("the file exists").len();
        assert!(smaller < full, "{smaller} < {full}");
        spool.finish("job-1");
        spool.acknowledge(&ack(4));
        assert!(
            !path.exists(),
            "the file of a fully acknowledged job is removed"
        );
        assert_eq!(spool.total_bytes(), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fr_lg_07_a_torn_record_at_the_end_is_ignored() {
        let mut bytes = encode(&chunk(LogStream::Stdout, 1, "one\n"));
        bytes.extend_from_slice(&encode(&chunk(LogStream::Stdout, 2, "two\n"))[..6]);
        let chunks = decode_all(&bytes);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn fr_lg_07_a_job_identifier_cannot_leave_the_directory() {
        assert!(file_name("../etc").is_none());
        assert!(file_name("").is_none());
        assert!(file_name("0b7c-12ab").is_some());
    }

    #[cfg(unix)]
    #[test]
    fn fr_lg_07_the_spool_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = dir("mode");
        let spool = LogSpool::open(&dir, DEFAULT_MAX_BYTES);
        spool
            .append("job-1", &chunk(LogStream::Stdout, 1, "line\n"))
            .expect("append");
        let mode = std::fs::metadata(dir.join(SPOOL_DIR).join("job-1.spool"))
            .expect("the file exists")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(dir);
    }
}
