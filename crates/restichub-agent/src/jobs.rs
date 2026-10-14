//! The agent-side execution of the repository init and test-access jobs (FR-PR-03, FR-PR-06,
//! FR-RP-02, FR-AG-09, FR-AG-10, ADR-0020, ADR-0026).
//!
//! The control plane delivers a signed job on the stream. The agent verifies the signature with the
//! key ring and checks the addressee, the tenant and the time rules ([`verify_job`]), validates the
//! steps ([`ValidatedJob`], FR-PR-01) and consults the job journal (ADR-0020). It then acknowledges
//! the job (FR-PR-06) and runs it off the stream task, so the heartbeat and the key updates keep
//! flowing. A job that the journal already completed is not run again; the stored result is re-sent.
//!
//! The agent sends every result through the stream that is open at that moment, not through the
//! stream that delivered the job: a job outlives one connection. The result is durable in the
//! journal before the send. A send into the stream buffer does not prove that the control plane got
//! the result, so the agent sends every stored result at each connection until the journal entry
//! expires (ADR-0020, FR-AG-12). The control plane ignores a result for a terminal job.
//!
//! This step of the epic runs `test_access` and `init` only. Any other step kind is refused with
//! [`RejectReason::UnsupportedStep`]. The agent starts no job on its own and holds no timer that
//! starts one (ADR-0002).
//!
//! The restic command line comes from the typed step through the compiler builders (ADR-0021), so no
//! value of a job becomes an option. The repository secrets reach restic only through the child
//! environment (FR-AG-06); they never reach an argument, a log, the journal or an error. The agent
//! runs `init` only after a `test_access` of the same repository exited with restic code 10 (the
//! repository does not exist); any other result of `test_access` stops the job with its typed
//! outcome (AC-RP-01). A cancel or a timeout stops the step in two stages (FR-AG-10).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Context as _, Result};
use prost_types::Timestamp;
use restichub_compiler::ValidatedJob;
use restichub_compiler::arg::build_command;
use restichub_proto::agent::v1::{
    AgentMessage, ErrorDetail, JobAck, JobRejected, JobResult, JobStatus, RejectReason, SignedJob,
    Step, StepResult, StepStatus, agent_message, step,
};
use restichub_proto::signing::{
    Admission, Expected, JobJournal as _, JournalState, KeyRing, Rejection, admit, verify_job,
};
use tokio::sync::mpsc;

use crate::exec::JobChild;
use crate::journal::{FileJournal, JOURNAL_FILE, JournalSnapshot};

/// How often the agent checks the restic child for its exit.
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// The grace period between the graceful stop signal and the forced kill of the process tree
/// (FR-AG-10, AC-AG-06).
pub const STOP_GRACE: Duration = Duration::from_secs(30);

/// The timeout of a step that gives none and whose job gives none either (FR-AG-05).
const DEFAULT_STEP_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// The largest number of jobs that the agent runs at the same time (ADR-0020).
const MAX_CONCURRENT_JOBS: usize = 4;

/// The action that [`Jobs::handle`] takes after it consults the journal (ADR-0020).
enum Dispatch {
    Rejected(RejectReason),
    Completed(JobResult),
    Running,
    /// The job is reserved in `running`. A snapshot is present for a new journal entry, which the
    /// caller writes before it runs the job.
    Run {
        cancel: CancelHandle,
        snapshot: Option<Result<JournalSnapshot>>,
    },
}

