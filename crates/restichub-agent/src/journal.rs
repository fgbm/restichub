//! The persistent job journal of the agent (ADR-0020, FR-PR-03, FR-AG-12).
//!
//! The agent records every accepted job and the result of every completed job, so a job that the
//! control plane delivers again does not run a second time; the agent re-sends the stored result
//! (ADR-0020, AC-PR-01). The journal holds no delivery mark: a send into the stream buffer does not
//! prove that the control plane got the result. The agent sends every stored result at each
//! connection until the entry expires, and the control plane ignores a result for a job that is
//! already terminal. The entry lifetime therefore bounds the re-sends.
//!
//! The journal is a protected file in the agent configuration directory. It is replaced atomically,
//! so a reader sees the old or the new content (FR-AG-04). Every entry carries an expiry: the
//! control plane always sets `expires_at`, and the agent refuses a job without one before it reaches
//! the journal. The agent prunes the entries past their expiry at load and at every completion
//! (ADR-0020). The journal holds at most `MAX_ENTRIES` jobs. When it is full,
//! [`FileJournal::make_room`] drops the expired entries only, so an unexpired result is never lost
//! to a new job: when only unexpired entries remain, the caller refuses the new job instead.

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use prost::Message as _;
use prost_types::Timestamp;
use restichub_proto::agent::v1::JobResult;
use restichub_proto::signing::{CLOCK_SKEW, JobJournal, JournalState};
use serde::{Deserialize, Serialize};

use crate::paths::replace_file;

/// The name of the journal file in the agent configuration directory.
pub const JOURNAL_FILE: &str = "job-journal.json";

/// The largest number of jobs that the journal keeps.
const MAX_ENTRIES: usize = 4096;

/// The most log bytes that all stored results hold together. When the total is larger, the journal
/// drops the log tail of the oldest results first, so the file stays small however many jobs it
/// holds (FR-AG-12).
const MAX_TAIL_BYTES: u64 = 4 * 1024 * 1024;

/// One job in the journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    job_id: String,
    /// Expiry of the job in Unix seconds. The control plane always sets it (ADR-0020).
    expires_at: i64,
    /// Base64 of the serialized [`JobResult`], or `None` while the job runs.
    result: Option<String>,
    /// Bytes of log in the stored result, so the journal can cap the total without decoding.
    #[serde(default)]
    tail_bytes: u64,
}

/// The serialized journal, in the order of acceptance.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredJournal {
    entries: Vec<Entry>,
}

/// The journal of one agent, backed by a file (ADR-0020).
pub struct FileJournal {
    path: PathBuf,
    entries: Vec<Entry>,
    /// Counts the snapshots, so a late write never replaces a newer one.
    sequence: u64,
}

/// The journal at one moment. The caller serializes and writes it outside the lock of the journal
/// and off the async workers (ADR-0020).
pub struct JournalSnapshot {
    path: PathBuf,
    entries: Vec<Entry>,
    sequence: u64,
}

impl JournalSnapshot {
    /// Writes the snapshot unless a newer one is already on disk (FR-AG-04).
    ///
    /// `written` holds the sequence of the newest write and serializes the writers: the replace uses
    /// one temporary file.
    ///
    /// # Errors
    ///
    /// Returns an error when the journal cannot be serialized or the file cannot be replaced.
    pub fn write(&self, written: &Mutex<u64>) -> Result<()> {
        let mut newest = written
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.sequence <= *newest {
            return Ok(());
        }
        replace_file(&self.path, &serialize(&self.entries)?)?;
        *newest = self.sequence;
        Ok(())
    }
}

