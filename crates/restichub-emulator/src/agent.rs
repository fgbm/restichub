//! Protocol loop of one emulated agent.
//!
//! The agent keeps a permanent stream to the control plane (FR-ID-07). It sends
//! `AgentHello`, answers `Ping` with `Pong`, verifies every job (ADR-0005,
//! ADR-0020), runs the steps and reports the results. A dropped stream
//! reconnects with exponential backoff and full jitter (FR-ID-08). The agent
//! starts no action on its own (ADR-0002).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use restichub_proto::agent::v1::{
    AgentHello, AgentMessage, ErrorDetail, JobAck, JobRejected, JobResult, JobStatus, Pong,
    RejectReason, SignedJob, StepProgress, StepResult, StepStatus, agent_message,
    agent_service_client::AgentServiceClient, control_message, step,
};
use restichub_proto::signing::{
    Admission, Expected, InMemoryJournal, JobJournal, KeyRing, admit, verify_job,
};
use restichub_proto::validate::validate_job;
use tokio::sync::{mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};

use crate::executor::{Executor, StepOutcome};
use crate::identity::AgentIdentity;
use crate::metrics::Metrics;
use crate::now_timestamp;

/// Number of outbound messages buffered for one agent.
const OUT_CHANNEL: usize = 64;

/// Period of the journal purge (ADR-0020).
const JOURNAL_PURGE_SECONDS: u64 = 60;

/// Parameters of the reconnect delay (FR-ID-08).
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    pub base: Duration,
    pub max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            base: Duration::from_secs(1),
            max: Duration::from_secs(60),
        }
    }
}

impl Backoff {
    /// Returns a wait of full jitter between zero and the capped exponential
    /// value (FR-ID-08).
    #[must_use]
    pub fn delay(&self, attempt: u32) -> Duration {
        let shift = attempt.min(6);
        let cap = self.base.saturating_mul(1u32 << shift).min(self.max);
        cap.mul_f64(random_fraction())
    }
}

fn random_fraction() -> f64 {
    crate::random_fraction()
}

/// Configuration of one emulated agent.
pub struct AgentConfig {
    pub tenant_id: String,
    pub server_url: String,
    pub server_name: String,
    pub ca_pem: String,
    pub identity: AgentIdentity,
    pub key_ring: KeyRing,
    pub executor: Arc<dyn Executor>,
    pub backoff: Backoff,
    pub load: AgentLoad,
}

/// Load-profile behaviour of one agent. The default holds no profile: the host
/// OS, no clock offset, no acknowledgement delay and no reconnect storm.
pub struct AgentLoad {
    pub metrics: Arc<Metrics>,
    pub run_start: Instant,
    pub drop_at: Option<Duration>,
    /// Set when the storm fired, so a later reconnect does not drop again.
    pub storm_dropped: AtomicBool,
    /// Offset of the agent clock in microseconds, for job validation.
    pub clock_skew_micros: i64,
    pub ack_delay: Duration,
    /// Label that the agent reports in `AgentHello.os`. `None` uses the host OS.
    pub os_label: Option<String>,
}

impl AgentLoad {
    /// Behaviour with no load profile.
    #[must_use]
    pub fn new(metrics: Arc<Metrics>) -> Self {
        Self {
            metrics,
            run_start: Instant::now(),
            drop_at: None,
            storm_dropped: AtomicBool::new(false),
            clock_skew_micros: 0,
            ack_delay: Duration::ZERO,
            os_label: None,
        }
    }
}

/// Signals that a running job must stop (FR-AG-10).
#[derive(Clone)]
pub struct CancelToken {
    rx: watch::Receiver<bool>,
}

impl CancelToken {
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.rx.borrow()
    }

    /// Waits until the control plane stops the job.
    pub async fn cancelled(&mut self) {
        let _ = self.rx.wait_for(|value| *value).await;
    }
}

struct CancelHandle {
    tx: watch::Sender<bool>,
}

impl CancelHandle {
    fn cancel(&self) {
        let _ = self.tx.send(true);
    }
}

fn cancel_pair() -> (CancelHandle, CancelToken) {
    let (tx, rx) = watch::channel(false);
    (CancelHandle { tx }, CancelToken { rx })
}

/// Reports the progress of one step (FR-AG-08).
pub struct ProgressSink {
    tx: mpsc::Sender<AgentMessage>,
    job_id: String,
    step_id: String,
}