/// The running jobs of one agent and the journal that records them (ADR-0020).
#[derive(Clone)]
pub struct Jobs {
    program: PathBuf,
    expected_sha256: String,
    cache: PathBuf,
    journal: Arc<Mutex<FileJournal>>,
    running: Arc<Mutex<HashMap<String, CancelHandle>>>,
    /// The stream that is open at this moment, or `None` between two connections (ADR-0020). A
    /// result goes through the current stream, not through the one that delivered the job.
    connection: Arc<Mutex<Option<mpsc::Sender<AgentMessage>>>>,
    /// The sequence of the newest journal write on disk, which also serializes the writers.
    written: Arc<Mutex<u64>>,
    /// The grace period between the graceful stop and the forced kill (FR-AG-10).
    stop_grace: Duration,
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
            connection: Arc::new(Mutex::new(None)),
            written: Arc::new(Mutex::new(0)),
            stop_grace: STOP_GRACE,
        })
    }

    /// Sets the grace period between the graceful stop and the forced kill (FR-AG-10).
    #[must_use]
    pub fn with_stop_grace(mut self, grace: Duration) -> Self {
        self.stop_grace = grace;
        self
    }

    /// Sets the stream that carries the results (ADR-0020).
    ///
    /// The connect loop calls it after the stream opens, before it sends the stored results.
    pub fn attach(&self, tx: &mpsc::Sender<AgentMessage>) {
        *lock(&self.connection) = Some(tx.clone());
    }

    /// Verifies, admits and dispatches one signed job (FR-PR-03, FR-PR-06, ADR-0020).
    ///
    /// A refusal and a replayed job are answered at once. An accepted job is acknowledged and runs
    /// off the stream task, so this call returns without waiting for it. A job that already runs in
    /// this process is acknowledged again and is not started a second time (ADR-0020).
    pub async fn handle(&self, signed: SignedJob, ring: &KeyRing, agent_id: &str, tenant_id: &str) {
        let expected = Expected {
            agent_id: agent_id.to_owned(),
            tenant_id: tenant_id.to_owned(),
            now: now_timestamp(),
        };
        let validated = match verify_and_validate(&signed, ring, &expected) {
            Ok(validated) => validated,
            Err(rejection) => {
                let job_id = rejection.job_id.unwrap_or_default();
                let _ = self.send(rejected(job_id, rejection.reason)).await;
                return;
            }
        };
        let job_id = validated.job().job_id.clone();
        match self.dispatch(&validated, expected.now) {
            Dispatch::Rejected(reason) => {
                let _ = self.send(rejected(job_id, reason)).await;
            }
            Dispatch::Completed(result) => {
                let _ = self.send(job_result_message(result)).await;
            }
            // A redelivery of a running job does not fork a second restic (ADR-0020).
            Dispatch::Running => {
                let _ = self.send(ack(job_id)).await;
            }
            Dispatch::Run { cancel, snapshot } => {
                if let Some(snapshot) = snapshot
                    && let Err(error) = self.write_journal(snapshot).await
                {
                    tracing::warn!(%error, "cannot write the job journal");
                    // Fail closed: the agent cannot record the accepted job, so it does not run it.
                    self.abandon(&job_id).await;
                    let _ = self.send(rejected(job_id, RejectReason::Internal)).await;
                    return;
                }
                if self.send(ack(job_id.clone())).await {
                    let jobs = self.clone();
                    tokio::spawn(async move {
                        jobs.execute(validated, cancel).await;
                    });
                } else {
                    // The stream dropped after the admission; the control plane delivers the job
                    // again and the journal answers that replay (ADR-0020).
                    lock(&self.running).remove(&job_id);
                }
            }
        }
    }

    /// Decides what to do with a verified job: the stored result first, then a running job, then
    /// the concurrency cap and the journal room (ADR-0020).
    fn dispatch(&self, validated: &ValidatedJob, now: Timestamp) -> Dispatch {
        let job = validated.job();
        // The journal lock comes before the `running` lock everywhere, so the two never deadlock.
        let mut journal = lock(&self.journal);
        let mut running = lock(&self.running);
        if let JournalState::Completed(result) = journal.lookup(&job.job_id) {
            return Dispatch::Completed(result);
        }
        if running.contains_key(&job.job_id) {
            return Dispatch::Running;
        }
        // The agent refuses a new job beyond the cap, so the host never runs an unbounded number of
        // restic processes.
        if running.len() >= MAX_CONCURRENT_JOBS || !journal.make_room(now) {
            return Dispatch::Rejected(RejectReason::Busy);
        }
        let expires_at = job.expires_at.unwrap_or(now);
        let snapshot = match admit(job, &mut *journal, expires_at) {
            // The journal changes only for a new admission (ADR-0020).
            Admission::New => Some(journal.snapshot()),
            Admission::InProgress | Admission::AlreadyCompleted(_) => None,
        };
        let cancel = CancelHandle::new();
        running.insert(job.job_id.clone(), cancel.clone());
        Dispatch::Run { cancel, snapshot }
    }

    /// Writes a journal snapshot off the async workers: the write waits for the disk (FR-AG-04).
    async fn write_journal(&self, snapshot: Result<JournalSnapshot>) -> Result<()> {
        let snapshot = snapshot?;
        let written = Arc::clone(&self.written);
        tokio::task::spawn_blocking(move || snapshot.write(&written))
            .await
            .context("the journal write stopped")?
    }

    /// Drops the entry of a job that the agent cannot run and releases its slot.
    async fn abandon(&self, job_id: &str) {
        let snapshot = {
            let mut journal = lock(&self.journal);
            journal.forget(job_id);
            lock(&self.running).remove(job_id);
            journal.snapshot()
        };
        if let Err(error) = self.write_journal(snapshot).await {
            tracing::warn!(%error, "cannot write the job journal");
        }
    }

    /// Stops the restic child of the running job (FR-AG-10).
    ///
    /// It sets the flag; the step loop sends the signal and, after the grace period, kills the tree.
    pub fn cancel(&self, job_id: &str) {
        if let Some(handle) = lock(&self.running).get(job_id) {
            handle.cancel();
        }
    }

    /// Re-sends the results that the control plane may have missed (ADR-0020).
    ///
    /// The agent calls it at every connection, so a result that a dropped stream lost reaches the
    /// control plane. The send is repeated until the journal entry expires.
    pub async fn send_stored_results(&self) {
        let stored = lock(&self.journal).stored_results();
        for result in stored {
            if !self.send(job_result_message(result)).await {
                return;
            }
        }
    }

    /// Sends one message through the stream that is open at this moment (ADR-0020).
    ///
    /// False when no stream is open or the send fails.
    async fn send(&self, message: AgentMessage) -> bool {
        let tx = lock(&self.connection).clone();
        match tx {
            Some(tx) => tx.send(message).await.is_ok(),
            None => false,
        }
    }

    /// Runs the steps of one job and reports the typed result (FR-AG-09, AC-RP-01).
    async fn execute(&self, validated: ValidatedJob, cancel: CancelHandle) {
        let job = validated.job();
        let job_id = job.job_id.clone();
        let steps_total = u32::try_from(job.steps.len()).unwrap_or(u32::MAX);
        let had_init = job
            .steps
            .iter()
            .any(|step| matches!(step.kind, Some(step::Kind::Init(_))));
        let job_deadline = (job.timeout_seconds > 0).then(|| {
            tokio::time::Instant::now() + Duration::from_secs(u64::from(job.timeout_seconds))
        });
        let mut succeeded = 0_u32;
        let mut failed = 0_u32;
        let mut cancelled = false;
        // The exit code of a step that stopped the job outside the missing-repository probe.
        let mut stopped_code: Option<i32> = None;
        // The repository of the test_access step and its probe result. The init step runs only when
        // its own repository matches (FR-RP-02, AC-RP-01).
        let mut probe_repository: Option<String> = None;
        let mut probe_missing = false;
        let mut init_ran = false;
        let mut init_succeeded = false;
        let mut init_invalid = false;

        for step in &job.steps {
            // A cancel that arrived between two steps stops the job before the next step starts
            // (FR-AG-10).
            if cancel.is_cancelled() {
                cancelled = true;
                break;
            }
            let is_test_access = matches!(step.kind, Some(step::Kind::TestAccess(_)));
            let is_init = matches!(step.kind, Some(step::Kind::Init(_)));
            if is_init {
                let bound = probe_repository
                    .as_deref()
                    .is_some_and(|probed| Some(probed) == repository_id(step).as_deref());
                if !bound {
                    // The job holds an init without a preceding test_access of the same repository,
                    // so the sequence is invalid and the job fails (FR-RP-02, AC-RP-01).
                    init_invalid = true;
                    failed += 1;
                    let now = now_timestamp();
                    self.send_step_result(&job_id, step, StepOutcome::failed(), now, now)
                        .await;
                    break;
                }
                if !probe_missing {
                    // The repository exists, so init is skipped and never re-initializes it.
                    let now = now_timestamp();
                    self.send_step_result(&job_id, step, StepOutcome::skipped(), now, now)
                        .await;
                    continue;
                }
            }
            let started_at = now_timestamp();
            let mut limit = step_timeout(step.timeout_seconds, job.timeout_seconds);
            if let Some(deadline) = job_deadline {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    // The job deadline passed between two steps, so the next step does not start.
                    failed += 1;
                    stopped_code = Some(StepOutcome::failed().exit_code);
                    self.send_step_result(
                        &job_id,
                        step,
                        StepOutcome::failed(),
                        started_at,
                        started_at,
                    )
                    .await;
                    break;
                }
                limit = limit.min(remaining);
            }
            if is_init {
                init_ran = true;
            }
            let outcome = self.run_step(&validated, step, &cancel, Some(limit)).await;
            let finished_at = now_timestamp();
            self.send_step_result(&job_id, step, outcome, started_at, finished_at)
                .await;
            if is_test_access {
                probe_repository = repository_id(step);
                probe_missing = outcome.exit_code == 10;
            }
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
            init_ran,
            init_succeeded,
            init_invalid,
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
        // The result is durable before the job leaves `running`, so a redelivery in between gets the
        // acknowledgement and a later one gets the stored result, never a second run (ADR-0020).
        let snapshot = {
            let mut journal = lock(&self.journal);
            journal.record_completed(&job_id, result.clone());
            journal.prune(now_timestamp());
            journal.snapshot()
        };
        if let Err(error) = self.write_journal(snapshot).await {
            tracing::warn!(%error, "cannot write the job journal");
        }
        lock(&self.running).remove(&job_id);
        // The result stays in the journal and goes out at every connection until the entry expires
        // (ADR-0020, FR-AG-12).
        let _ = self.send(job_result_message(result)).await;
    }

    /// Runs one step with the bundled restic and stops it in two stages (FR-AG-05, FR-AG-06,
    /// FR-AG-09, FR-AG-10).
    ///
    /// `limit` is the timeout of the step, or `None` for no timeout.
    async fn run_step(
        &self,
        validated: &ValidatedJob,
        step: &Step,
        cancel: &CancelHandle,
        limit: Option<Duration>,
    ) -> StepOutcome {
        // A cancel that arrived before the spawn starts no process (FR-AG-10).
        if cancel.is_cancelled() {
            return StepOutcome::cancelled(false);
        }
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
        let started = tokio::time::Instant::now();
        let mut timed_out = false;
        // The stop is two stages: the graceful signal first, then the forced kill of the tree after
        // the grace period (FR-AG-10, AC-AG-07).
        let mut stopped_at: Option<tokio::time::Instant> = None;
        let mut hard_killed = false;
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
                if cancel.is_cancelled() {
                    return StepOutcome::cancelled(hard_killed);
                }
                if timed_out {
                    return StepOutcome {
                        possible_stale_lock: hard_killed,
                        ..StepOutcome::failed()
                    };
                }
                return StepOutcome::from_exit(code);
            }
            if stopped_at.is_none() {
                if cancel.is_cancelled() {
                    interrupt(&cancel.child);
                    stopped_at = Some(tokio::time::Instant::now());
                } else if limit.is_some_and(|limit| started.elapsed() >= limit) {
                    timed_out = true;
                    interrupt(&cancel.child);
                    stopped_at = Some(tokio::time::Instant::now());
                }
            } else if !hard_killed
                && stopped_at.is_some_and(|since| since.elapsed() >= self.stop_grace)
            {
                kill_tree(&cancel.child);
                hard_killed = true;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    /// Sends the result of one step through the current stream (FR-AG-09).
    async fn send_step_result(
        &self,
        job_id: &str,
        step: &Step,
        outcome: StepOutcome,
        started_at: Timestamp,
        finished_at: Timestamp,
    ) {
        let result = StepResult {
            job_id: job_id.to_owned(),
            step_id: step.step_id.clone(),
            status: outcome.status as i32,
            exit_code: outcome.exit_code,
            started_at: Some(started_at),
            finished_at: Some(finished_at),
            possible_stale_lock: outcome.possible_stale_lock,
            ..StepResult::default()
        };
        let _ = self
            .send(AgentMessage {
                payload: Some(agent_message::Payload::StepResult(result)),
            })
            .await;
    }
}

