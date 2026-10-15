//! The agent-side execution of the jobs: repository init and test-access, backup, check,
//! snapshots, ls, restore, forget, prune, unlock and stats (FR-PR-02, FR-PR-03, FR-PR-06, FR-RP-02, FR-AG-08,
//! FR-AG-09, FR-AG-10, ADR-0020, ADR-0026).
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
//! The agent refuses a job with a `hook` step with [`RejectReason::UnsupportedStep`]: that step
//! kind belongs to a later epic. The agent starts no job on its own and holds no timer that starts
//! one (ADR-0002).
//!
//! While a step runs, the agent sends progress and scrubbed log chunks. The result of the job holds
//! the typed result of every step and a bounded tail of the log. Both stay in the journal, so a
//! job that finished while the control plane was away reaches it after the reconnect (AC-PR-07).
//!
//! The restic command line comes from the typed step through the compiler builders (ADR-0021), so no
//! value of a job becomes an option. The repository secrets reach restic only through the child
//! environment (FR-AG-06); they never reach an argument, a log, the journal or an error: the agent
//! scrubs the restic output before it sends or stores a line. The agent
//! runs `init` only after a `test_access` of the same repository exited with restic code 10 (the
//! repository does not exist); any other result of `test_access` stops the job with its typed
//! outcome (AC-RP-01). A cancel or a timeout stops the step in two stages (FR-AG-10).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Context as _, Result};
use prost_types::Timestamp;
use restichub_compiler::ValidatedJob;
use restichub_proto::agent::v1::{
    AgentMessage, ErrorDetail, JobAck, JobRejected, JobResult, JobStatus, RejectReason, SignedJob,
    Step, StepResult, StepStatus, agent_message, step, step_result,
};
use restichub_proto::signing::{
    Admission, Expected, JobJournal as _, JournalState, KeyRing, Rejection, admit, verify_job,
};
use tokio::sync::mpsc;

use crate::journal::{FileJournal, JOURNAL_FILE, JournalSnapshot};
use crate::restore_roots::RestoreAccess;
use crate::restic_store::Layout;

mod log;
mod runner;

use log::JobLog;
use runner::{CancelHandle, StepError, StepOutcome};

/// The grace period between the graceful stop signal and the forced kill of the process tree
/// (FR-AG-10, AC-AG-06).
pub const STOP_GRACE: Duration = Duration::from_secs(30);

/// The timeout of a step that gives none and whose job gives none either (FR-AG-05).
const DEFAULT_STEP_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// The largest number of jobs that the agent runs at the same time (ADR-0020).
const MAX_CONCURRENT_JOBS: usize = 4;

/// The largest number of browse jobs (`snapshots` and `ls`) at the same time. They have their own
/// room, so a person who browses cannot make a scheduled backup `Busy` (ADR-0035).
const MAX_BROWSE_JOBS: usize = 2;

/// The action that [`Jobs::handle`] takes after it consults the journal (ADR-0020).
enum Dispatch {
    Rejected(RejectReason),
    Completed(JobResult),
    Running,
    /// The job is reserved in `running`. A snapshot is present for a new journal entry, which the
    /// caller writes before it runs the job.
    Run {
        cancel: CancelHandle,
        snapshot: Option<JournalSnapshot>,
    },
}

