//! Job logs of an emulated agent (FR-AG-08, FR-LG-07).
//!
//! A step writes synthetic lines at the profile rate and sends them as `LogChunk`s. A chunk stays
//! in the spool of the agent until a `LogAck` covers it, and a new stream sends the spool again,
//! as the real agent does. The spool keeps only the position of each chunk and makes the text
//! again for a resend, so a large fleet holds no log text in memory.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use restichub_proto::agent::v1::{AgentMessage, LogAck, LogChunk, LogStream, agent_message};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use crate::lock;
use crate::metrics::Metrics;

/// Bytes of log that one job of the real agent sends before the truncation marker.
const LOG_LIMIT: usize = 1024 * 1024;

/// The size at which the real agent starts a new chunk.
const CHUNK_LIMIT: usize = 8 * 1024;

const TICK: Duration = Duration::from_millis(250);

/// Jobs that one spool tracks; the oldest goes first, so a control plane that never acknowledges
/// cannot grow the spool without a bound.
const MAX_JOBS: usize = 64;

const TRUNCATION_MARKER: &str = "log truncated: the limit of 1048576 bytes is reached\n";

#[derive(Debug, Clone, Copy)]
pub struct LogLoad {
    pub lines_per_second: f64,
    pub line_bytes: usize,
}

/// Where one chunk sits in the synthetic log of a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkRecord {
    step_id: String,
    stream: LogStream,
    sequence: u64,
    first_line: u64,
    lines: u64,
    line_bytes: usize,
}

impl ChunkRecord {
    fn data(&self) -> Vec<u8> {
        if self.stream == LogStream::Agent {
            return TRUNCATION_MARKER.as_bytes().to_vec();
        }
        (self.first_line..self.first_line + self.lines)
            .flat_map(|number| line_text(number, self.line_bytes))
            .collect()
    }

    fn chunk(&self, job_id: &str) -> LogChunk {
        LogChunk {
            job_id: job_id.to_owned(),
            step_id: self.step_id.clone(),
            stream: self.stream as i32,
            sequence: self.sequence,
            data: self.data(),
        }
    }
}

/// One line of `line_bytes` bytes with its newline. The text holds no secret.
#[must_use]
pub fn line_text(number: u64, line_bytes: usize) -> Vec<u8> {
    let mut line = format!("emulated log line {number} ").into_bytes();
    line.resize(line_bytes.saturating_sub(1), b'.');
    line.push(b'\n');
    line
}

#[must_use]
pub fn due_lines(per_second: f64, elapsed: Duration) -> u64 {
    (per_second * elapsed.as_secs_f64()).floor() as u64
}

/// The log of one job: the line count, the sequences and the byte budget.
pub struct JobLog {
    job_id: String,
    load: LogLoad,
    next_line: u64,
    stdout_sequence: u64,
    used: usize,
    truncated: bool,
}

impl JobLog {
    #[must_use]
    pub fn new(job_id: &str, load: LogLoad) -> Self {
        Self {
            job_id: job_id.to_owned(),
            load,
            next_line: 1,
            stdout_sequence: 0,
            used: 0,
            truncated: false,
        }
    }

    /// Cuts `lines` new lines of a step into chunks. At the byte limit of a job the log ends with
    /// one marker chunk, as the log of the real agent.
    pub fn take(&mut self, step_id: &str, lines: u64) -> Vec<ChunkRecord> {
        if self.truncated || lines == 0 {
            return Vec::new();
        }
        let line_bytes = self.load.line_bytes;
        let budget = ((LOG_LIMIT - self.used) / line_bytes) as u64;
        let granted = lines.min(budget);
        let per_chunk = (CHUNK_LIMIT / line_bytes).max(1) as u64;
        let mut records = Vec::new();
        let mut left = granted;
        while left > 0 {
            let count = left.min(per_chunk);
            self.stdout_sequence += 1;
            records.push(ChunkRecord {
                step_id: step_id.to_owned(),
                stream: LogStream::Stdout,
                sequence: self.stdout_sequence,
                first_line: self.next_line,
                lines: count,
                line_bytes,
            });
            self.next_line += count;
            left -= count;
        }
        self.used += granted as usize * line_bytes;
        if granted < lines {
            self.truncated = true;
            records.push(ChunkRecord {
                step_id: step_id.to_owned(),
                stream: LogStream::Agent,
                sequence: 1,
                first_line: 0,
                lines: 0,
                line_bytes,
            });
        }
        records
    }
}