/// Sends the graceful stop signal to the child of a step (FR-AG-10).
fn interrupt(child: &Mutex<Option<JobChild>>) {
    if let Some(child) = lock(child).as_ref()
        && let Err(error) = child.interrupt()
    {
        tracing::warn!(%error, "cannot stop the restic child");
    }
}

/// Kills the whole process tree of the child of a step (FR-AG-10).
fn kill_tree(child: &Mutex<Option<JobChild>>) {
    if let Some(child) = lock(child).as_mut()
        && let Err(error) = child.kill_tree()
    {
        tracing::warn!(%error, "cannot kill the restic child");
    }
}

/// Verifies the signature and the job rules, then validates the steps (FR-PR-03, FR-PR-01).
///
/// The function is separate from [`Jobs::handle`] so a test can drive the checks without a restic
/// child.
fn verify_and_validate(
    signed: &SignedJob,
    ring: &KeyRing,
    expected: &Expected,
) -> Result<ValidatedJob, Rejection> {
    let job = verify_job(signed, ring, expected)?;
    let job_id = job.job_id.clone();
    let validated = ValidatedJob::new(job).map_err(|rejection| Rejection {
        job_id: rejection.job_id.or_else(|| Some(job_id.clone())),
        reason: rejection.reason,
    })?;
    if validated
        .job()
        .steps
        .iter()
        .any(|step| !step.kind.as_ref().is_some_and(is_supported))
    {
        return Err(Rejection {
            job_id: Some(job_id),
            reason: RejectReason::UnsupportedStep,
        });
    }
    Ok(validated)
}

