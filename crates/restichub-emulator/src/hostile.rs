//! Hostile agent modes for the scenario of a hostile agent (AC-PR-05).
//!
//! Only the emulator has them. A hostile agent sends job results for jobs that it never got, a
//! flood of progress messages for unknown jobs, and one message at or above the protocol size
//! limit. The control plane must refuse each of them and keep serving the other agents. A message
//! of an agent holds no tenant field: the control plane takes the tenant from the certificate, so
//! the protocol leaves no wrong tenant to send.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use prost::Message as _;
use restichub_proto::agent::v1::{
    AgentMessage, JobResult, JobStatus, LogChunk, LogStream, StepProgress, StepStatus,
    agent_message,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::agent::AgentConfig;
use crate::metrics::Metrics;
use crate::profile::{Hostile, Oversize};
use crate::{lock, random_fraction};

/// Job identifiers that the pool of one process remembers.
const SEEN_LIMIT: usize = 1024;

const FLOOD_TICK: Duration = Duration::from_millis(10);

/// The modes of one hostile agent.
#[derive(Debug)]
pub struct HostileLoad {
    pub forged_result_share: f64,
    pub flood_per_second: f64,
    pub oversize: Option<Oversize>,
    oversize_sent: AtomicBool,
}

impl HostileLoad {
    #[must_use]
    pub fn new(hostile: &Hostile) -> Self {
        Self {
            forged_result_share: hostile.forged_result_share,
            flood_per_second: hostile.flood_per_second,
            oversize: hostile.oversize,
            oversize_sent: AtomicBool::new(false),
        }
    }
}

/// The jobs that the agents of this process got, so that a hostile agent can name a real job of
/// another agent.
#[derive(Debug, Default)]
pub struct SeenJobs {
    jobs: std::sync::Mutex<VecDeque<(String, String)>>,
}

impl SeenJobs {
    pub fn add(&self, agent_id: &str, job_id: &str) {
        let mut jobs = lock(&self.jobs);
        if jobs.len() >= SEEN_LIMIT {
            jobs.pop_front();
        }
        jobs.push_back((agent_id.to_owned(), job_id.to_owned()));
    }

    /// A job of an agent other than `agent_id`, picked at random.
    #[must_use]
    pub fn foreign(&self, agent_id: &str, fraction: f64) -> Option<String> {
        let jobs = lock(&self.jobs);
        let foreign: Vec<&String> = jobs
            .iter()
            .filter(|(owner, _)| owner != agent_id)
            .map(|(_, job)| job)
            .collect();
        if foreign.is_empty() {
            return None;
        }
        let index = ((fraction * foreign.len() as f64) as usize).min(foreign.len() - 1);
        Some(foreign[index].clone())
    }
}

/// A random version 4 UUID, the form of a job identifier of the control plane.
#[must_use]
pub fn random_uuid() -> String {
    let mut bytes = [0u8; 16];
    // getrandom does not fail on a supported host; zero bytes still give a valid form.
    let _ = getrandom::fill(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// After a real job result, sends at the profile share a successful result for a job that the
/// agent never got: a job of another agent when the pool knows one, else a job that does not
/// exist (AC-PR-05).
pub async fn after_result(config: &AgentConfig, tx: &mpsc::Sender<AgentMessage>) {
    let Some(hostile) = &config.load.hostile else {
        return;
    };
    if random_fraction() >= hostile.forged_result_share {
        return;
    }
    let foreign = config
        .load
        .seen_jobs
        .as_ref()
        .and_then(|seen| seen.foreign(&config.identity.agent_id, random_fraction()));
    config.load.metrics.forged_result(foreign.is_some());
    let result = JobResult {
        job_id: foreign.unwrap_or_else(random_uuid),
        status: StepStatus::Success as i32,
        steps_total: 1,
        steps_succeeded: 1,
        outcome: JobStatus::Succeeded as i32,
        ..JobResult::default()
    };
    let _ = tx
        .send(AgentMessage {
            payload: Some(agent_message::Payload::JobResult(result)),
        })
        .await;
}

/// Starts the flood and the oversize message of a hostile agent on a new stream. The caller
/// aborts the task when the stream ends.
#[must_use]
pub fn start(config: &Arc<AgentConfig>, tx: &mpsc::Sender<AgentMessage>) -> Option<JoinHandle<()>> {
    let hostile = config.load.hostile.as_ref()?;
    let oversize = hostile
        .oversize
        .filter(|_| !hostile.oversize_sent.swap(true, Ordering::Relaxed));
    if oversize.is_none() && hostile.flood_per_second <= 0.0 {
        return None;
    }
    let per_second = hostile.flood_per_second;
    let metrics = Arc::clone(&config.load.metrics);
    let tx = tx.clone();
    Some(tokio::spawn(async move {
        if let Some(kind) = oversize {
            if tx.send(oversize_message(kind)).await.is_err() {
                return;
            }
            metrics.oversize_message();
        }
        if per_second > 0.0 {
            flood(&tx, per_second, &metrics).await;
        }
    }))
}

async fn flood(tx: &mpsc::Sender<AgentMessage>, per_second: f64, metrics: &Metrics) {
    let progress = AgentMessage {
        payload: Some(agent_message::Payload::StepProgress(StepProgress {
            job_id: random_uuid(),
            step_id: "step-1".to_owned(),
            percent_done: 0.5,
            ..StepProgress::default()
        })),
    };
    let started = Instant::now();
    let mut sent = 0u64;
    let mut ticker = tokio::time::interval(FLOOD_TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let due = (per_second * started.elapsed().as_secs_f64()) as u64;
        while sent < due {
            if tx.send(progress.clone()).await.is_err() {
                return;
            }
            metrics.flood_message();
            sent += 1;
        }
    }
}

/// A log chunk for an unknown job whose encoded message is exactly at, or one byte above,
/// [`restichub_proto::MAX_MESSAGE_BYTES`].
#[must_use]
pub fn oversize_message(kind: Oversize) -> AgentMessage {
    let target = match kind {
        Oversize::AtLimit => restichub_proto::MAX_MESSAGE_BYTES,
        Oversize::AboveLimit => restichub_proto::MAX_MESSAGE_BYTES + 1,
    };
    let mut chunk = LogChunk {
        job_id: random_uuid(),
        step_id: "step-1".to_owned(),
        stream: LogStream::Stdout as i32,
        sequence: 1,
        data: Vec::new(),
    };
    let mut size = target;
    // The length prefixes grow with the data, so the size settles after a few rounds.
    for _ in 0..8 {
        chunk.data = vec![b'x'; size];
        let message = AgentMessage {
            payload: Some(agent_message::Payload::LogChunk(chunk.clone())),
        };
        let encoded = message.encoded_len();
        if encoded == target {
            return message;
        }
        size = (size + target).saturating_sub(encoded);
    }
    AgentMessage {
        payload: Some(agent_message::Payload::LogChunk(chunk)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_random_uuid_has_the_version_4_form() {
        let uuid = random_uuid();

        assert_eq!(uuid.len(), 36);
        assert_eq!(uuid.as_bytes()[14], b'4');
        assert!(matches!(uuid.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
        assert_ne!(uuid, random_uuid());
    }

    #[test]
    fn ac_pr_05_a_forged_result_names_a_job_of_another_agent() {
        let seen = SeenJobs::default();
        assert_eq!(seen.foreign("agent-a", 0.5), None);

        seen.add("agent-a", "job-a");
        assert_eq!(seen.foreign("agent-a", 0.5), None);
        seen.add("agent-b", "job-b");
        assert_eq!(seen.foreign("agent-a", 0.99), Some("job-b".to_owned()));
        assert_eq!(seen.foreign("agent-b", 0.0), Some("job-a".to_owned()));
    }

    #[test]
    fn the_pool_keeps_the_newest_jobs() {
        let seen = SeenJobs::default();
        for index in 0..=SEEN_LIMIT {
            seen.add("agent-b", &format!("job-{index}"));
        }

        assert_eq!(seen.foreign("agent-a", 0.0), Some("job-1".to_owned()));
    }

    #[test]
    fn the_oversize_message_is_at_or_one_byte_above_the_limit() {
        assert_eq!(
            oversize_message(Oversize::AtLimit).encoded_len(),
            restichub_proto::MAX_MESSAGE_BYTES
        );
        assert_eq!(
            oversize_message(Oversize::AboveLimit).encoded_len(),
            restichub_proto::MAX_MESSAGE_BYTES + 1
        );
    }
}
