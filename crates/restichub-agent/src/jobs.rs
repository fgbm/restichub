//! The agent-side execution of the repository init and test-access jobs (FR-PR-03, FR-PR-06,
//! FR-RP-02, ADR-0020, ADR-0026).
//!
//! The control plane delivers a signed job on the stream. The agent verifies the signature with the
//! key ring and checks the addressee, the tenant and the time rules ([`verify_job`]), validates the
//! steps ([`ValidatedJob`], FR-PR-01) and consults the job journal (ADR-0020). It then acknowledges
//! the job (FR-PR-06) and runs it off the stream task, so the heartbeat and the key updates keep
//! flowing. A job that the journal already completed is not run again; the stored result is re-sent.
//!
//! This step of the epic runs `test_access` and `init` only. Any other step kind is refused with
//! [`RejectReason::UnsupportedStep`]. The agent starts no job on its own and holds no timer that
//! starts one (ADR-0002).
//!
//! The restic command line comes from the typed step through the compiler builders (ADR-0021), so no
//! value of a job becomes an option. The repository secrets reach restic only through the child
//! environment (FR-AG-06); they never reach an argument, a log, the journal or an error. The agent
//! runs `init` only after `test_access` exited with restic code 10 (the repository does not exist);
//! any other result of `test_access` stops the job with its typed outcome (AC-RP-01).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::Result;
use prost_types::Timestamp;
use restichub_compiler::ValidatedJob;
use restichub_compiler::arg::build_command;
use restichub_proto::agent::v1::{
    AgentMessage, ErrorDetail, JobAck, JobRejected, JobResult, JobStatus, RejectReason, SignedJob,
    Step, StepResult, StepStatus, agent_message, step,
};
use restichub_proto::signing::{Admission, Expected, JobJournal as _, KeyRing, admit, verify_job};
use tokio::sync::mpsc;

use crate::journal::{FileJournal, JOURNAL_FILE};

/// How often the agent checks the restic child for its exit.
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// The decision of one signed job after the checks (FR-PR-03, ADR-0020).
pub enum Intake {
    /// Refused with a typed reason; no process starts.
    Rejected {
        job_id: String,
        reason: RejectReason,
    },
    /// The journal holds the result of the job; the agent re-sends it and does not run the job.
    Completed(Box<JobResult>),
    /// The job is new, or an earlier run did not finish; the agent may run it.
    Accepted(Box<ValidatedJob>),
}

/// The running jobs of one agent and the journal that records them (ADR-0020).
#[derive(Clone)]
pub struct Jobs {
    program: PathBuf,
    expected_sha256: String,
    cache: PathBuf,
    journal: Arc<Mutex<FileJournal>>,
    running: Arc<Mutex<HashMap<String, CancelHandle>>>,
}

