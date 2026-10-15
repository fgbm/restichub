//! The delivery of job log chunks through the spool (FR-LG-07).
//!
//! A running job only writes its chunks into the spool and wakes the drain task. One drain task per
//! connection reads the spool and sends the chunks in order, so no other task can reorder them and
//! no step of a job waits for the control plane. The task leaves room in the stream buffer for the
//! control messages of the agent, and every wait in it is bounded (FR-AG-10).
//!
//! From the attach of a stream until the greeting the agent does not know whether the control plane
//! acknowledges chunks, so the chunks wait in the spool and count as unacknowledged. A control plane
//! that lists the capability gets the whole spool from its first unacknowledged chunk. Any other
//! control plane gets the spool once and the agent drops what it sent, as it did before the spool.

use std::sync::atomic::Ordering;
use std::time::Duration;

use restichub_proto::agent::v1::{AgentMessage, LogAck, LogChunk, agent_message};
use tokio::sync::mpsc;

use super::{Jobs, lock};
use crate::log_spool::{self, Stored};

/// How long the drain task waits before it looks at the spool again, when nothing wakes it.
const DRAIN_RETRY: Duration = Duration::from_millis(250);

/// The most chunks that the drain task takes from one job at a time.
const DRAIN_BATCH: usize = 64;

/// How long the drain task waits for room in the stream buffer before it gives the chunk back.
const CHUNK_SEND_WAIT: Duration = Duration::from_secs(5);

/// How long the end of a job waits for its log to leave the agent before the job result follows.
const LOG_SENT_WAIT: Duration = Duration::from_secs(5);

/// What the agent knows about the log path of the open stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LogMode {
    /// The control plane does not acknowledge chunks, or no stream was open yet.
    Direct,
    /// A stream is open, and the greeting did not tell yet.
    Pending,
    /// The control plane acknowledges chunks.
    Acked,
}

impl LogMode {
    fn code(self) -> u8 {
        match self {
            Self::Direct => 0,
            Self::Pending => 1,
            Self::Acked => 2,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Pending,
            2 => Self::Acked,
            _ => Self::Direct,
        }
    }
}

impl Jobs {
    pub(super) fn log_mode(&self) -> LogMode {
        LogMode::from_code(self.log_mode.load(Ordering::SeqCst))
    }

    pub(super) fn set_log_mode(&self, mode: LogMode) {
        self.log_mode.store(mode.code(), Ordering::SeqCst);
    }

    /// Starts the drain task of a stream and stops the one of the stream before.
    pub(super) fn start_log_drain(&self, tx: mpsc::Sender<AgentMessage>) {
        let jobs = self.clone();
        let task = tokio::spawn(async move { jobs.drain_logs(tx).await });
        if let Some(previous) = lock(&self.log_drain).replace(task.abort_handle()) {
            previous.abort();
        }
    }

    /// Records the greeting: whether the control plane acknowledges log chunks (FR-LG-07).
    ///
    /// The drain task sends the spool from its first chunk. The call returns at once.
    pub fn greeted(&self, log_acks: bool) {
        self.log_spool.rewind_all();
        self.set_log_mode(if log_acks {
            LogMode::Acked
        } else {
            LogMode::Direct
        });
        self.log_spool.sweep();
        self.log_wake.notify_one();
    }

    /// Removes the chunks that the control plane acknowledged (FR-LG-07).
    pub fn log_ack(&self, ack: &LogAck) {
        self.log_spool.acknowledge(ack);
    }

    async fn drain_logs(&self, tx: mpsc::Sender<AgentMessage>) {
        while !tx.is_closed() {
            let mode = self.log_mode();
            if mode != LogMode::Pending {
                self.drain_once(&tx, mode).await;
            }
            tokio::select! {
                () = self.log_wake.notified() => {}
                () = tokio::time::sleep(DRAIN_RETRY) => {}
            }
        }
    }

