//! The acknowledgements of log chunks on one agent connection (FR-LG-07).
//!
//! An acknowledgement covers every chunk of the stream up to its sequence, because the agent sends
//! the chunks in order. The connection therefore keeps only the newest sequence of each stream and
//! sends one acknowledgement for a job on a timer, never one for each chunk. That keeps the shared
//! outbound buffer free for pings and cancels, however many chunks arrive.
//!
//! A chunk that fails to store for now stops the job on this connection: the control plane stores
//! no later chunk of it and sends no acknowledgement, because a later chunk stored first would put
//! its lines before the missing ones. The connection then closes, so the agent sends the job again
//! from its first unacknowledged chunk.

use std::collections::{HashMap, HashSet};

use restichub_proto::agent::v1::{ControlMessage, LogAck, LogChunk, LogStreamAck, control_message};
use tokio::sync::mpsc;

use crate::job_data::ChunkOutcome;

/// The most jobs with a failed log write that one connection remembers. Past it the connection
/// stops all acknowledgements, so no later acknowledgement can cover a chunk that was not stored.
const MAX_FAILED_JOBS: usize = 1024;

/// The most jobs with a waiting acknowledgement. A job past the bound waits for a later chunk.
const MAX_WAITING_JOBS: usize = 1024;

pub(crate) struct LogAcks {
    enabled: bool,
    failed: HashSet<String>,
    /// The newest stored sequence of each stream, for each job, not yet sent.
    waiting: HashMap<String, HashMap<i32, u64>>,
    reconnect: bool,
}

