//! The persistent job journal of the agent (ADR-0020, FR-PR-03, FR-AG-12).
//!
//! The agent records every accepted job and the result of every completed job, so a job that the
//! control plane delivers again does not run a second time; the agent re-sends the stored result
//! (ADR-0020, AC-PR-01). A result that the agent could not send, because the stream was down, stays
//! in the journal and is re-sent at the next connection.
//!
//! The journal is a protected file in the agent configuration directory. It is replaced atomically,
//! so a reader sees the old or the new content (FR-AG-04), and it is bounded: the entries past their
//! expiry are pruned at load and at every completion, and the journal holds at most
//! `MAX_ENTRIES` jobs. An entry past its expiry is safe to drop, because the agent rejects an
//! expired job before it consults the journal (ADR-0020).

use std::path::PathBuf;

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

/// One job in the journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    job_id: String,
    /// Expiry of the job in Unix seconds, or `None` when the job carried none.
    expires_at: Option<i64>,
    /// Base64 of the serialized [`JobResult`], or `None` while the job runs.
    result: Option<String>,
    /// True once the agent has sent the stored result to the control plane.
    delivered: bool,
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
        Ok(Self { path, entries })
    }

    /// Writes the journal to its file with an atomic replace (FR-AG-04).
    ///
    /// # Errors
    ///
    /// Returns an error when the journal cannot be serialized or the file cannot be replaced.
    pub fn persist(&self) -> Result<()> {
        let stored = StoredJournal {
            entries: self.entries.clone(),
        };
        let bytes = serde_json::to_vec(&stored).context("cannot serialize the job journal")?;
        replace_file(&self.path, &bytes)
    }

    /// The completed results that the agent has not sent yet (ADR-0020).
    #[must_use]
    pub fn pending_results(&self) -> Vec<JobResult> {
        self.entries
            .iter()
            .filter(|entry| !entry.delivered)
            .filter_map(|entry| entry.result.as_deref())
            .filter_map(decode_result)
            .collect()
    }

    /// Marks the stored result of `job_id` as sent to the control plane.
    pub fn mark_delivered(&mut self, job_id: &str) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.job_id == job_id) {
            entry.delivered = true;
        }
    }

    /// Drops the entries past their expiry and keeps the journal within `MAX_ENTRIES`.
    ///
    /// The caller passes the current time, so the pruning is independent of the wall clock inside
    /// this method.
    pub fn prune(&mut self, now: Timestamp) {
        let now = i128::from(now.seconds) + i128::from(now.nanos) / 1_000_000_000;
        let retain = i128::from(CLOCK_SKEW.as_secs());
        self.entries.retain(|entry| match entry.expires_at {
            Some(expires_at) => i128::from(expires_at) + retain > now,
            None => true,
        });
        if self.entries.len() > MAX_ENTRIES {
            let excess = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..excess);
        }
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
            expires_at: Some(expires_at.seconds),
            result: None,
            delivered: false,
        });
    }

    fn record_completed(&mut self, job_id: &str, result: JobResult) {
        let encoded = STANDARD.encode(result.encode_to_vec());
        match self.entries.iter_mut().find(|entry| entry.job_id == job_id) {
            Some(entry) => {
                entry.result = Some(encoded);
                entry.delivered = false;
            }
            None => self.entries.push(Entry {
                job_id: job_id.to_owned(),
                expires_at: None,
                result: Some(encoded),
                delivered: false,
            }),
        }
    }

    fn purge_expired(&mut self, now: Timestamp) {
        self.prune(now);
    }
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
    use restichub_proto::agent::v1::JobResult;
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
    fn an_undelivered_result_is_pending_until_it_is_marked() {
        let mut journal = FileJournal::load(temp_path("pending")).expect("cannot load");
        journal.record_accepted("job-1", at(2_000));
        journal.record_completed("job-1", result("job-1"));
        assert_eq!(journal.pending_results(), vec![result("job-1")]);
        journal.mark_delivered("job-1");
        assert!(journal.pending_results().is_empty());
    }

    #[test]
    fn an_entry_past_its_expiry_is_pruned() {
        let mut journal = FileJournal::load(temp_path("prune")).expect("cannot load");
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
    fn the_journal_is_bounded() {
        let mut journal = FileJournal::load(temp_path("bounded")).expect("cannot load");
        for index in 0..(super::MAX_ENTRIES + 10) {
            // No expiry: the cap is the only bound.
            journal.record_completed(&format!("job-{index}"), result(&format!("job-{index}")));
        }
        journal.prune(at(1_000));
        assert_eq!(journal.entries.len(), super::MAX_ENTRIES);
        // Pruning drops the oldest entries first.
        assert_eq!(journal.lookup("job-0"), JournalState::Unknown);
        assert_eq!(journal.lookup("job-9"), JournalState::Unknown);
        assert_ne!(journal.lookup("job-10"), JournalState::Unknown);
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
