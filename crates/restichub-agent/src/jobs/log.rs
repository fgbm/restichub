//! The bounded log of one job (FR-AG-08, FR-LG-03).
//!
//! The log of a job holds at most [`LOG_LIMIT`] bytes. The agent then adds one marker line and drops
//! the rest. The last [`TAIL_LIMIT`] bytes also stay in the job journal, so the control plane gets
//! them after a reconnect (AC-PR-07).

use std::collections::VecDeque;

use restichub_proto::agent::v1::{LogChunk, LogStream};

/// Bytes of log that one job sends. After that the log ends with a marker (FR-AG-08).
pub(super) const LOG_LIMIT: usize = 1024 * 1024;

/// The most log bytes that the journal keeps for a job.
pub(super) const TAIL_LIMIT: usize = 64 * 1024;

/// The size at which the agent starts a new chunk.
const CHUNK_LIMIT: usize = 8 * 1024;

/// Size of the sequence table: one slot for each `LogStream` value.
const STREAMS: usize = 3;

/// The log lines of one job, waiting to be sent, and the tail for the journal.
pub(super) struct JobLog {
    job_id: String,
    used: usize,
    truncated: bool,
    search_path_noted: bool,
    pending: Vec<LogChunk>,
    pending_bytes: usize,
    tail: VecDeque<LogChunk>,
    tail_bytes: usize,
    sequence: [u64; STREAMS],
    /// Lines that the full spool refused since the last marker.
    lost_lines: u64,
    last_step: String,
}

impl JobLog {
    pub(super) fn new(job_id: &str) -> Self {
        Self {
            job_id: job_id.to_owned(),
            used: 0,
            truncated: false,
            search_path_noted: false,
            pending: Vec::new(),
            pending_bytes: 0,
            tail: VecDeque::new(),
            tail_bytes: 0,
            sequence: [0; STREAMS],
            lost_lines: 0,
            last_step: String::new(),
        }
    }

    pub(super) fn job_id(&self) -> &str {
        &self.job_id
    }

    /// Lines refused by the spool that no marker reports yet.
    pub(super) fn lost_lines(&self) -> u64 {
        self.lost_lines
    }

    /// Counts lines that the spool could not keep (FR-LG-07).
    pub(super) fn note_lost(&mut self, lines: u64) {
        self.lost_lines += lines;
    }

    /// The marker chunk for the lines lost so far, or `None` when nothing was lost. The marker
    /// takes the place of the gap, so the log never has a silent hole (FR-LG-07).
    pub(super) fn take_loss_marker(&mut self) -> Option<LogChunk> {
        if self.lost_lines == 0 {
            return None;
        }
        let lines = std::mem::take(&mut self.lost_lines);
        let text = if lines == 1 {
            "1 log line was lost while the control plane was unreachable\n".to_owned()
        } else {
            format!("{lines} log lines were lost while the control plane was unreachable\n")
        };
        let slot = &mut self.sequence[LogStream::Agent as usize % STREAMS];
        *slot += 1;
        Some(LogChunk {
            job_id: self.job_id.clone(),
            step_id: self.last_step.clone(),
            stream: LogStream::Agent as i32,
            sequence: *slot,
            data: text.into_bytes(),
        })
    }

    /// Adds the line about the dropped search path entries to the first step that has one. Later
    /// steps of the job add nothing, because the entries are the same.
    pub(super) fn note_search_path(&mut self, step_id: &str, line: &str) {
        if !self.search_path_noted {
            self.search_path_noted = true;
            self.push(step_id, LogStream::Agent, line);
        }
    }

    /// Adds one scrubbed line of a step. After the limit the line is dropped.
    pub(super) fn push(&mut self, step_id: &str, stream: LogStream, line: &str) {
        if self.truncated {
            return;
        }
        let size = line.len() + 1;
        if self.used + size > LOG_LIMIT {
            self.truncated = true;
            let marker = format!("log truncated: the limit of {LOG_LIMIT} bytes is reached\n");
            self.append(step_id, LogStream::Agent, &marker);
            return;
        }
        self.used += size;
        self.append(step_id, stream, &format!("{line}\n"));
    }