impl Jobs {
    /// Loads the journal from the configuration directory and prunes it (ADR-0020).
    ///
    /// # Errors
    ///
    /// Returns an error when the journal cannot be read.
    pub fn open(
        program: PathBuf,
        expected_sha256: String,
        cache: PathBuf,
        dir: &Path,
    ) -> Result<Self> {
        let mut journal = FileJournal::load(dir.join(JOURNAL_FILE))?;
        journal.prune(now_timestamp());
        journal.persist()?;
        Ok(Self {
            program,
            expected_sha256,
            cache,
            journal: Arc::new(Mutex::new(journal)),
            running: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Verifies, admits and dispatches one signed job (FR-PR-03, FR-PR-06, ADR-0020).
    ///
    /// A refusal and a replayed job are answered at once. An accepted job is acknowledged and runs
    /// off the stream task, so this call returns without waiting for it.
    pub async fn handle(
        &self,
        signed: SignedJob,
        ring: &KeyRing,
        agent_id: &str,
        tenant_id: &str,
        tx: &mpsc::Sender<AgentMessage>,
    ) {
        let expected = Expected {
            agent_id: agent_id.to_owned(),
            tenant_id: tenant_id.to_owned(),
            now: now_timestamp(),
        };
        let decision = {
            let mut journal = lock(&self.journal);
            let decision = intake(&signed, ring, &expected, &mut journal);
            if let Err(error) = journal.persist() {
                tracing::warn!(%error, "cannot write the job journal");
            }
            decision
        };
        match decision {
            Intake::Rejected { job_id, reason } => {
                let _ = tx.send(rejected(job_id, reason)).await;
            }
            Intake::Completed(result) => {
                let job_id = result.job_id.clone();
                if tx.send(job_result_message(*result)).await.is_ok() {
                    let mut journal = lock(&self.journal);
                    journal.mark_delivered(&job_id);
                    if let Err(error) = journal.persist() {
                        tracing::warn!(%error, "cannot write the job journal");
                    }
                }
            }
            Intake::Accepted(validated) => {
                let job_id = validated.job().job_id.clone();
                let ack = AgentMessage {
                    payload: Some(agent_message::Payload::JobAck(JobAck {
                        job_id: job_id.clone(),
                    })),
                };
                if tx.send(ack).await.is_err() {
                    // The stream dropped after the admission; the control plane delivers the job
                    // again and the journal answers that replay (ADR-0020).
                    return;
                }
                let cancel = CancelHandle::new();
                lock(&self.running).insert(job_id, cancel.clone());
                let jobs = self.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    jobs.execute(*validated, tx, cancel).await;
                });
            }
        }
    }

    /// Stops the restic child of the running job (FR-AG-10).
    pub fn cancel(&self, job_id: &str) {
        if let Some(handle) = lock(&self.running).get(job_id) {
            handle.cancel();
        }
    }

    /// Re-sends every stored result that the agent has not sent yet (ADR-0020).
    ///
    /// The agent calls it at every connection, so a result that a dropped stream lost reaches the
    /// control plane.
    pub async fn flush_pending(&self, tx: &mpsc::Sender<AgentMessage>) {
        let pending = lock(&self.journal).pending_results();
        for result in pending {
            if tx.send(job_result_message(result.clone())).await.is_err() {
                return;
            }
            let mut journal = lock(&self.journal);
            journal.mark_delivered(&result.job_id);
            if let Err(error) = journal.persist() {
                tracing::warn!(%error, "cannot write the job journal");
            }
        }
    }

    /// Runs the steps of one job and reports the typed result (FR-AG-09, AC-RP-01).
    async fn execute(
        &self,
        validated: ValidatedJob,
        tx: mpsc::Sender<AgentMessage>,
        cancel: CancelHandle,
    ) {
        let job = validated.job();
        let job_id = job.job_id.clone();
        let steps_total = u32::try_from(job.steps.len()).unwrap_or(u32::MAX);
        let had_init = job
            .steps
            .iter()
            .any(|step| matches!(step.kind, Some(step::Kind::Init(_))));
        let mut succeeded = 0_u32;
        let mut failed = 0_u32;
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
                    &tx,
                    &job_id,
                    &step.step_id,
                    StepStatus::Skipped,
                    -1,
                    now,
                    now,
                )
                .await;
                continue;
            }
            let started_at = now_timestamp();
            let outcome = self.run_step(&validated, step, &cancel).await;
            let finished_at = now_timestamp();
            send_step_result(
                &tx,
                &job_id,
                &step.step_id,
                outcome.status,
                outcome.exit_code,
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

        lock(&self.running).remove(&job_id);
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
        };
        // The result is durable before the agent tries to send it, so a dropped stream cannot lose it
        // (ADR-0020).
        {
            let mut journal = lock(&self.journal);
            journal.record_completed(&job_id, result.clone());
            if let Err(error) = journal.persist() {
                tracing::warn!(%error, "cannot write the job journal");
            }
        }
        if tx.send(job_result_message(result)).await.is_ok() {
            let mut journal = lock(&self.journal);
            journal.mark_delivered(&job_id);
            if let Err(error) = journal.persist() {
                tracing::warn!(%error, "cannot write the job journal");
            }
        }
    }

    /// Runs one step with the bundled restic and waits for it (FR-AG-05, FR-AG-06, FR-AG-09).
    async fn run_step(
        &self,
        validated: &ValidatedJob,
        step: &Step,
        cancel: &CancelHandle,
    ) -> StepOutcome {
        let line = match build_command(validated, step) {
            Ok(line) => line,
            Err(error) => {
                tracing::warn!(%error, "cannot build the restic command");
                return StepOutcome::failed();
            }
        };
        // The hash check and the start share one opening of the binary (FR-AG-11, threat T20). The
        // standard streams are detached: a restic message can carry the repository location, so the
        // agent reports the exit code only (threat T9).
        let child = match crate::exec::spawn_job_restic(
            &self.program,
            &self.expected_sha256,
            &line,
            &self.cache,
        ) {
            Ok(child) => child,
            Err(error) => {
                tracing::warn!(%error, "cannot start restic");
                return StepOutcome::failed();
            }
        };
        *lock(&cancel.child) = Some(child);
        let deadline = Duration::from_secs(u64::from(step.timeout_seconds));
        let started = tokio::time::Instant::now();
        let mut timed_out = false;
        loop {
            let reaped = {
                let mut slot = lock(&cancel.child);
                match slot.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(Some(status)) => {
                            let code = status.code().unwrap_or(-1);
                            *slot = None;
                            Some(code)
                        }
                        Ok(None) => None,
                        Err(error) => {
                            tracing::warn!(%error, "cannot wait for restic");
                            *slot = None;
                            return StepOutcome::failed();
                        }
                    },
                    None => None,
                }
            };
            if let Some(code) = reaped {
                if cancel.cancelled.load(Ordering::SeqCst) {
                    return StepOutcome::cancelled();
                }
                if timed_out {
                    return StepOutcome::failed();
                }
                return StepOutcome {
                    status: status_for_exit(code),
                    exit_code: code,
                };
            }
            if !timed_out && !deadline.is_zero() && started.elapsed() >= deadline {
                timed_out = true;
                if let Some(child) = lock(&cancel.child).as_mut() {
                    let _ = child.kill();
                }
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

/// Verifies, validates and admits one signed job (FR-PR-03, FR-PR-01, ADR-0020).
///
/// The function is separate from [`Jobs::handle`] so a test can drive the checks without a restic
/// child.
pub fn intake(
    signed: &SignedJob,
    ring: &KeyRing,
    expected: &Expected,
    journal: &mut FileJournal,
) -> Intake {
    let job = match verify_job(signed, ring, expected) {
        Ok(job) => job,
        Err(rejection) => {
            return Intake::Rejected {
                job_id: rejection.job_id.unwrap_or_default(),
                reason: rejection.reason,
            };
        }
    };
    let job_id = job.job_id.clone();
    let validated = match ValidatedJob::new(job) {
        Ok(validated) => validated,
        Err(rejection) => {
            return Intake::Rejected {
                job_id,
                reason: rejection.reason,
            };
        }
    };
    if validated
        .job()
        .steps
        .iter()
        .any(|step| !step.kind.as_ref().is_some_and(is_supported))
    {
        return Intake::Rejected {
            job_id,
            reason: RejectReason::UnsupportedStep,
        };
    }
    match admit(validated.job(), journal, expected.now) {
        Admission::AlreadyCompleted(result) => Intake::Completed(Box::new(result)),
        Admission::New | Admission::InProgress => Intake::Accepted(Box::new(validated)),
    }
}

/// The step kinds that this step of the epic runs (FR-PR-02).
fn is_supported(kind: &step::Kind) -> bool {
    matches!(kind, step::Kind::Init(_) | step::Kind::TestAccess(_))
}

/// Exit status and code of one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StepOutcome {
    status: StepStatus,
    exit_code: i32,
}