/// The step kinds that this step of the epic runs (FR-PR-02).
fn is_supported(kind: &step::Kind) -> bool {
    matches!(kind, step::Kind::Init(_) | step::Kind::TestAccess(_))
}

/// The repository that a step acts on (FR-RP-02).
fn repository_id(step: &Step) -> Option<String> {
    match step.kind.as_ref()? {
        step::Kind::Init(init) => Some(init.repository_id.clone()),
        step::Kind::TestAccess(test_access) => Some(test_access.repository_id.clone()),
        _ => None,
    }
}

/// The timeout of one step: its own value, the job value when the step gives none, or the default
/// (FR-AG-05).
#[must_use]
fn step_timeout(step_seconds: u32, job_seconds: u32) -> Duration {
    if step_seconds > 0 {
        Duration::from_secs(u64::from(step_seconds))
    } else if job_seconds > 0 {
        Duration::from_secs(u64::from(job_seconds))
    } else {
        DEFAULT_STEP_TIMEOUT
    }
}

/// Exit status and code of one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StepOutcome {
    status: StepStatus,
    exit_code: i32,
    /// Set after a forced kill of the tree: restic may have left a lock (FR-AG-10, AC-AG-07).
    possible_stale_lock: bool,
}

impl StepOutcome {
    fn failed() -> Self {
        Self {
            status: StepStatus::Failed,
            exit_code: -1,
            possible_stale_lock: false,
        }
    }