    fn append(&mut self, step_id: &str, stream: LogStream, text: &str) {
        if self.last_step != step_id {
            step_id.clone_into(&mut self.last_step);
        }
        self.pending_bytes += text.len();
        if let Some(last) = self.pending.last_mut()
            && last.step_id == step_id
            && last.stream == stream as i32
            && last.data.len() + text.len() <= CHUNK_LIMIT
        {
            last.data.extend_from_slice(text.as_bytes());
            return;
        }
        let slot = &mut self.sequence[stream as usize % STREAMS];
        *slot += 1;
        self.pending.push(LogChunk {
            job_id: self.job_id.clone(),
            step_id: step_id.to_owned(),
            stream: stream as i32,
            sequence: *slot,
            data: text.as_bytes().to_vec(),
        });
    }

    pub(super) fn pending_bytes(&self) -> usize {
        self.pending_bytes
    }

    /// Takes the chunks to send and moves a copy into the tail.
    pub(super) fn take_pending(&mut self) -> Vec<LogChunk> {
        let chunks = std::mem::take(&mut self.pending);
        self.pending_bytes = 0;
        for chunk in &chunks {
            self.tail_bytes += chunk.data.len();
            self.tail.push_back(chunk.clone());
        }
        while self.tail_bytes > TAIL_LIMIT
            && let Some(old) = self.tail.pop_front()
        {
            self.tail_bytes -= old.data.len();
        }
        chunks
    }

    pub(super) fn tail(&self) -> Vec<LogChunk> {
        self.tail.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use restichub_proto::agent::v1::LogStream;

    use super::{JobLog, LOG_LIMIT, TAIL_LIMIT};

    #[test]
    fn fr_ag_08_lines_of_one_stream_share_a_chunk() {
        let mut log = JobLog::new("job-1");
        log.push("step-1", LogStream::Stderr, "one");
        log.push("step-1", LogStream::Stderr, "two");
        log.push("step-2", LogStream::Stderr, "three");
        let chunks = log.take_pending();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].data, b"one\ntwo\n");
        assert_eq!(chunks[0].sequence, 1);
        assert_eq!(chunks[1].step_id, "step-2");
        assert_eq!(chunks[1].sequence, 2);
        assert_eq!(chunks[0].job_id, "job-1");
    }

    #[test]
    fn fr_ag_08_the_log_stops_at_the_limit_with_a_marker() {
        let mut log = JobLog::new("job-1");
        let line = "x".repeat(1023);
        let mut total = 0;
        for _ in 0..(LOG_LIMIT / 1024 + 50) {
            log.push("step-1", LogStream::Stdout, &line);
            for chunk in log.take_pending() {
                total += chunk.data.len();
            }
        }
        // The sent log stays within the limit plus the one marker line.
        assert!(total <= LOG_LIMIT + 100, "{total}");
        assert!(total >= LOG_LIMIT - 1024, "{total}");
        let tail = log.tail();
        let last = tail.last().expect("the tail holds the marker");
        assert_eq!(last.stream, LogStream::Agent as i32);
        assert!(String::from_utf8_lossy(&last.data).contains("log truncated"));
        // After the marker the log drops every line.
        log.push("step-2", LogStream::Stdout, "late");
        assert!(log.take_pending().is_empty());
    }

    #[test]
    fn fr_lg_07_the_marker_counts_the_lost_lines_once() {
        let mut log = JobLog::new("job-1");
        assert!(log.take_loss_marker().is_none());
        log.note_lost(3);
        log.note_lost(2);
        let marker = log.take_loss_marker().expect("a marker");
        assert_eq!(
            String::from_utf8_lossy(&marker.data),
            "5 log lines were lost while the control plane was unreachable\n"
        );
        assert_eq!(marker.stream, LogStream::Agent as i32);
        assert!(log.take_loss_marker().is_none());
        log.note_lost(1);
        let single = log.take_loss_marker().expect("a marker");
        assert_eq!(single.sequence, 2);
        assert!(String::from_utf8_lossy(&single.data).starts_with("1 log line was lost"));
    }

    #[test]
    fn ac_pr_07_the_tail_is_bounded_and_keeps_the_newest_lines() {
        let mut log = JobLog::new("job-1");
        for index in 0..400 {
            log.push("step-1", LogStream::Stdout, &format!("{index:0>500}"));
            let _ = log.take_pending();
        }
        let tail = log.tail();
        let bytes: usize = tail.iter().map(|chunk| chunk.data.len()).sum();
        assert!(bytes <= TAIL_LIMIT, "{bytes}");
        let last = tail.last().expect("the tail has chunks");
        assert!(String::from_utf8_lossy(&last.data).contains(&format!("{:0>500}", 399)));
    }
}