    /// Sends every chunk that this stream did not get yet. A chunk that finds no room goes back
    /// to the spool for the next round.
    async fn drain_once(&self, tx: &mpsc::Sender<AgentMessage>, mode: LogMode) {
        for job_id in self.log_spool.pending_jobs() {
            loop {
                let batch = self.log_spool.claim_unsent(&job_id, DRAIN_BATCH);
                if batch.is_empty() {
                    break;
                }
                let total = batch.len();
                for (index, chunk) in batch.into_iter().enumerate() {
                    if !send_chunk(tx, chunk).await {
                        self.log_spool.unclaim(&job_id, total - index);
                        return;
                    }
                }
                if mode == LogMode::Direct {
                    self.log_spool.release_sent(&job_id);
                }
            }
        }
    }

    /// Takes the pending chunks of a job. They go into the spool when the control plane may
    /// acknowledge them; otherwise the agent sends them once and a chunk without room is lost.
    /// The journal tail still keeps the newest ones (AC-PR-07).
    pub(super) async fn send_log(&self, log: &mut super::log::JobLog) {
        let chunks = log.take_pending();
        if chunks.is_empty() {
            return;
        }
        if self.log_mode() == LogMode::Direct && !self.log_spool.holds(log.job_id()) {
            for chunk in chunks {
                self.send_within(
                    AgentMessage {
                        payload: Some(agent_message::Payload::LogChunk(chunk)),
                    },
                    super::runner::SEND_WAIT,
                )
                .await;
            }
            return;
        }
        for chunk in chunks {
            self.spool_chunk(log, chunk);
        }
        self.log_wake.notify_one();
    }

    /// When the cap is reached the agent drops the chunk and counts its lines. The next chunk that
    /// fits follows one marker line with the count (FR-LG-07).
    fn spool_chunk(&self, log: &mut super::log::JobLog, chunk: LogChunk) {
        let lost = log.lost_lines();
        if lost > 0 && !self.log_spool.fits_with_marker(&chunk) {
            log.note_lost(log_spool::line_count(&chunk));
            return;
        }
        if let Some(marker) = log.take_loss_marker()
            && !self.keep_in_spool(log.job_id(), &marker)
        {
            log.note_lost(lost);
        }
        if !self.keep_in_spool(log.job_id(), &chunk) {
            log.note_lost(log_spool::line_count(&chunk));
        }
    }

    /// Ends the log of a job: reports the lines that the spool still owes and tells the spool that
    /// nothing follows (FR-LG-07).
    pub(super) fn close_log(&self, log: &mut super::log::JobLog) {
        if let Some(marker) = log.take_loss_marker()
            && let Err(error) = self.log_spool.append_marker(log.job_id(), &marker)
        {
            tracing::warn!(%error, "cannot write the log spool");
        }
        self.log_spool.finish(log.job_id());
        self.log_wake.notify_one();
    }

    fn keep_in_spool(&self, job_id: &str, chunk: &LogChunk) -> bool {
        match self.log_spool.append(job_id, chunk) {
            Ok(Stored::Kept) => true,
            Ok(Stored::Full) => false,
            Err(error) => {
                tracing::warn!(%error, "cannot write the log spool");
                false
            }
        }
    }

    /// Waits until the drain task sent the log of the job, so the job result follows its last
    /// lines. The wait ends when the stream closes or after a bound; the spool keeps the rest.
    pub(super) async fn wait_log_sent(&self, job_id: &str) {
        let deadline = tokio::time::Instant::now() + LOG_SENT_WAIT;
        while self.log_spool.has_unsent(job_id) {
            let open = lock(&self.connection)
                .as_ref()
                .is_some_and(|tx| !tx.is_closed());
            if !open || tokio::time::Instant::now() >= deadline {
                return;
            }
            self.log_wake.notify_one();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// Sends one chunk without taking the last slots of the stream buffer, which the control messages
/// of the agent need. False when the stream closed or no room appeared within the bound.
async fn send_chunk(tx: &mpsc::Sender<AgentMessage>, chunk: LogChunk) -> bool {
    let mut message = AgentMessage {
        payload: Some(agent_message::Payload::LogChunk(chunk)),
    };
    let reserve = tx.max_capacity() / 4;
    let deadline = tokio::time::Instant::now() + CHUNK_SEND_WAIT;
    loop {
        if tx.capacity() > reserve {
            match tx.try_send(message) {
                Ok(()) => return true,
                Err(mpsc::error::TrySendError::Closed(_)) => return false,
                Err(mpsc::error::TrySendError::Full(back)) => message = back,
            }
        }
        if tx.is_closed() || tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