impl ProgressSink {
    /// Sends one progress message over the emulated amount of work.
    pub async fn report(&self, percent_done: f64, seconds_elapsed: f64) {
        const TOTAL_BYTES: u64 = 1_000_000;
        let progress = StepProgress {
            job_id: self.job_id.clone(),
            step_id: self.step_id.clone(),
            bytes_done: (percent_done * TOTAL_BYTES as f64) as u64,
            total_bytes: TOTAL_BYTES,
            percent_done,
            seconds_elapsed,
            ..StepProgress::default()
        };
        let _ = self
            .tx
            .send(AgentMessage {
                payload: Some(agent_message::Payload::StepProgress(progress)),
            })
            .await;
    }
}

#[derive(Clone)]
struct Session {
    tx: mpsc::Sender<AgentMessage>,
    ring: Arc<Mutex<KeyRing>>,
    journal: Arc<Mutex<InMemoryJournal>>,
    jobs: Arc<Mutex<HashMap<String, CancelHandle>>>,
    config: Arc<AgentConfig>,
}

/// Keeps one emulated agent connected until `shutdown` is set. The stream
/// reconnects with exponential backoff and full jitter (FR-ID-08).
pub async fn run_agent(config: Arc<AgentConfig>, mut shutdown: watch::Receiver<bool>) {
    let journal = Arc::new(Mutex::new(InMemoryJournal::default()));
    // The journal drops the entries that passed their expiry (ADR-0020). The
    // task starts no job.
    let purge = tokio::spawn(purge_journal(Arc::clone(&journal), shutdown.clone()));
    let mut attempt = 0u32;
    let mut connected = false;
    while !*shutdown.borrow() {
        if connected {
            config.load.metrics.inc_reconnect();
        }
        match connect_once(&config, &journal, &mut shutdown).await {
            Ok(()) => {
                attempt = 0;
                connected = true;
            }
            Err(error) => {
                // A load generator does not stop on a connection error.
                eprintln!("{}: {error:#}", config.identity.agent_id);
            }
        }
        if *shutdown.borrow() {
            break;
        }
        let delay = config.backoff.delay(attempt);
        attempt = attempt.saturating_add(1);
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
    purge.abort();
}

async fn purge_journal(journal: Arc<Mutex<InMemoryJournal>>, mut shutdown: watch::Receiver<bool>) {
    let mut ticker = tokio::time::interval(Duration::from_secs(JOURNAL_PURGE_SECONDS));
    loop {
        tokio::select! {
            _ = ticker.tick() => crate::lock(&journal).purge_expired(now_timestamp()),
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

async fn connect_once(
    config: &Arc<AgentConfig>,
    journal: &Arc<Mutex<InMemoryJournal>>,
    shutdown: &mut watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(&config.ca_pem))
        .identity(Identity::from_pem(
            &config.identity.cert_pem,
            &config.identity.key_pem,
        ))
        .domain_name(config.server_name.clone());
    let endpoint = Endpoint::from_shared(config.server_url.clone())
        .context("invalid server URL")?
        .tls_config(tls)?;
    let started = Instant::now();
    let channel = endpoint.connect().await?;
    let mut client = AgentServiceClient::new(channel);

    let (tx, rx) = mpsc::channel(OUT_CHANNEL);
    let response = client.connect(ReceiverStream::new(rx)).await?;
    let mut inbound = response.into_inner();

    send_hello(&tx, config.load.os_label.as_deref()).await?;

    let metrics = Arc::clone(&config.load.metrics);
    metrics.record_connect(started.elapsed().as_micros() as u64);

    let session = Session {
        tx: tx.clone(),
        ring: Arc::new(Mutex::new(config.key_ring.clone())),
        journal: Arc::clone(journal),
        jobs: Arc::new(Mutex::new(HashMap::new())),
        config: Arc::clone(config),
    };

    while !*shutdown.borrow() {
        let storm = storm_drop(config);
        let message = tokio::select! {
            message = inbound.message() => message?,
            () = storm => break,
            result = shutdown.changed() => {
                if result.is_err() || *shutdown.borrow() {
                    break;
                }
                continue;
            }
        };
        let Some(message) = message else {
            break;
        };
        match message.payload {
            Some(control_message::Payload::Ping(ping)) => {
                let pong = AgentMessage {
                    payload: Some(agent_message::Payload::Pong(Pong { nonce: ping.nonce })),
                };
                let _ = tx.send(pong).await;
            }
            Some(control_message::Payload::SignedJob(signed)) => {
                let (handle, token) = cancel_pair();
                tokio::spawn(run_job(session.clone(), signed, handle, token));
            }
            Some(control_message::Payload::CancelJob(cancel)) => {
                if let Some(handle) = crate::lock(&session.jobs).remove(&cancel.job_id) {
                    handle.cancel();
                }
            }
            Some(control_message::Payload::SigningKeyUpdate(update)) => {
                let mut ring = crate::lock(&session.ring);
                if let Err(error) = ring.apply_key_update(&update) {
                    eprintln!(
                        "{}: key update refused: {error:?}",
                        config.identity.agent_id
                    );
                }
            }
            // Certificate rotation needs the enrollment of epic #1.
            Some(
                control_message::Payload::CertificateIssued(_)
                | control_message::Payload::ServerHello(_),
            )
            | None => {}
        }
    }

    cancel_running(&session);
    metrics.stream_closed();
    Ok(())
}

/// Waits for the reconnect storm of the profile, then drops the stream once
/// (FR-ID-08). Without a storm, or after the drop, it never completes.
///
/// The deadline is absolute, so a control message between the connect and the
/// storm does not cancel it.
async fn storm_drop(config: &AgentConfig) {
    let Some(deadline) = config.load.drop_at.map(|at| config.load.run_start + at) else {
        std::future::pending::<()>().await;
        return;
    };
    if config.load.storm_dropped.load(Ordering::Relaxed) {
        std::future::pending::<()>().await;
        return;
    }
    tokio::time::sleep(deadline.saturating_duration_since(Instant::now())).await;
    config.load.storm_dropped.store(true, Ordering::Relaxed);
}

async fn send_hello(tx: &mpsc::Sender<AgentMessage>, os_label: Option<&str>) -> anyhow::Result<()> {
    let hello = AgentMessage {
        payload: Some(agent_message::Payload::AgentHello(AgentHello {
            protocol_version_major: restichub_proto::PROTOCOL_VERSION_MAJOR,
            protocol_version_minor: restichub_proto::PROTOCOL_VERSION_MINOR,
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
            restic_version: "0.19.1".to_owned(),
            os: os_label.unwrap_or(std::env::consts::OS).to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            supported_step_types: supported_step_types(),
            ..AgentHello::default()
        })),
    };
    tx.send(hello).await.context("cannot send AgentHello")?;
    Ok(())
}

fn supported_step_types() -> Vec<String> {
    [
        "backup",
        "check",
        "snapshots",
        "ls",
        "restore",
        "unlock",
        "forget",
        "prune",
        "stats",
    ]
    .iter()
    .map(|name| (*name).to_owned())
    .collect()
}

fn cancel_running(session: &Session) {
    let handles: Vec<CancelHandle> = crate::lock(&session.jobs)
        .drain()
        .map(|(_, handle)| handle)
        .collect();
    for handle in handles {
        handle.cancel();
    }
}

async fn run_job(
    session: Session,
    signed: SignedJob,
    handle: CancelHandle,
    mut token: CancelToken,
) {
    let received = Instant::now();
    let expected = Expected {
        agent_id: session.config.identity.agent_id.clone(),
        tenant_id: session.config.tenant_id.clone(),
        now: skewed_timestamp(now_timestamp(), session.config.load.clock_skew_micros),
    };
    let ring = crate::lock(&session.ring).clone();
    let job = match verify_job(&signed, &ring, &expected) {
        Ok(job) => job,
        Err(rejection) => {
            session.config.load.metrics.inc_rejected();
            send_rejected(&session.tx, rejection.job_id, rejection.reason).await;
            return;
        }
    };
    if let Err(rejection) = validate_job(&job) {
        session.config.load.metrics.inc_rejected();
        send_rejected(&session.tx, rejection.job_id, rejection.reason).await;
        return;
    }

    let admission = {
        let mut journal = crate::lock(&session.journal);
        admit(&job, &mut *journal, now_timestamp())
    };
    match admission {
        Admission::AlreadyCompleted(result) => {
            let _ = session.tx.send(job_result_message(result)).await;
            return;
        }
        Admission::New | Admission::InProgress => {}
    }

    let job_id = job.job_id.clone();
    crate::lock(&session.jobs).insert(job_id.clone(), handle);

    // A slow agent holds the acknowledgement for the profile delay.
    if !session.config.load.ack_delay.is_zero() {
        tokio::time::sleep(session.config.load.ack_delay).await;
    }
    let ack = AgentMessage {
        payload: Some(agent_message::Payload::JobAck(JobAck {
            job_id: job_id.clone(),
        })),
    };
    if session.tx.send(ack).await.is_err() {
        crate::lock(&session.jobs).remove(&job_id);
        return;
    }
    session
        .config
        .load
        .metrics
        .record_ack(received.elapsed().as_micros() as u64);
    let acked = Instant::now();

    let steps_total = u32::try_from(job.steps.len()).unwrap_or(u32::MAX);
    let had_init = job
        .steps
        .iter()
        .any(|step| matches!(step.kind, Some(step::Kind::Init(_))));
    let mut succeeded = 0u32;
    let mut failed = 0u32;
    let mut cancelled = false;
    // The exit code of a step that stopped the job outside the missing-repository probe.
    let mut stopped_code: Option<i32> = None;
    // Set when a test_access step reported that the repository does not exist (restic exit code
    // 10). Only then does the init step run (FR-RP-02, AC-RP-01).
    let mut probe_missing = false;
    let mut init_succeeded = false;

    for step in &job.steps {
        let is_test_access = matches!(step.kind, Some(step::Kind::TestAccess(_)));
        let is_init = matches!(step.kind, Some(step::Kind::Init(_)));
        // The agent runs init only after test_access reported a missing repository. Any other
        // result of test_access skips it, so an existing repository is never re-initialized.
        if is_init && !probe_missing {
            let now = now_timestamp();
            send_step_result(
                &session.tx,
                &job_id,
                &step.step_id,
                StepOutcome {
                    status: StepStatus::Skipped,
                    exit_code: -1,
                },
                now,
                now,
            )
            .await;
            continue;
        }
        let started_at = now_timestamp();
        let sink = ProgressSink {
            tx: session.tx.clone(),
            job_id: job_id.clone(),
            step_id: step.step_id.clone(),
        };
        let outcome = tokio::select! {
            outcome = session.config.executor.run_step(step, &sink) => outcome,
            () = token.cancelled() => {
                cancelled = true;
                StepOutcome {
                    status: StepStatus::Cancelled,
                    exit_code: -1,
                }
            }
        };
        let finished_at = now_timestamp();
        send_step_result(
            &session.tx,
            &job_id,
            &step.step_id,
            outcome,
            started_at,
            finished_at,
        )
        .await;
        match outcome.status {
            StepStatus::Success => {
                succeeded += 1;
                if is_init {
                    init_succeeded = true;
                }
            }
            StepStatus::Cancelled => {
                cancelled = true;
                break;
            }
            _ => {
                failed += 1;
                if is_test_access && outcome.exit_code == 10 {
                    // The repository does not exist: the init step may run.
                    probe_missing = true;
                } else {
                    // Every other failure stops the job with its own outcome: no init runs
                    // (FR-RP-02, AC-RP-01).
                    stopped_code = Some(outcome.exit_code);
                    break;
                }
            }
        }
    }

    let outcome = job_outcome(
        cancelled,
        stopped_code,
        probe_missing,
        had_init,
        init_succeeded,
    );
    let status = match outcome {
        JobStatus::Succeeded => StepStatus::Success,
        JobStatus::Cancelled => StepStatus::Cancelled,
        JobStatus::ConfigError => StepStatus::ConfigError,
        JobStatus::Failed | JobStatus::NotInitialized | JobStatus::Unspecified => {
            StepStatus::Failed
        }
    };
    let result = JobResult {
        job_id: job_id.clone(),
        status: status as i32,
        steps_total,
        steps_succeeded: succeeded,
        steps_failed: failed,
        outcome: outcome as i32,
        error: job_error(outcome),
        steps: Vec::new(),
        log_tail: Vec::new(),
    };
    crate::lock(&session.journal).record_completed(&job_id, result.clone());
    let _ = session.tx.send(job_result_message(result)).await;
    session
        .config
        .load
        .metrics
        .record_result(acked.elapsed().as_micros() as u64);
    crate::lock(&session.jobs).remove(&job_id);
}

/// Adds a clock offset in microseconds to a timestamp.
fn skewed_timestamp(base: prost_types::Timestamp, skew_micros: i64) -> prost_types::Timestamp {
    if skew_micros == 0 {
        return base;
    }
    let total = base.seconds as i128 * 1_000_000 + base.nanos as i128 / 1_000 + skew_micros as i128;
    prost_types::Timestamp {
        seconds: total.div_euclid(1_000_000) as i64,
        nanos: (total.rem_euclid(1_000_000) * 1_000) as i32,
    }
}

async fn send_rejected(
    tx: &mpsc::Sender<AgentMessage>,
    job_id: Option<String>,
    reason: RejectReason,
) {
    let message = AgentMessage {
        payload: Some(agent_message::Payload::JobRejected(JobRejected {
            job_id: job_id.unwrap_or_default(),
            reason: reason as i32,
        })),
    };
    let _ = tx.send(message).await;
}

async fn send_step_result(
    tx: &mpsc::Sender<AgentMessage>,
    job_id: &str,
    step_id: &str,
    outcome: StepOutcome,
    started_at: prost_types::Timestamp,
    finished_at: prost_types::Timestamp,
) {
    let result = StepResult {
        job_id: job_id.to_owned(),
        step_id: step_id.to_owned(),
        status: outcome.status as i32,
        exit_code: outcome.exit_code,
        started_at: Some(started_at),
        finished_at: Some(finished_at),
        possible_stale_lock: outcome.status == StepStatus::Cancelled,
        ..StepResult::default()
    };
    let _ = tx
        .send(AgentMessage {
            payload: Some(agent_message::Payload::StepResult(result)),
        })
        .await;
}

fn job_result_message(result: JobResult) -> AgentMessage {
    AgentMessage {
        payload: Some(agent_message::Payload::JobResult(result)),
    }
}

/// Maps the step results to the typed outcome of a job (FR-AG-09, AC-RP-01).
///
/// The init chain: the agent runs init only after test_access exited with code 10 (a missing
/// repository). A successful init after that probe is a successful job. A test_access that failed
/// with any other code (1, 11, 12, a timeout, access denied) stops the job, and a standalone
/// test_access with exit code 10 is the normal `NOT_INITIALIZED` result.
fn job_outcome(
    cancelled: bool,
    stopped_code: Option<i32>,
    probe_missing: bool,
    had_init: bool,
    init_succeeded: bool,
) -> JobStatus {
    if cancelled {
        return JobStatus::Cancelled;
    }
    if let Some(code) = stopped_code {
        // restic exit code 12 is a wrong password or wrong credentials (AC-RP-01).
        return if code == 12 {
            JobStatus::ConfigError
        } else {
            JobStatus::Failed
        };
    }
    if probe_missing {
        if had_init {
            return if init_succeeded {
                JobStatus::Succeeded
            } else {
                JobStatus::Failed
            };
        }
        return JobStatus::NotInitialized;
    }
    JobStatus::Succeeded
}

/// The stable code that explains a failed or cancelled outcome (NFR-DOC-04). It holds no secret.
fn job_error(outcome: JobStatus) -> Option<ErrorDetail> {
    let code = match outcome {
        JobStatus::ConfigError => "wrong_password",
        JobStatus::NotInitialized => "repository_missing",
        JobStatus::Failed => "step_failed",
        JobStatus::Cancelled => "cancelled",
        JobStatus::Succeeded | JobStatus::Unspecified => return None,
    };
    Some(ErrorDetail {
        code: code.to_owned(),
        params: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::{job_error, job_outcome};
    use restichub_proto::agent::v1::JobStatus;

    /// AC-RP-01: the init chain runs init only after test_access exit code 10.
    #[test]
    fn ac_rp_01_maps_the_init_chain_to_a_job_outcome() {
        // A repository that exists: test_access exit 0, init skipped.
        assert_eq!(
            job_outcome(false, None, false, true, false),
            JobStatus::Succeeded
        );
        // A missing repository with a successful init.
        assert_eq!(
            job_outcome(false, None, true, true, true),
            JobStatus::Succeeded
        );
        // A standalone test_access on a missing repository.
        assert_eq!(
            job_outcome(false, None, true, false, false),
            JobStatus::NotInitialized
        );
        // A wrong password on an existing repository (exit code 12).
        assert_eq!(
            job_outcome(false, Some(12), false, true, false),
            JobStatus::ConfigError
        );
        // Any other failure (exit codes 1, 11, a timeout).
        assert_eq!(
            job_outcome(false, Some(1), false, true, false),
            JobStatus::Failed
        );
        assert_eq!(
            job_outcome(false, Some(11), false, true, false),
            JobStatus::Failed
        );
        // The control plane cancelled the job before any step finished.
        assert_eq!(
            job_outcome(true, None, false, true, false),
            JobStatus::Cancelled
        );
        // The error code that a wrong password reports.
        assert_eq!(
            job_error(JobStatus::ConfigError).map(|error| error.code),
            Some("wrong_password".to_owned())
        );
        assert_eq!(
            job_error(JobStatus::NotInitialized).map(|error| error.code),
            Some("repository_missing".to_owned())
        );
    }
}