impl FileJournal {
    /// Reads the journal from `path` (ADR-0020).
    ///
    /// A missing file is an empty journal. A malformed file is an error: the agent cannot prove that
    /// a job is new, so it refuses to run rather than risk a second run (AC-PR-01).
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or does not parse.
    pub fn load(path: PathBuf) -> Result<Self> {
        let entries = match std::fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice::<StoredJournal>(&bytes)
                    .with_context(|| format!("the job journal {} is malformed", path.display()))?
                    .entries
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", path.display()));
            }
        };
        Ok(Self {
            path,
            entries,
            sequence: 0,
        })
    }

    /// Writes the journal to its file with an atomic replace (FR-AG-04).
    ///
    /// # Errors
    ///
    /// Returns an error when the journal cannot be serialized or the file cannot be replaced.
    pub fn persist(&self) -> Result<()> {
        let bytes = self.serialize()?;
        replace_file(&self.path, &bytes)
    }

    /// Copies the journal for a write outside the lock of the journal (ADR-0020).
    pub fn snapshot(&mut self) -> JournalSnapshot {
        self.sequence += 1;
        JournalSnapshot {
            path: self.path.clone(),
            entries: self.entries.clone(),
            sequence: self.sequence,
        }
    }

    fn serialize(&self) -> Result<Vec<u8>> {
        serialize(&self.entries)
    }

    /// Drops the log tail of the oldest results until the stored tails fit [`MAX_TAIL_BYTES`].
    fn trim_tails(&mut self) {
        let mut total: u64 = self.entries.iter().map(|entry| entry.tail_bytes).sum();
        for entry in &mut self.entries {
            if total <= MAX_TAIL_BYTES {
                return;
            }
            if entry.tail_bytes == 0 {
                continue;
            }
            let Some(mut result) = entry.result.as_deref().and_then(decode_result) else {
                continue;
            };
            result.log_tail.clear();
            entry.result = Some(STANDARD.encode(result.encode_to_vec()));
            total -= entry.tail_bytes;
            entry.tail_bytes = 0;
        }
    }

    /// The stored results of the completed jobs that have not expired (ADR-0020).
    #[must_use]
    pub fn stored_results(&self) -> Vec<JobResult> {
        self.entries
            .iter()
            .filter_map(|entry| entry.result.as_deref())
            .filter_map(decode_result)
            .collect()
    }

    /// Removes the entry of `job_id` (ADR-0020).
    ///
    /// The caller rolls back an acceptance whose persistence failed, so the journal never claims a
    /// job that the agent did not record on disk.
    pub fn forget(&mut self, job_id: &str) {
        self.entries.retain(|entry| entry.job_id != job_id);
    }

    /// Drops the entries past their expiry (ADR-0020).
    ///
    /// The caller passes the current time, so the pruning is independent of the wall clock inside
    /// this method.
    pub fn prune(&mut self, now: Timestamp) {
        let now = i128::from(now.seconds) + i128::from(now.nanos) / 1_000_000_000;
        let retain = i128::from(CLOCK_SKEW.as_secs());
        self.entries
            .retain(|entry| i128::from(entry.expires_at) + retain > now);
    }

    /// Makes room for a new entry and reports whether the journal can hold it (ADR-0020).
    ///
    /// The method drops the expired entries only. A full journal of unexpired entries gives
    /// `false`, so the caller refuses the new job instead of dropping a result that the control
    /// plane may not have seen.
    pub fn make_room(&mut self, now: Timestamp) -> bool {
        self.prune(now);
        self.entries.len() < MAX_ENTRIES
    }
}

impl JobJournal for FileJournal {
    fn lookup(&self, job_id: &str) -> JournalState {
        match self.entries.iter().find(|entry| entry.job_id == job_id) {
            None => JournalState::Unknown,
            Some(entry) => match entry.result.as_deref().and_then(decode_result) {
                Some(result) => JournalState::Completed(result),
                None => JournalState::InProgress,
            },
        }
    }

    fn record_accepted(&mut self, job_id: &str, expires_at: Timestamp) {
        if self.entries.iter().any(|entry| entry.job_id == job_id) {
            return;
        }
        self.entries.push(Entry {
            job_id: job_id.to_owned(),
            expires_at: expires_at.seconds,
            result: None,
            tail_bytes: 0,
        });
    }

    fn record_completed(&mut self, job_id: &str, result: JobResult) {
        let encoded = STANDARD.encode(result.encode_to_vec());
        let tail_bytes = result
            .log_tail
            .iter()
            .map(|chunk| chunk.data.len() as u64)
            .sum();
        // The agent records the acceptance before it runs the job, so the entry exists here. A
        // missing entry would have no expiry and could never be pruned, so it is not stored.
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.job_id == job_id) {
            entry.result = Some(encoded);
            entry.tail_bytes = tail_bytes;
            self.trim_tails();
        }
    }

    fn purge_expired(&mut self, now: Timestamp) {
        self.prune(now);
    }
}

fn serialize(entries: &[Entry]) -> Result<Vec<u8>> {
    #[derive(Serialize)]
    struct Stored<'a> {
        entries: &'a [Entry],
    }
    serde_json::to_vec(&Stored { entries }).context("cannot serialize the job journal")
}