/// The running jobs of one agent and the journal that records them (ADR-0020).
#[derive(Clone)]
pub struct Jobs {
    program: PathBuf,
    expected_sha256: String,
    /// Where the protected record lives. A job reads the hash from it at every spawn, so a
    /// `restic update` takes effect without a restart (FR-AG-11, threat T20).
    record: Option<Arc<Layout>>,
    cache: PathBuf,
    journal: Arc<Mutex<FileJournal>>,
    running: Arc<Mutex<HashMap<String, CancelHandle>>>,
    /// Identifiers of the running jobs that use the room of [`MAX_BROWSE_JOBS`] only.
    browse: Arc<Mutex<HashSet<String>>>,
    /// The stream that is open at this moment, or `None` between two connections (ADR-0020). A
    /// result goes through the current stream, not through the one that delivered the job.
    connection: Arc<Mutex<Option<mpsc::Sender<AgentMessage>>>>,
    /// The sequence of the newest journal write on disk, which also serializes the writers.
    written: Arc<Mutex<u64>>,
    /// The jobs whose log tail and step results this process already re-sent. The control plane
    /// has no acknowledgement of a result, so a later connection sends the job result alone.
    resent: Arc<Mutex<HashSet<String>>>,
    /// The grace period between the graceful stop and the forced kill (FR-AG-10).
    stop_grace: Duration,
    /// The roots where a restore may write and the files of the agent that it may never touch
    /// (FR-RS-03, ADR-0035).
    restore: Arc<RestoreAccess>,
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
        let restore = RestoreAccess::for_host(dir, &cache, &program);
        Ok(Self {
            program,
            expected_sha256,
            record: None,
            cache,
            journal: Arc::new(Mutex::new(journal)),
            running: Arc::new(Mutex::new(HashMap::new())),
            browse: Arc::new(Mutex::new(HashSet::new())),
            connection: Arc::new(Mutex::new(None)),
            written: Arc::new(Mutex::new(0)),
            resent: Arc::new(Mutex::new(HashSet::new())),
            stop_grace: STOP_GRACE,
            restore: Arc::new(restore),
        })
    }

    /// Reads the expected hash from the protected record before every spawn (ADR-0038).
    #[must_use]
    pub fn with_record(mut self, layout: Layout) -> Self {
        self.record = Some(Arc::new(layout));
        self
    }

    /// The SHA-256 that the next spawn must match.
    pub(crate) fn expected_sha256(&self) -> Result<String> {
        match &self.record {
            Some(layout) => Ok(layout.read_record()?.sha256),
            None => Ok(self.expected_sha256.clone()),
        }
    }

    /// Replaces the restore roots and the protected paths (FR-RS-03).
    #[must_use]
    pub fn with_restore_access(mut self, access: RestoreAccess) -> Self {
        self.restore = Arc::new(access);
        self
    }

    /// Roots for `AgentHello`, so the control plane can suggest a target (FR-RS-03).
    #[must_use]
    pub fn restore_roots(&self) -> Vec<String> {
        self.restore.root_strings()
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
        if validated
            .job()
            .steps
            .iter()
            .any(|step| self.restore_denied(step))
        {
            let _ = self
                .send(rejected(job_id, RejectReason::RestoreTargetDenied))
                .await;
            return;
        }
        match self.dispatch(&validated, expected.now) {
            Dispatch::Rejected(reason) => {
                let _ = self.send(rejected(job_id, reason)).await;
            }
            Dispatch::Completed(result) => {
                self.send_result(result).await;
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
                    self.release(&job_id);
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
        let browse = is_browse(job);
        let browse_running = lock(&self.browse).len();
        let full = if browse {
            browse_running >= MAX_BROWSE_JOBS
        } else {
            running.len().saturating_sub(browse_running) >= MAX_CONCURRENT_JOBS
        };
        if full || !journal.make_room(now) {
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
        if browse {
            lock(&self.browse).insert(job.job_id.clone());
        }
        Dispatch::Run { cancel, snapshot }
    }

    /// Frees the room of a job that stopped.
    fn release(&self, job_id: &str) {
        lock(&self.running).remove(job_id);
        lock(&self.browse).remove(job_id);
    }

    /// Writes a journal snapshot off the async workers: the write waits for the disk (FR-AG-04).
    async fn write_journal(&self, snapshot: JournalSnapshot) -> Result<()> {
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
            self.release(job_id);
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
    /// control plane. The job result is repeated until the journal entry expires; the log tail and
    /// the step results go out once for each process start.
    pub async fn send_stored_results(&self) {
        let stored = lock(&self.journal).stored_results();
        lock(&self.resent).retain(|id| stored.iter().any(|result| &result.job_id == id));
        for result in stored {
            if !self.send_result(result).await {
                return;
            }
        }
    }

    /// Sends the log tail, the step results and the job result of a stored job (AC-PR-07).
    ///
    /// The log tail and the step results go out only if this process did not send them yet. False
    /// when no stream is open or a send fails.
    async fn send_result(&self, mut result: JobResult) -> bool {
        if lock(&self.resent).contains(&result.job_id) {
            result.log_tail.clear();
            result.steps.clear();
            return self.send(job_result_message(result)).await;
        }
        let job_id = result.job_id.clone();
        for chunk in std::mem::take(&mut result.log_tail) {
            let message = AgentMessage {
                payload: Some(agent_message::Payload::LogChunk(chunk)),
            };
            if !self.send(message).await {
                return false;
            }
        }
        for step_result in std::mem::take(&mut result.steps) {
            let message = AgentMessage {
                payload: Some(agent_message::Payload::StepResult(step_result)),
            };
            if !self.send(message).await {
                return false;
            }
        }
        lock(&self.resent).insert(job_id);
        self.send(job_result_message(result)).await
    }

    /// Sends a message that may be lost: progress and log chunks wait `wait` for room in the stream
    /// buffer, so a slow control plane never blocks a cancel (FR-AG-10).
    async fn send_within(&self, message: AgentMessage, wait: Duration) {
        let tx = lock(&self.connection).clone();
        if let Some(tx) = tx {
            let _ = tokio::time::timeout(wait, tx.send(message)).await;
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
        let mut log = JobLog::new(&job_id);
        let mut step_results: Vec<StepResult> = Vec::new();
        let mut succeeded = 0_u32;
        let mut failed = 0_u32;
        let mut progress = Progress {
            had_init,
            ..Progress::default()
        };
        // The status of a step that stopped the job outside the missing-repository probe.
        let mut stopped_status: Option<StepStatus> = None;
        // The repository of the test_access step. The init step runs only when its own repository
        // matches (FR-RP-02, AC-RP-01).
        let mut probe_repository: Option<String> = None;

        for step in &job.steps {
            // A cancel that arrived between two steps stops the job before the next step starts
            // (FR-AG-10).
            if cancel.is_cancelled() {
                progress.cancelled = true;
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
                    progress.init_invalid = true;
                    failed += 1;
                    let now = now_timestamp();
                    step_results.push(
                        self.send_step_result(&job_id, step, &StepOutcome::failed(), now, now)
                            .await,
                    );
                    break;
                }
                if !progress.probe_missing {
                    // The repository exists, so init is skipped and never re-initializes it.
                    let now = now_timestamp();
                    step_results.push(
                        self.send_step_result(&job_id, step, &StepOutcome::skipped(), now, now)
                            .await,
                    );
                    continue;
                }
            }
            let started_at = now_timestamp();
            // A link can change between the admission and the start, so the target is checked
            // again before the restore writes (FR-RS-03).
            if self.restore_denied(step) {
                failed += 1;
                progress.stopped_code = Some(StepOutcome::failed().exit_code);
                stopped_status = Some(StepStatus::Failed);
                step_results.push(
                    self.send_step_result(
                        &job_id,
                        step,
                        &StepOutcome::failed(),
                        started_at,
                        started_at,
                    )
                    .await,
                );
                break;
            }
            let mut limit = step_timeout(step.timeout_seconds, job.timeout_seconds);
            if let Some(deadline) = job_deadline {
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    // The job deadline passed between two steps, so the next step does not start.
                    failed += 1;
                    progress.stopped_code = Some(StepOutcome::failed().exit_code);
                    stopped_status = Some(StepStatus::Failed);
                    step_results.push(
                        self.send_step_result(
                            &job_id,
                            step,
                            &StepOutcome::failed(),
                            started_at,
                            started_at,
                        )
                        .await,
                    );
                    break;
                }
                limit = limit.min(remaining);
            }
            if is_init {
                progress.init_ran = true;
            }
            let outcome = self
                .run_step(&validated, step, &cancel, Some(limit), &mut log)
                .await;
            let finished_at = now_timestamp();
            step_results.push(
                self.send_step_result(&job_id, step, &outcome, started_at, finished_at)
                    .await,
            );
            if is_test_access {
                probe_repository = repository_id(step);
                progress.probe_missing = outcome.exit_code == 10;
            }
            match outcome.status {
                StepStatus::Success => {
                    succeeded += 1;
                    if is_init {
                        progress.init_succeeded = true;
                    }
                }
                // Restic finished and reported a problem with some files: the job goes on.
                StepStatus::Warning => {
                    succeeded += 1;
                    progress.warned = true;
                }
                StepStatus::Cancelled => {
                    progress.cancelled = true;
                    break;
                }
                _ => {
                    failed += 1;
                    if is_test_access && outcome.exit_code == 10 {
                        // The repository does not exist: the init step may run.
                    } else {
                        // Every other failure stops the job with its own outcome: no init runs
                        // (FR-RP-02, AC-RP-01).
                        progress.stopped_code = Some(outcome.exit_code);
                        stopped_status = Some(outcome.status);
                        break;
                    }
                }
            }
        }

        self.send_log(&mut log).await;
        let outcome = job_outcome(&progress);
        let status = stopped_status.unwrap_or(match outcome {
            JobStatus::SucceededWithWarnings => StepStatus::Warning,
            JobStatus::Succeeded => StepStatus::Success,
            JobStatus::Cancelled => StepStatus::Cancelled,
            JobStatus::RepositoryLocked => StepStatus::Retry,
            JobStatus::ConfigError => StepStatus::ConfigError,
            JobStatus::Failed | JobStatus::NotInitialized | JobStatus::Unspecified => {
                StepStatus::Failed
            }
        });
        let result = JobResult {
            job_id: job_id.clone(),
            status: status as i32,
            steps_total,
            steps_succeeded: succeeded,
            steps_failed: failed,
            outcome: outcome as i32,
            error: job_error(outcome),
            steps: step_results.iter().map(compact).collect(),
            log_tail: log.tail(),
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
        self.release(&job_id);
        // The step results and the log went out while the job ran. The stored copy goes out at
        // every connection until the entry expires (ADR-0020, FR-AG-12).
        let _ = self
            .send(job_result_message(JobResult {
                steps: Vec::new(),
                log_tail: Vec::new(),
                ..result
            }))
            .await;
    }

    /// True when the step is a restore whose target the agent must refuse (FR-RS-03, ADR-0035).
    fn restore_denied(&self, step: &Step) -> bool {
        let Some(step::Kind::Restore(restore)) = step.kind.as_ref() else {
            return false;
        };
        match self.restore.check(&restore.target) {
            Ok(()) => false,
            Err(reason) => {
                tracing::warn!(%reason, "a restore target is refused");
                true
            }
        }
    }

    /// Sends the result of one step through the current stream (FR-AG-09) and returns it.
    async fn send_step_result(
        &self,
        job_id: &str,
        step: &Step,
        outcome: &StepOutcome,
        started_at: Timestamp,
        finished_at: Timestamp,
    ) -> StepResult {
        let result = StepResult {
            job_id: job_id.to_owned(),
            step_id: step.step_id.clone(),
            status: outcome.status as i32,
            exit_code: outcome.exit_code,
            started_at: Some(started_at),
            finished_at: Some(finished_at),
            error: outcome.error.map(StepError::detail),
            possible_stale_lock: outcome.possible_stale_lock,
            output: outcome.output.clone(),
        };
        let _ = self
            .send(AgentMessage {
                payload: Some(agent_message::Payload::StepResult(result.clone())),
            })
            .await;
        result
    }
}

/// The step result for the journal: the large outputs stay out (AC-PR-07).
///
/// The journal keeps the small summaries of a backup, a check, a restore and the stats. The list of
/// snapshots, a page of files and the forget result can be large, and the control plane asks for
/// them again when it needs them.
fn compact(result: &StepResult) -> StepResult {
    let mut result = result.clone();
    if matches!(
        result.output,
        Some(
            step_result::Output::SnapshotList(_)
                | step_result::Output::LsPage(_)
                | step_result::Output::ForgetResult(_)
        )
    ) {
        result.output = None;
    }
    result
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

/// The step kinds that the agent runs (FR-PR-02). The `hook` step belongs to the hook epic.
fn is_supported(kind: &step::Kind) -> bool {
    !matches!(kind, step::Kind::Hook(_))
}

/// True for a job that only lists snapshots or the files of one (ADR-0035).
fn is_browse(job: &restichub_proto::agent::v1::Job) -> bool {
    !job.steps.is_empty()
        && job.steps.iter().all(|step| {
            matches!(
                step.kind,
                Some(step::Kind::Snapshots(_) | step::Kind::Ls(_))
            )
        })
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

/// What the steps of one job did, as input of [`job_outcome`].
#[allow(clippy::struct_excessive_bools)]
#[derive(Default)]
struct Progress {
    cancelled: bool,
    /// The exit code of the step that stopped the job, outside the missing-repository probe.
    stopped_code: Option<i32>,
    probe_missing: bool,
    had_init: bool,
    init_ran: bool,
    init_succeeded: bool,
    init_invalid: bool,
    /// Exit code 3 of a step: restic finished but could not read some files.
    warned: bool,
}

/// Maps the step results to the typed outcome of a job (FR-AG-09, AC-RP-01).
///
/// The init chain: the agent runs init only after a `test_access` of the same repository exited with
/// code 10 (a missing repository). A successful init after that probe is a successful job. A
/// test_access that failed with any other code (1, 12, a timeout, access denied) stops the job,
/// and a standalone test_access with exit code 10 is the normal `NOT_INITIALIZED` result. An init
/// without a matching test_access is an invalid sequence and fails the job. Exit code 11 gives
/// `REPOSITORY_LOCKED` and exit code 3 gives `SUCCEEDED_WITH_WARNINGS`, so neither hides behind a
/// plain failure or a plain success.
fn job_outcome(progress: &Progress) -> JobStatus {
    if progress.cancelled {
        return JobStatus::Cancelled;
    }
    if let Some(code) = progress.stopped_code {
        // restic exit code 12 is a wrong password or wrong credentials (AC-RP-01).
        return match code {
            12 => JobStatus::ConfigError,
            11 => JobStatus::RepositoryLocked,
            _ => JobStatus::Failed,
        };
    }
    if progress.init_invalid {
        return JobStatus::Failed;
    }
    if progress.had_init && progress.init_ran {
        return if progress.init_succeeded {
            succeeded(progress)
        } else {
            JobStatus::Failed
        };
    }
    if !progress.had_init && progress.probe_missing {
        return JobStatus::NotInitialized;
    }
    succeeded(progress)
}

fn succeeded(progress: &Progress) -> JobStatus {
    if progress.warned {
        JobStatus::SucceededWithWarnings
    } else {
        JobStatus::Succeeded
    }
}

/// The stable code that explains a failed or cancelled outcome (NFR-DOC-04). It holds no secret.
fn job_error(outcome: JobStatus) -> Option<ErrorDetail> {
    let code = match outcome {
        JobStatus::ConfigError => "wrong_password",
        JobStatus::NotInitialized => "repository_missing",
        JobStatus::Failed => "step_failed",
        JobStatus::Cancelled => "cancelled",
        JobStatus::RepositoryLocked => "repository_locked",
        JobStatus::SucceededWithWarnings => "succeeded_with_warnings",
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
mod tests;