/// The chunks that wait for a `LogAck`, by job, oldest job first.
pub struct LogSpool {
    jobs: std::sync::Mutex<Vec<(String, Vec<ChunkRecord>)>>,
    /// Set when the control plane did not list `log-ack-v1`: then a sent chunk is not kept.
    direct: AtomicBool,
    metrics: Arc<Metrics>,
}

impl LogSpool {
    #[must_use]
    pub fn new(metrics: Arc<Metrics>) -> Self {
        Self {
            jobs: std::sync::Mutex::new(Vec::new()),
            direct: AtomicBool::new(false),
            metrics,
        }
    }

    fn keep(&self, job_id: &str, record: ChunkRecord) {
        if self.direct.load(Ordering::Relaxed) {
            return;
        }
        let mut jobs = lock(&self.jobs);
        match jobs.iter_mut().find(|(id, _)| id == job_id) {
            Some((_, records)) => records.push(record),
            None => {
                if jobs.len() >= MAX_JOBS {
                    let (_, dropped) = jobs.remove(0);
                    self.metrics.log_chunks_dropped(dropped.len());
                }
                jobs.push((job_id.to_owned(), vec![record]));
            }
        }
        self.metrics.log_chunks_kept(1);
    }

    /// Removes the chunks that `ack` covers and returns their count.
    pub fn acknowledge(&self, ack: &LogAck) -> usize {
        let mut jobs = lock(&self.jobs);
        let Some(position) = jobs.iter().position(|(id, _)| *id == ack.job_id) else {
            return 0;
        };
        let records = &mut jobs[position].1;
        let before = records.len();
        records.retain(|record| {
            !ack.streams.iter().any(|stream| {
                stream.stream == record.stream as i32 && record.sequence <= stream.sequence
            })
        });
        let removed = before - records.len();
        if records.is_empty() {
            jobs.remove(position);
        }
        self.metrics.log_chunks_acked(removed);
        removed
    }

    /// Records whether the control plane acknowledges chunks, and returns the chunks to send
    /// again on the new stream. Without acknowledgements the spool is emptied.
    pub fn greeted(&self, acks: bool) -> Vec<LogChunk> {
        self.direct.store(!acks, Ordering::Relaxed);
        let mut jobs = lock(&self.jobs);
        if !acks {
            let count = jobs.iter().map(|(_, records)| records.len()).sum();
            jobs.clear();
            self.metrics.log_chunks_dropped(count);
            return Vec::new();
        }
        jobs.iter()
            .flat_map(|(job_id, records)| records.iter().map(|record| record.chunk(job_id)))
            .collect()
    }

    /// Chunks that wait for an acknowledgement.
    #[must_use]
    pub fn len(&self) -> usize {
        lock(&self.jobs)
            .iter()
            .map(|(_, records)| records.len())
            .sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Where the chunks of a step go.
pub struct LogSink<'a> {
    pub tx: &'a mpsc::Sender<AgentMessage>,
    pub spool: &'a LogSpool,
    pub metrics: &'a Metrics,
}

impl LogSink<'_> {
    async fn send(&self, job_id: &str, record: ChunkRecord) {
        let chunk = record.chunk(job_id);
        self.metrics.log_chunk_sent(chunk.data.len());
        // The record goes into the spool first, so an acknowledgement never comes before it.
        self.spool.keep(job_id, record);
        let _ = self
            .tx
            .send(AgentMessage {
                payload: Some(agent_message::Payload::LogChunk(chunk)),
            })
            .await;
    }
}