    fn skipped() -> Self {
        Self {
            status: StepStatus::Skipped,
            exit_code: -1,
            possible_stale_lock: false,
        }
    }

    fn cancelled(possible_stale_lock: bool) -> Self {
        Self {
            status: StepStatus::Cancelled,
            exit_code: -1,
            possible_stale_lock,
        }
    }

    fn from_exit(code: i32) -> Self {
        Self {
            status: status_for_exit(code),
            exit_code: code,
            possible_stale_lock: false,
        }
    }
}

/// The handle that stops the restic child of a running job (FR-AG-10).
#[derive(Clone)]
struct CancelHandle {
    child: Arc<Mutex<Option<JobChild>>>,
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
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
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
/// The init chain: the agent runs init only after a `test_access` of the same repository exited with
/// code 10 (a missing repository). A successful init after that probe is a successful job. A
/// test_access that failed with any other code (1, 11, 12, a timeout, access denied) stops the job,
/// and a standalone test_access with exit code 10 is the normal `NOT_INITIALIZED` result. An init
/// without a matching test_access is an invalid sequence and fails the job.
#[allow(clippy::fn_params_excessive_bools)]
fn job_outcome(
    cancelled: bool,
    stopped_code: Option<i32>,
    probe_missing: bool,
    had_init: bool,
    init_ran: bool,
    init_succeeded: bool,
    init_invalid: bool,
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
    if init_invalid {
        return JobStatus::Failed;
    }
    if had_init && init_ran {
        return if init_succeeded {
            JobStatus::Succeeded
        } else {
            JobStatus::Failed
        };
    }
    if !had_init && probe_missing {
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

fn ack(job_id: String) -> AgentMessage {
    AgentMessage {
        payload: Some(agent_message::Payload::JobAck(JobAck { job_id })),
    }
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

    use super::{job_error, job_outcome, status_for_exit, verify_and_validate};
    use crate::journal::FileJournal;
    use restichub_compiler::ValidatedJob;
    use restichub_proto::agent::v1::{JobStatus, RejectReason, SignedJob, StepStatus};
    use restichub_proto::signing::{Admission, admit};
    use std::time::Duration;

    /// The decision of the verification and the journal for one signed job.
    enum Intake {
        Rejected(RejectReason),
        Completed(Box<restichub_proto::agent::v1::JobResult>),
        New(Box<ValidatedJob>),
    }

    /// Runs the same checks as `Jobs::handle` and admits the job with the expiry that it carries.
    fn intake(
        signed: &SignedJob,
        ring: &KeyRing,
        expected: &Expected,
        journal: &mut FileJournal,
    ) -> Intake {
        match verify_and_validate(signed, ring, expected) {
            Ok(validated) => {
                let expires_at = validated.job().expires_at.unwrap_or(expected.now);
                match admit(validated.job(), journal, expires_at) {
                    Admission::AlreadyCompleted(result) => Intake::Completed(Box::new(result)),
                    Admission::New | Admission::InProgress => Intake::New(Box::new(validated)),
                }
            }
            Err(rejection) => Intake::Rejected(rejection.reason),
        }
    }

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
            Intake::Rejected(reason) => Some(*reason),
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
        assert!(matches!(first, Intake::New(_)));
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
            job_outcome(false, None, false, true, false, false, false),
            JobStatus::Succeeded
        );
        // A missing repository with a successful init.
        assert_eq!(
            job_outcome(false, None, true, true, true, true, false),
            JobStatus::Succeeded
        );
        // Init failure gives a failed job.
        assert_eq!(
            job_outcome(false, None, true, true, true, false, false),
            JobStatus::Failed
        );
        // A standalone test_access on a missing repository.
        assert_eq!(
            job_outcome(false, None, true, false, false, false, false),
            JobStatus::NotInitialized
        );
        // An init step without a matching test_access is an invalid sequence.
        assert_eq!(
            job_outcome(false, None, false, true, false, false, true),
            JobStatus::Failed
        );
        // A wrong password on an existing repository (exit code 12).
        assert_eq!(
            job_outcome(false, Some(12), false, true, false, false, false),
            JobStatus::ConfigError
        );
        // Any other failure (exit codes 1, 11, a timeout).
        assert_eq!(
            job_outcome(false, Some(1), false, true, false, false, false),
            JobStatus::Failed
        );
        assert_eq!(
            job_outcome(false, Some(11), false, true, false, false, false),
            JobStatus::Failed
        );
        // The control plane cancelled the job before any step finished.
        assert_eq!(
            job_outcome(true, None, false, true, false, false, false),
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
    fn fr_ag_05_a_step_without_a_timeout_uses_the_job_timeout_or_the_default() {
        // Step value wins over the job value.
        assert_eq!(super::step_timeout(600, 900), Duration::from_secs(600));
        // No step value: the job value stands in.
        assert_eq!(super::step_timeout(0, 900), Duration::from_secs(900));
        // Neither: the default.
        assert_eq!(super::step_timeout(0, 0), super::DEFAULT_STEP_TIMEOUT);
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
            Intake::New(validated) => assert_eq!(validated.job().steps.len(), 2),
            _ => panic!("the init chain must be accepted"),
        }
    }
}