/// Decodes a stored result. A value that does not decode is skipped, so it cannot block the drain.
fn decode_result(encoded: &str) -> Option<JobResult> {
    let bytes = STANDARD.decode(encoded).ok()?;
    JobResult::decode(bytes.as_slice()).ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use prost::Message as _;
    use restichub_proto::agent::v1::{JobResult, LogChunk};
    use restichub_proto::signing::{JobJournal, JournalState};

    use super::{FileJournal, decode_result};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "restichub-agent-journal-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        dir.join(super::JOURNAL_FILE)
    }

    fn result(job_id: &str) -> JobResult {
        JobResult {
            job_id: job_id.to_owned(),
            ..JobResult::default()
        }
    }

    fn at(seconds: i64) -> prost_types::Timestamp {
        prost_types::Timestamp { seconds, nanos: 0 }
    }

    #[test]
    fn ac_pr_01_a_completed_job_returns_its_stored_result() {
        let mut journal = FileJournal::load(temp_path("stored")).expect("cannot load");
        journal.record_accepted("job-1", at(2_000));
        assert_eq!(journal.lookup("job-1"), JournalState::InProgress);
        journal.record_completed("job-1", result("job-1"));
        assert_eq!(
            journal.lookup("job-1"),
            JournalState::Completed(result("job-1"))
        );
    }

    #[test]
    fn a_result_survives_a_reload() {
        let path = temp_path("reload");
        let mut journal = FileJournal::load(path.clone()).expect("cannot load");
        journal.record_accepted("job-1", at(2_000));
        journal.record_completed("job-1", result("job-1"));
        journal.persist().expect("cannot persist");

        let reloaded = FileJournal::load(path).expect("cannot load");
        assert_eq!(
            reloaded.lookup("job-1"),
            JournalState::Completed(result("job-1"))
        );
    }

    #[test]
    fn adr_0020_a_stored_result_stays_until_the_entry_expires() {
        let mut journal = FileJournal::load(temp_path("stored-results")).expect("cannot load");
        journal.record_accepted("job-1", at(2_000));
        journal.record_accepted("job-2", at(2_000));
        journal.record_completed("job-1", result("job-1"));
        // A job that still runs has no result to send.
        assert_eq!(journal.stored_results(), vec![result("job-1")]);
        assert_eq!(journal.stored_results(), vec![result("job-1")]);
        journal.prune(at(2_000 + 60 * 5));
        assert!(journal.stored_results().is_empty());
    }

    #[test]
    fn adr_0020_a_full_journal_of_expired_entries_makes_room() {
        let mut journal = FileJournal::load(temp_path("room")).expect("cannot load");
        for index in 0..super::MAX_ENTRIES {
            let job_id = format!("job-{index}");
            journal.record_accepted(&job_id, at(2_000));
            journal.record_completed(&job_id, result(&job_id));
        }
        assert!(
            journal.make_room(at(2_000 + 60 * 5)),
            "an expired entry makes room"
        );
        assert!(journal.entries.len() < super::MAX_ENTRIES);
    }

    #[test]
    fn adr_0020_a_late_snapshot_does_not_replace_a_newer_one() {
        let path = temp_path("snapshots");
        let mut journal = FileJournal::load(path.clone()).expect("cannot load");
        let written = std::sync::Mutex::new(0);
        journal.record_accepted("job-1", at(2_000));
        let older = journal.snapshot();
        journal.record_accepted("job-2", at(2_000));
        let newer = journal.snapshot();
        newer.write(&written).expect("cannot write");
        older.write(&written).expect("cannot write");

        let reloaded = FileJournal::load(path).expect("cannot load");
        assert_eq!(reloaded.lookup("job-2"), JournalState::InProgress);
    }

    #[test]
    fn adr_0020_a_full_journal_of_live_entries_refuses_a_new_job() {
        let mut journal = FileJournal::load(temp_path("full")).expect("cannot load");
        for index in 0..super::MAX_ENTRIES {
            let job_id = format!("job-{index}");
            journal.record_accepted(&job_id, at(2_000));
            journal.record_completed(&job_id, result(&job_id));
        }
        // The entries are live and unexpired, so a new job does not replace one.
        assert!(
            !journal.make_room(at(1_000)),
            "live entries must not be dropped"
        );
        assert_eq!(journal.entries.len(), super::MAX_ENTRIES);
    }

    #[test]
    fn an_entry_past_its_expiry_is_pruned() {
        let mut journal = FileJournal::load(temp_path("prune-expired")).expect("cannot load");
        // The lifetime is one hour, so the entry is pruned after it plus the clock skew.
        journal.record_accepted("job-1", at(0));
        journal.record_completed("job-1", result("job-1"));
        journal.prune(at(0));
        assert_eq!(
            journal.lookup("job-1"),
            JournalState::Completed(result("job-1"))
        );
        journal.prune(at(60 * 60 + 60 * 5));
        assert_eq!(journal.lookup("job-1"), JournalState::Unknown);
    }

    #[test]
    fn fr_ag_12_the_stored_log_tails_stay_inside_the_bound() {
        let path = temp_path("tails");
        let mut journal = FileJournal::load(path.clone()).expect("cannot load");
        // 100 results with a 60 KiB tail each hold 6 MiB of log, more than the bound.
        for index in 0..100 {
            let job_id = format!("job-{index}");
            journal.record_accepted(&job_id, at(2_000));
            journal.record_completed(
                &job_id,
                JobResult {
                    log_tail: vec![LogChunk {
                        job_id: job_id.clone(),
                        data: vec![b'x'; 60 * 1024],
                        ..LogChunk::default()
                    }],
                    ..result(&job_id)
                },
            );
        }
        journal.persist().expect("cannot persist");
        let size = std::fs::metadata(&path).expect("metadata").len();
        // The tails take 4 MiB at most, and base64 adds a third.
        assert!(size <= super::MAX_TAIL_BYTES * 4 / 3 + 64 * 1024, "{size}");
        let results = journal.stored_results();
        assert_eq!(results.len(), 100, "no result is lost");
        assert!(results[0].log_tail.is_empty(), "the oldest tail is dropped");
        assert_eq!(results[99].log_tail.len(), 1, "the newest tail stays");
    }

    #[test]
    fn a_stored_result_round_trips() {
        let encoded = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            result("job-1").encode_to_vec(),
        );
        assert_eq!(decode_result(&encoded), Some(result("job-1")));
        assert_eq!(decode_result("not base64"), None);
    }
}