impl StepOutcome {
    fn failed() -> Self {
        Self {
            status: StepStatus::Failed,
            exit_code: -1,
        }
    }

    fn cancelled() -> Self {
        Self {
            status: StepStatus::Cancelled,
            exit_code: -1,
        }
    }
}

/// The handle that stops the restic child of a running job (FR-AG-10).
#[derive(Clone)]
struct CancelHandle {
    child: Arc<Mutex<Option<Child>>>,
    cancelled: Arc<AtomicBool>,
}

impl CancelHandle {
    fn new() -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(child) = lock(&self.child).as_mut() {
            let _ = child.kill();
        }
    }
}

/// Maps a restic exit code to a step status (FR-AG-09, AC-AG-05).
#[must_use]
fn status_for_exit(code: i32) -> StepStatus {
    match code {
        0 => StepStatus::Success,
        3 => StepStatus::Warning,
        11 => StepStatus::Retry,
        10 | 12 => StepStatus::ConfigError,
        130 => StepStatus::Cancelled,
        _ => StepStatus::Failed,
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

/// Reads the wall clock as a protocol timestamp.
fn now_timestamp() -> Timestamp {
    let now = time::OffsetDateTime::now_utc();
    Timestamp {
        seconds: now.unix_timestamp(),
        nanos: now.nanosecond() as i32,
    }
}

fn rejected(job_id: String, reason: RejectReason) -> AgentMessage {
    AgentMessage {
        payload: Some(agent_message::Payload::JobRejected(JobRejected {
            job_id,
            reason: reason as i32,
        })),
    }
}

fn job_result_message(result: JobResult) -> AgentMessage {
    AgentMessage {
        payload: Some(agent_message::Payload::JobResult(result)),
    }
}

async fn send_step_result(
    tx: &mpsc::Sender<AgentMessage>,
    job_id: &str,
    step_id: &str,
    status: StepStatus,
    exit_code: i32,
    started_at: Timestamp,
    finished_at: Timestamp,
) {
    let result = StepResult {
        job_id: job_id.to_owned(),
        step_id: step_id.to_owned(),
        status: status as i32,
        exit_code,
        started_at: Some(started_at),
        finished_at: Some(finished_at),
        possible_stale_lock: status == StepStatus::Cancelled,
        ..StepResult::default()
    };
    let _ = tx
        .send(AgentMessage {
            payload: Some(agent_message::Payload::StepResult(result)),
        })
        .await;
}

/// Locks a mutex, ignoring a panic that poisoned it: the journal stays usable.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use ed25519_dalek::SigningKey;
    use restichub_proto::agent::v1::{
        BackupStep, InitStep, Job, RepositoryAccess, Step, TestAccessStep, step,
    };
    use restichub_proto::signing::{Expected, KeyRing, sign_job};

    use super::{Intake, intake, job_error, job_outcome, status_for_exit};
    use crate::journal::FileJournal;
    use restichub_proto::agent::v1::{JobStatus, RejectReason, StepStatus};

    const KEY_ID: &str = "job-key-1";

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7_u8; 32])
    }

    fn ring() -> KeyRing {
        let mut ring = KeyRing::new();
        ring.insert(KEY_ID, signing_key().verifying_key())
            .expect("insert");
        ring
    }

    fn at(seconds: i64) -> prost_types::Timestamp {
        prost_types::Timestamp { seconds, nanos: 0 }
    }

    fn expected() -> Expected {
        Expected {
            agent_id: "agent-1".to_owned(),
            tenant_id: "tenant-1".to_owned(),
            now: at(1_000),
        }
    }

    fn test_access() -> Step {
        Step {
            step_id: "test-access".to_owned(),
            kind: Some(step::Kind::TestAccess(TestAccessStep {
                repository_id: "repo-1".to_owned(),
            })),
            ..Step::default()
        }
    }

    fn init() -> Step {
        Step {
            step_id: "init".to_owned(),
            kind: Some(step::Kind::Init(InitStep {
                repository_id: "repo-1".to_owned(),
            })),
            ..Step::default()
        }
    }

    fn job(agent: &str, tenant: &str, issued: i64, expires: i64, steps: Vec<Step>) -> Job {
        Job {
            job_id: "job-1".to_owned(),
            tenant_id: tenant.to_owned(),
            agent_id: agent.to_owned(),
            issued_at: Some(at(issued)),
            expires_at: Some(at(expires)),
            repositories: vec![RepositoryAccess {
                repository_id: "repo-1".to_owned(),
                location: "/srv/backup".to_owned(),
                password: "topsecret".to_owned(),
                ..RepositoryAccess::default()
            }],
            steps,
            ..Job::default()
        }
    }

    fn temp_journal(name: &str) -> FileJournal {
        let dir = std::env::temp_dir().join(format!(
            "restichub-agent-jobs-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("cannot create the directory");
        FileJournal::load(dir.join(crate::journal::JOURNAL_FILE)).expect("cannot load")
    }

    fn expected_reason(decision: &Intake) -> Option<RejectReason> {
        match decision {
            Intake::Rejected { reason, .. } => Some(*reason),
            _ => None,
        }
    }

    #[test]
    fn ac_pr_01_rejects_a_bad_signature() {
        let mut signed = sign_job(
            &job("agent-1", "tenant-1", 900, 2_000, vec![test_access()]),
            KEY_ID,
            &signing_key(),
        );
        signed.job[4] ^= 0xff;
        let mut journal = temp_journal("bad-signature");
        let decision = intake(&signed, &ring(), &expected(), &mut journal);
        assert_eq!(
            expected_reason(&decision).as_ref(),
            Some(&RejectReason::BadSignature)
        );
    }

    #[test]
    fn ac_pr_01_rejects_a_job_of_another_agent() {
        let signed = sign_job(
            &job("agent-2", "tenant-1", 900, 2_000, vec![test_access()]),
            KEY_ID,
            &signing_key(),
        );
        let mut journal = temp_journal("wrong-agent");
        let decision = intake(&signed, &ring(), &expected(), &mut journal);
        assert_eq!(
            expected_reason(&decision).as_ref(),
            Some(&RejectReason::WrongAgent)
        );
    }

    #[test]
    fn ac_pr_01_rejects_a_job_of_another_tenant() {
        let signed = sign_job(
            &job("agent-1", "tenant-2", 900, 2_000, vec![test_access()]),
            KEY_ID,
            &signing_key(),
        );
        let mut journal = temp_journal("wrong-tenant");
        let decision = intake(&signed, &ring(), &expected(), &mut journal);
        assert_eq!(
            expected_reason(&decision).as_ref(),
            Some(&RejectReason::WrongTenant)
        );
    }

    #[test]
    fn ac_pr_01_rejects_a_lifetime_over_24_hours() {
        let signed = sign_job(
            &job(
                "agent-1",
                "tenant-1",
                900,
                900 + 24 * 60 * 60 + 1,
                vec![test_access()],
            ),
            KEY_ID,
            &signing_key(),
        );
        let mut journal = temp_journal("lifetime");
        let decision = intake(&signed, &ring(), &expected(), &mut journal);
        assert_eq!(
            expected_reason(&decision).as_ref(),
            Some(&RejectReason::LifetimeTooLong)
        );
    }

    #[test]
    fn ac_pr_01_rejects_a_future_issued_at() {
        let signed = sign_job(
            &job(
                "agent-1",
                "tenant-1",
                1_000 + 5 * 60 + 1,
                2_000,
                vec![test_access()],
            ),
            KEY_ID,
            &signing_key(),
        );
        let mut journal = temp_journal("future");
        let decision = intake(&signed, &ring(), &expected(), &mut journal);
        assert_eq!(
            expected_reason(&decision).as_ref(),
            Some(&RejectReason::NotYetValid)
        );
    }

    #[test]
    fn ac_pr_01_rejects_an_expired_job() {
        let signed = sign_job(
            &job("agent-1", "tenant-1", 500, 800, vec![test_access()]),
            KEY_ID,
            &signing_key(),
        );
        let mut journal = temp_journal("expired");
        let decision = intake(&signed, &ring(), &expected(), &mut journal);
        assert_eq!(
            expected_reason(&decision).as_ref(),
            Some(&RejectReason::Expired)
        );
    }

    #[test]
    fn ac_pr_01_a_replay_of_a_completed_job_returns_the_stored_result() {
        let signed = sign_job(
            &job("agent-1", "tenant-1", 900, 2_000, vec![test_access()]),
            KEY_ID,
            &signing_key(),
        );
        let mut journal = temp_journal("replay");
        let first = intake(&signed, &ring(), &expected(), &mut journal);
        assert!(matches!(first, Intake::Accepted(_)));
        // Completion stores the result in the journal.
        use restichub_proto::signing::JobJournal as _;
        journal.record_completed(
            "job-1",
            restichub_proto::agent::v1::JobResult {
                job_id: "job-1".to_owned(),
                ..Default::default()
            },
        );
        // After a restart the journal holds the result, so the agent does not run the job again.
        let replay = intake(&signed, &ring(), &expected(), &mut journal);
        match replay {
            Intake::Completed(result) => assert_eq!(result.job_id, "job-1"),
            _ => panic!("the replay must return the stored result"),
        }
    }

    #[test]
    fn fr_pr_01_rejects_an_unsupported_step_kind() {
        let backup = Step {
            step_id: "backup".to_owned(),
            kind: Some(step::Kind::Backup(BackupStep {
                repository_id: "repo-1".to_owned(),
                paths: vec!["/srv".to_owned()],
                ..BackupStep::default()
            })),
            ..Step::default()
        };
        let signed = sign_job(
            &job("agent-1", "tenant-1", 900, 2_000, vec![backup]),
            KEY_ID,
            &signing_key(),
        );
        let mut journal = temp_journal("unsupported");
        let decision = intake(&signed, &ring(), &expected(), &mut journal);
        assert_eq!(
            expected_reason(&decision).as_ref(),
            Some(&RejectReason::UnsupportedStep)
        );
    }

    #[test]
    fn ac_ag_05_maps_the_restic_exit_codes() {
        assert_eq!(status_for_exit(0), StepStatus::Success);
        assert_eq!(status_for_exit(1), StepStatus::Failed);
        assert_eq!(status_for_exit(3), StepStatus::Warning);
        assert_eq!(status_for_exit(10), StepStatus::ConfigError);
        assert_eq!(status_for_exit(11), StepStatus::Retry);
        assert_eq!(status_for_exit(12), StepStatus::ConfigError);
        assert_eq!(status_for_exit(130), StepStatus::Cancelled);
        assert_eq!(status_for_exit(99), StepStatus::Failed);
    }

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
        assert_eq!(
            job_error(JobStatus::ConfigError).map(|error| error.code),
            Some("wrong_password".to_owned())
        );
        assert_eq!(
            job_error(JobStatus::NotInitialized).map(|error| error.code),
            Some("repository_missing".to_owned())
        );
    }

    #[test]
    fn a_repository_step_is_needed_for_the_chain() {
        // The init chain starts with a test_access step; the job of the control plane holds it.
        let steps = vec![test_access(), init()];
        let signed = sign_job(
            &job("agent-1", "tenant-1", 900, 2_000, steps),
            KEY_ID,
            &signing_key(),
        );
        let mut journal = temp_journal("chain");
        let decision = intake(&signed, &ring(), &expected(), &mut journal);
        match decision {
            Intake::Accepted(validated) => assert_eq!(validated.job().steps.len(), 2),
            _ => panic!("the init chain must be accepted"),
        }
    }
}