/// Runs `step` and writes its log at the profile rate until the step ends.
pub async fn alongside<F: Future>(
    step: F,
    log: Option<&mut JobLog>,
    step_id: &str,
    sink: &LogSink<'_>,
) -> F::Output {
    let Some(log) = log else {
        return step.await;
    };
    tokio::pin!(step);
    let started = Instant::now();
    let mut written = 0u64;
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        let finished = tokio::select! {
            output = &mut step => Some(output),
            _ = ticker.tick() => None,
        };
        let due = due_lines(log.load.lines_per_second, started.elapsed());
        let records = log.take(step_id, due.saturating_sub(written));
        written = written.max(due);
        let job_id = log.job_id.clone();
        for record in records {
            sink.send(&job_id, record).await;
        }
        if let Some(output) = finished {
            return output;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use restichub_proto::agent::v1::LogStreamAck;

    const LOAD: LogLoad = LogLoad {
        lines_per_second: 10.0,
        line_bytes: 100,
    };

    #[test]
    fn a_line_has_the_profile_size_and_one_newline() {
        let line = line_text(7, 100);

        assert_eq!(line.len(), 100);
        assert!(line.starts_with(b"emulated log line 7 "));
        assert_eq!(line.iter().filter(|byte| **byte == b'\n').count(), 1);
        assert_eq!(line_text(7, 1), b"\n");
    }

    #[test]
    fn the_lines_due_follow_the_rate() {
        assert_eq!(due_lines(10.0, Duration::from_millis(2_550)), 25);
        assert_eq!(due_lines(0.0, Duration::from_secs(60)), 0);
    }

    #[test]
    fn fr_ag_08_lines_go_into_chunks_of_at_most_8_kib() {
        let mut log = JobLog::new("job-1", LOAD);
        let records = log.take("step-1", 200);

        assert_eq!(records.len(), 3);
        assert_eq!(records.iter().map(|record| record.lines).sum::<u64>(), 200);
        for (index, record) in records.iter().enumerate() {
            assert_eq!(record.sequence, index as u64 + 1);
            assert!(record.data().len() <= CHUNK_LIMIT);
        }
        let next = log.take("step-2", 1);
        assert_eq!(next[0].sequence, 4);
        assert_eq!(next[0].first_line, 201);
    }

    #[test]
    fn fr_ag_08_the_log_ends_with_a_marker_at_the_job_limit() {
        let mut log = JobLog::new("job-1", LOAD);
        let records = log.take("step-1", 20_000);

        let marker = records.last().expect("marker");
        assert_eq!(marker.stream, LogStream::Agent);
        assert_eq!(marker.data(), TRUNCATION_MARKER.as_bytes());
        let bytes: usize = records
            .iter()
            .filter(|record| record.stream == LogStream::Stdout)
            .map(|record| record.data().len())
            .sum();
        assert!(bytes <= LOG_LIMIT);
        assert!(log.take("step-2", 10).is_empty());
    }

    fn ack(job_id: &str, sequence: u64) -> LogAck {
        LogAck {
            job_id: job_id.to_owned(),
            streams: vec![LogStreamAck {
                stream: LogStream::Stdout as i32,
                sequence,
            }],
        }
    }

    #[test]
    fn fr_lg_07_an_acknowledgement_removes_the_covered_chunks() {
        let spool = LogSpool::new(Arc::new(Metrics::default()));
        let mut log = JobLog::new("job-1", LOAD);
        for record in log.take("step-1", 300) {
            spool.keep("job-1", record);
        }
        assert_eq!(spool.len(), 4);

        assert_eq!(spool.acknowledge(&ack("job-1", 2)), 2);
        assert_eq!(spool.acknowledge(&ack("job-2", 9)), 0);
        let resent = spool.greeted(true);
        assert_eq!(
            resent
                .iter()
                .map(|chunk| chunk.sequence)
                .collect::<Vec<_>>(),
            [3, 4]
        );

        assert_eq!(spool.acknowledge(&ack("job-1", 4)), 2);
        assert!(spool.is_empty());
    }

    #[test]
    fn fr_lg_07_without_acknowledgements_the_spool_keeps_nothing() {
        let spool = LogSpool::new(Arc::new(Metrics::default()));
        let mut log = JobLog::new("job-1", LOAD);
        for record in log.take("step-1", 100) {
            spool.keep("job-1", record);
        }

        assert!(spool.greeted(false).is_empty());
        assert!(spool.is_empty());
        for record in log.take("step-1", 100) {
            spool.keep("job-1", record);
        }
        assert!(spool.is_empty());
    }

    #[test]
    fn the_spool_drops_the_oldest_job_above_the_bound() {
        let spool = LogSpool::new(Arc::new(Metrics::default()));
        for index in 0..=MAX_JOBS {
            let job_id = format!("job-{index}");
            let mut log = JobLog::new(&job_id, LOAD);
            for record in log.take("step-1", 1) {
                spool.keep(&job_id, record);
            }
        }

        assert_eq!(spool.len(), MAX_JOBS);
        assert_eq!(spool.acknowledge(&ack("job-0", 1)), 0);
    }
}