impl LogAcks {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            failed: HashSet::new(),
            waiting: HashMap::new(),
            reconnect: false,
        }
    }

    /// False for a job whose chunks the connection must not store any more.
    pub(crate) fn admits(&self, job_id: &str) -> bool {
        !self.enabled || !self.failed.contains(job_id)
    }

    /// Records what became of a chunk.
    pub(crate) fn record(&mut self, chunk: &LogChunk, outcome: ChunkOutcome) {
        if !self.enabled {
            return;
        }
        match outcome {
            ChunkOutcome::Failed => {
                if self.failed.len() < MAX_FAILED_JOBS {
                    self.failed.insert(chunk.job_id.clone());
                    self.waiting.remove(&chunk.job_id);
                    self.reconnect = true;
                } else {
                    self.enabled = false;
                    self.waiting.clear();
                }
            }
            ChunkOutcome::Stored | ChunkOutcome::Refused(_) => {
                if self.failed.contains(&chunk.job_id) {
                    return;
                }
                if !self.waiting.contains_key(&chunk.job_id)
                    && self.waiting.len() >= MAX_WAITING_JOBS
                {
                    return;
                }
                let newest = self
                    .waiting
                    .entry(chunk.job_id.clone())
                    .or_default()
                    .entry(chunk.stream)
                    .or_insert(0);
                *newest = (*newest).max(chunk.sequence);
            }
        }
    }

    pub(crate) fn has_waiting(&self) -> bool {
        !self.waiting.is_empty()
    }

    /// True once when a store failure asks for a new connection.
    pub(crate) fn take_reconnect(&mut self) -> bool {
        std::mem::take(&mut self.reconnect)
    }

    /// Sends the waiting acknowledgements while the buffer keeps a quarter of its room for other
    /// messages. What does not fit waits for the next call; an acknowledgement lost is harmless,
    /// because the agent keeps the chunk.
    pub(crate) fn flush(&mut self, tx: &mpsc::Sender<ControlMessage>) {
        let reserve = tx.max_capacity() / 4;
        let jobs: Vec<String> = self.waiting.keys().cloned().collect();
        for job_id in jobs {
            if tx.capacity() <= reserve {
                return;
            }
            let Some(streams) = self.waiting.remove(&job_id) else {
                continue;
            };
            let ack = LogAck {
                job_id: job_id.clone(),
                streams: streams
                    .iter()
                    .map(|(stream, sequence)| LogStreamAck {
                        stream: *stream,
                        sequence: *sequence,
                    })
                    .collect(),
            };
            let sent = tx.try_send(ControlMessage {
                payload: Some(control_message::Payload::LogAck(ack)),
            });
            if sent.is_err() {
                self.waiting.insert(job_id, streams);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use restichub_proto::agent::v1::LogStream;

    use super::*;
    use crate::job_data::ChunkRefusal;

    fn chunk(job: &str, sequence: u64) -> LogChunk {
        LogChunk {
            job_id: job.to_owned(),
            step_id: "backup".to_owned(),
            stream: LogStream::Stdout as i32,
            sequence,
            data: Vec::new(),
        }
    }

    fn acks(rx: &mut mpsc::Receiver<ControlMessage>) -> Vec<LogAck> {
        let mut found = Vec::new();
        while let Ok(message) = rx.try_recv() {
            if let Some(control_message::Payload::LogAck(ack)) = message.payload {
                found.push(ack);
            }
        }
        found
    }

    #[test]
    fn ac_lg_05_many_chunks_make_one_acknowledgement_with_the_newest_sequence() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut log_acks = LogAcks::new(true);
        for sequence in 1..=500 {
            log_acks.record(&chunk("job-1", sequence), ChunkOutcome::Stored);
        }
        log_acks.flush(&tx);
        let sent = acks(&mut rx);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].streams.len(), 1);
        assert_eq!(sent[0].streams[0].sequence, 500);
        assert!(!log_acks.has_waiting());
    }

    #[test]
    fn ac_lg_05_a_resent_older_chunk_does_not_lower_the_acknowledgement() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut log_acks = LogAcks::new(true);
        log_acks.record(&chunk("job-1", 9), ChunkOutcome::Stored);
        log_acks.record(&chunk("job-1", 3), ChunkOutcome::Stored);
        log_acks.flush(&tx);
        assert_eq!(acks(&mut rx)[0].streams[0].sequence, 9);
    }

    #[test]
    fn ac_lg_05_acknowledgements_leave_room_for_other_messages() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut log_acks = LogAcks::new(true);
        for job in 0..20 {
            log_acks.record(&chunk(&format!("job-{job}"), 1), ChunkOutcome::Stored);
        }
        log_acks.flush(&tx);
        assert_eq!(tx.capacity(), 2, "a quarter of the buffer stays free");
        assert!(log_acks.has_waiting(), "the rest waits");
        assert_eq!(acks(&mut rx).len(), 6);
        log_acks.flush(&tx);
        assert_eq!(acks(&mut rx).len(), 6);
        // A full buffer drops nothing: the acknowledgement stays waiting.
        let (full, _keep) = mpsc::channel(4);
        for _ in 0..4 {
            full.try_send(ControlMessage::default()).expect("fill");
        }
        let mut waiting = LogAcks::new(true);
        waiting.record(&chunk("job-1", 1), ChunkOutcome::Stored);
        waiting.flush(&full);
        assert!(waiting.has_waiting());
    }

    #[test]
    fn ac_lg_05_a_refused_chunk_is_acknowledged_so_it_cannot_block_the_next_ones() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut log_acks = LogAcks::new(true);
        log_acks.record(&chunk("job-1", 1), ChunkOutcome::Stored);
        log_acks.record(
            &chunk("job-1", 2),
            ChunkOutcome::Refused(ChunkRefusal::TooLarge),
        );
        log_acks.record(&chunk("job-1", 3), ChunkOutcome::Stored);
        log_acks.flush(&tx);
        assert_eq!(acks(&mut rx)[0].streams[0].sequence, 3);
        assert!(!log_acks.take_reconnect());
    }

    #[test]
    fn ac_lg_05_a_failed_store_stops_the_job_and_asks_for_a_new_connection() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut log_acks = LogAcks::new(true);
        log_acks.record(&chunk("job-1", 1), ChunkOutcome::Stored);
        log_acks.record(&chunk("job-1", 2), ChunkOutcome::Failed);
        assert!(!log_acks.admits("job-1"), "no later chunk is stored");
        assert!(log_acks.admits("job-2"));
        log_acks.record(&chunk("job-1", 3), ChunkOutcome::Stored);
        log_acks.record(&chunk("job-2", 1), ChunkOutcome::Stored);
        assert!(log_acks.take_reconnect());
        assert!(!log_acks.take_reconnect());
        log_acks.flush(&tx);
        let sent = acks(&mut rx);
        assert_eq!(sent.len(), 1, "no acknowledgement covers the failed chunk");
        assert_eq!(sent[0].job_id, "job-2");
    }

    #[test]
    fn ac_lg_05_too_many_failed_jobs_stop_every_acknowledgement() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut log_acks = LogAcks::new(true);
        for job in 0..MAX_FAILED_JOBS {
            log_acks.record(&chunk(&format!("job-{job}"), 1), ChunkOutcome::Failed);
        }
        log_acks.record(&chunk("late", 1), ChunkOutcome::Failed);
        log_acks.record(&chunk("late", 2), ChunkOutcome::Stored);
        log_acks.record(&chunk("other", 1), ChunkOutcome::Stored);
        log_acks.flush(&tx);
        assert!(acks(&mut rx).is_empty());
        assert!(log_acks.admits("late"));
    }

    #[test]
    fn ac_lg_05_a_connection_without_the_capability_keeps_the_old_behavior() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut log_acks = LogAcks::new(false);
        log_acks.record(&chunk("job-1", 1), ChunkOutcome::Failed);
        log_acks.record(&chunk("job-1", 2), ChunkOutcome::Stored);
        assert!(log_acks.admits("job-1"));
        assert!(!log_acks.take_reconnect());
        log_acks.flush(&tx);
        assert!(acks(&mut rx).is_empty());
    }
}
