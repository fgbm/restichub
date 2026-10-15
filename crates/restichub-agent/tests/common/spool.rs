//! Helpers for the log spool tests: a temporary directory that cleans up, and the control plane
//! side of the log (FR-LG-07).

#![allow(clippy::expect_used)]

use std::collections::HashSet;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use restichub_agent::jobs::Jobs;
use restichub_agent::log_spool::SPOOL_DIR;
use restichub_proto::agent::v1::{AgentMessage, LogAck, LogChunk, LogStreamAck, agent_message};
use tokio::sync::mpsc;

use super::{
    AGENT, TENANT, access, backup_step, job, ring, signed, temp_config_dir,
};

/// A directory that the test removes at the end, with the files that the fake restic keeps next
/// to a location.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        let dir = temp_config_dir(name);
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        Self(dir)
    }
}

impl Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        let _ = std::fs::remove_file(format!("{}.restichub-out-backup", self.0.display()));
    }
}

/// The log of a job as the control plane stores it: a chunk once, in the order of arrival.
#[derive(Default)]
pub struct StoredLog {
    seen: HashSet<(i32, u64)>,
    text: String,
    chunks: Vec<LogChunk>,
}

impl StoredLog {
    pub fn take(&mut self, chunk: &LogChunk) {
        if self.seen.insert((chunk.stream, chunk.sequence)) {
            self.text.push_str(&String::from_utf8_lossy(&chunk.data));
            self.chunks.push(chunk.clone());
        }
    }

    pub fn lines(&self) -> Vec<&str> {
        self.text.lines().collect()
    }

    /// The acknowledgement of everything stored so far.
    pub fn ack(&self, job_id: &str) -> LogAck {
        let mut newest: Vec<(i32, u64)> = Vec::new();
        for chunk in &self.chunks {
            match newest
                .iter_mut()
                .find(|(stream, _)| *stream == chunk.stream)
            {
                Some(entry) => entry.1 = entry.1.max(chunk.sequence),
                None => newest.push((chunk.stream, chunk.sequence)),
            }
        }
        LogAck {
            job_id: job_id.to_owned(),
            streams: newest
                .into_iter()
                .map(|(stream, sequence)| LogStreamAck { stream, sequence })
                .collect(),
        }
    }
}

/// Takes what the stream holds. True when a job result was among it.
pub fn drain(rx: &mut mpsc::Receiver<AgentMessage>, stored: &mut StoredLog) -> bool {
    let mut result = false;
    while let Ok(message) = rx.try_recv() {
        match message.payload {
            Some(agent_message::Payload::LogChunk(chunk)) => stored.take(&chunk),
            Some(agent_message::Payload::JobResult(_)) => result = true,
            _ => {}
        }
    }
    result
}

/// Takes from the stream until `lines` lines are stored, or panics after a bound.
pub async fn collect_lines(
    rx: &mut mpsc::Receiver<AgentMessage>,
    stored: &mut StoredLog,
    lines: usize,
) {
    let started = Instant::now();
    while stored.lines().len() < lines {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "only {} of {lines} lines arrived",
            stored.lines().len()
        );
        drain(rx, stored);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

pub async fn wait_for_result(dir: &Path) {
    let journal = dir.join(restichub_agent::journal::JOURNAL_FILE);
    for _ in 0..600 {
        if std::fs::read_to_string(&journal)
            .unwrap_or_default()
            .matches("\"result\":\"")
            .count()
            > 0
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the job did not finish");
}

pub fn spool_files(dir: &Path) -> usize {
    std::fs::read_dir(dir.join(SPOOL_DIR)).map_or(0, |entries| entries.flatten().count())
}

pub async fn wait_for_spool_files(dir: &Path, count: usize) {
    for _ in 0..200 {
        if spool_files(dir) == count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(spool_files(dir), count);
}

pub fn lines_script(prefix: &str, count: usize) -> String {
    (1..=count).map(|n| format!("{prefix}-{n:04}\n")).collect()
}

pub async fn start_job(jobs: &Jobs, repo: &Path, job_id: &str) {
    let job = job(
        job_id,
        vec![access("repo-1", repo, "topsecret")],
        vec![backup_step("repo-1", &["/srv"])],
    );
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
}

/// Connects like the stream task, up to the greeting: the stored results go out first.
pub async fn connect(
    jobs: &Jobs,
    capacity: usize,
) -> (mpsc::Sender<AgentMessage>, mpsc::Receiver<AgentMessage>) {
    let (tx, rx) = mpsc::channel(capacity);
    jobs.attach(&tx);
    jobs.send_stored_results().await;
    (tx, rx)
}
