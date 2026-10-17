//! The agent-side execution of the jobs: repository init and test-access, backup, check,
//! snapshots, ls, restore, forget, prune, unlock and stats (FR-PR-02, FR-PR-03, FR-PR-06, FR-RP-02, FR-AG-08,
//! FR-AG-09, FR-AG-10, ADR-0020, ADR-0026).
//!
//! The control plane delivers a signed job on the stream. The agent verifies the signature with the
//! key ring and checks the addressee, the tenant and the time rules ([`verify_job`]), validates the
//! steps ([`ValidatedJob`], FR-PR-01) and consults the job journal (ADR-0020). It then acknowledges
//! the job (FR-PR-06) and runs it off the stream task, so the heartbeat and the key updates keep
//! flowing. A job that the journal already completed is not run again; the stored result is re-sent,
//! also when the host changed the trust mode since. A job with a script step that a restart
//! interrupted is not run again either: it gets a final result with the error `interrupted`
//! (ADR-0037).
//!
//! The agent sends every result through the stream that is open at that moment, not through the
//! stream that delivered the job: a job outlives one connection. The result is durable in the
//! journal before the send. A send into the stream buffer does not prove that the control plane got
//! the result, so the agent sends every stored result at each connection until the journal entry
//! expires (ADR-0020, FR-AG-12). The control plane ignores a result for a terminal job.
//!
//! The agent starts no job on its own and holds no timer that starts one (ADR-0002).
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
use std::sync::atomic::AtomicU8;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Context as _, Result};
use prost_types::Timestamp;
use restichub_compiler::ValidatedJob;
use restichub_proto::agent::v1::{
    AgentMessage, JobAck, JobRejected, JobResult, JobStatus, RejectReason, ScriptInterpreter,
    ServerScriptsState, SignedJob, Step, StepResult, StepStatus, agent_message, step, step_result,
};
use restichub_proto::signing::{
    Admission, Expected, JobJournal as _, JournalState, KeyRing, Rejection, admit, verify_job,
};
use tokio::sync::mpsc;

use crate::journal::{FileJournal, JOURNAL_FILE, JournalSnapshot};
use crate::log_spool::LogSpool;
use crate::repository_roots::RepositoryAccess;
use crate::restic_store::Layout;
use crate::restore_roots::RestoreAccess;
use crate::scripts::ScriptStore;

mod command_text;
mod log;
mod outcome;
mod phases;
mod runner;
mod spooling;
mod watchdog;

use log::JobLog;
use outcome::{Progress, interrupted_error, job_error, job_outcome};
use phases::{PhaseMode, Phased, ScriptFailure, Sink};
use runner::{CancelHandle, StepError, StepOutcome};
use spooling::LogMode;
use watchdog::StepLimits;

/// The grace period between the graceful stop signal and the forced kill of the process tree
/// (FR-AG-10, AC-AG-06).
pub const STOP_GRACE: Duration = Duration::from_secs(30);

/// How long past the job deadline the cleanup scripts (`after`, `on_*` and `finally`) may run
/// (FR-HK-05).
pub const CLEANUP_GRACE: Duration = Duration::from_secs(10 * 60);

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

/// What the journal and the running jobs already know about a redelivered job (ADR-0020).
enum Known {
    Stored(JobResult),
    Running,
}

/// The running jobs of one agent and the journal that records them (ADR-0020).
#[derive(Clone)]
pub struct Jobs {
    program: PathBuf,
    expected_sha256: String,
    /// A job reads the hash from the protected file at every spawn, so a `restic update` takes
    /// effect without a restart (FR-AG-11, threat T20).
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
    /// How long past the job deadline the cleanup scripts may run (FR-HK-05).
    cleanup_grace: Duration,
    /// The roots where a restore may write and the files of the agent that it may never touch
    /// (FR-RS-03, ADR-0035).
    restore: Arc<RestoreAccess>,
    /// Where a local backend may lie on a Linux host (ADR-0044).
    repository: Arc<RepositoryAccess>,
    /// The agent reads it again at each job (FR-HK-08).
    scripts: Arc<ScriptStore>,
    /// The log chunks that the control plane did not acknowledge yet (FR-LG-07).
    log_spool: Arc<LogSpool>,
    /// The [`LogMode`] of the open stream, as a code.
    log_mode: Arc<AtomicU8>,
    /// Wakes the drain task when a chunk waits in the spool.
    log_wake: Arc<tokio::sync::Notify>,
    log_drain: Arc<Mutex<Option<tokio::task::AbortHandle>>>,
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
        crate::service::warn_about_read_only_roots(restore.roots());
        let repository = RepositoryAccess::for_host(dir, &cache, &program);
        crate::service::warn_about_read_only_roots(repository.roots());
        let scripts = ScriptStore::for_config_dir(dir, &cache);
        scripts.clean_leftovers();
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
            cleanup_grace: CLEANUP_GRACE,
            restore: Arc::new(restore),
            repository: Arc::new(repository),
            scripts: Arc::new(scripts),
            log_spool: Arc::new(LogSpool::open(dir, LogSpool::max_bytes_from_env())),
            log_mode: Arc::new(AtomicU8::new(LogMode::Direct as u8)),
            log_wake: Arc::new(tokio::sync::Notify::new()),
            log_drain: Arc::new(Mutex::new(None)),
        })
    }

    /// Replaces the log spool, for a test that needs a small cap (FR-LG-07).
    #[cfg(feature = "test-util")]
    #[must_use]
    pub fn with_log_spool(mut self, spool: LogSpool) -> Self {
        self.log_spool = Arc::new(spool);
        self
    }

    /// Replaces the place and the trust rule of the server scripts setting (FR-HK-08).
    #[must_use]
    pub fn with_script_store(mut self, store: ScriptStore) -> Self {
        store.clean_leftovers();
        self.scripts = Arc::new(store);
        self
    }

    /// The trust mode and the interpreters for `AgentHello` (FR-HK-08).
    pub async fn script_report(&self) -> (ServerScriptsState, Vec<ScriptInterpreter>) {
        let store = Arc::clone(&self.scripts);
        match tokio::task::spawn_blocking(move || store.report()).await {
            Ok(report) => report,
            Err(error) => {
                tracing::warn!(%error, "the server scripts check stopped");
                (ServerScriptsState::Unspecified, Vec::new())
            }
        }
    }

    /// Reads the expected hash from the protected record before every spawn (ADR-0038).
    #[must_use]
    pub fn with_record(mut self, layout: Layout) -> Self {
        self.record = Some(Arc::new(layout));
        self
    }

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

    /// Replaces the roots of the local repositories (ADR-0044).
    #[must_use]
    pub fn with_repository_access(mut self, access: RepositoryAccess) -> Self {
        self.repository = Arc::new(access);
        self
    }

    /// Roots for `AgentHello`, so the control plane can suggest a target (FR-RS-03).
    #[must_use]
    pub fn restore_roots(&self) -> Vec<String> {
        self.restore.root_strings()
    }

    /// Identifiers of the jobs that run now or whose stored result is not yet expired, for
    /// `AgentHello` (ADR-0039). The agent re-sends a stored result at each connection, so the
    /// control plane must not fail such a job as lost.
    #[must_use]
    pub fn held_job_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = lock(&self.running).keys().cloned().collect();
        ids.extend(
            lock(&self.journal)
                .stored_results()
                .into_iter()
                .map(|result| result.job_id),
        );
        ids.truncate(MAX_HELD_JOB_IDS);
        ids
    }

    /// Sets the grace period between the graceful stop and the forced kill (FR-AG-10).
    #[must_use]
    pub fn with_stop_grace(mut self, grace: Duration) -> Self {
        self.stop_grace = grace;
        self
    }

    /// Sets how long past the job deadline the cleanup scripts may run (FR-HK-05).
    #[must_use]
    pub fn with_cleanup_grace(mut self, grace: Duration) -> Self {
        self.cleanup_grace = grace;
        self
    }

    /// Sets the stream that carries the results (ADR-0020).
    ///
    /// The connect loop calls it after the stream opens, before it sends the stored results. Log
    /// chunks wait in the spool until [`Self::greeted`] tells what the control plane accepts, so
    /// they keep their order across the greeting (FR-LG-07).
    pub fn attach(&self, tx: &mpsc::Sender<AgentMessage>) {
        self.set_log_mode(LogMode::Pending);
        // Nothing went out on this stream yet, whatever the stream before it got.
        self.log_spool.rewind_all();
        *lock(&self.connection) = Some(tx.clone());
        self.start_log_drain(tx.clone());
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
        // The journal answers a redelivery before the checks of the current host state: a job that
        // finished is not judged again (ADR-0020).
        match self.known(&validated).await {
            Some(Known::Stored(result)) => {
                self.send_result(result).await;
                return;
            }
            Some(Known::Running) => {
                let _ = self.send(ack(job_id)).await;
                return;
            }
            None => {}
        }
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
        // The sandbox of the service would stop restic with a read-only error (ADR-0044).
        if let Some((location, reason)) = self.repository.denied(validated.job()) {
            tracing::warn!(
                %reason,
                location,
                file = %crate::repository_roots::roots_file().display(),
                "a local repository is refused; add its root to the file and run install-service"
            );
            let _ = self
                .send(rejected(job_id, RejectReason::RepositoryPathDenied))
                .await;
            return;
        }
        if let Some(reason) = self.script_refusal(&validated).await {
            let _ = self.send(rejected(job_id, reason)).await;
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

    /// The answer that the journal already holds for a redelivered job, or `None` for a job that
    /// needs the checks and a run (ADR-0020).
    ///
    /// A job that the journal holds as accepted, without a result, and that runs in no task of this
    /// process was interrupted by a restart. The agent runs a job of restic steps again, but not a
    /// job with a script step: a program may have run, and it may not run twice. It stores
    /// and answers a final result instead (ADR-0037).
    async fn known(&self, validated: &ValidatedJob) -> Option<Known> {
        let job = validated.job();
        let snapshot = {
            let mut journal = lock(&self.journal);
            let running = lock(&self.running);
            match journal.lookup(&job.job_id) {
                JournalState::Completed(result) => return Some(Known::Stored(result)),
                _ if running.contains_key(&job.job_id) => return Some(Known::Running),
                JournalState::InProgress if runs_programs(job) => {
                    let result = interrupted_result(job);
                    journal.record_completed(&job.job_id, result.clone());
                    (result, journal.snapshot())
                }
                _ => return None,
            }
        };
        let (result, snapshot) = snapshot;
        if let Err(error) = self.write_journal(snapshot).await {
            tracing::warn!(%error, "cannot write the job journal");
        }
        Some(Known::Stored(result))
    }

    /// The reason to refuse a job whose script the host does not trust or whose values break a
    /// rule, or `None` (FR-HK-08, AC-HK-07).
    async fn script_refusal(&self, validated: &ValidatedJob) -> Option<RejectReason> {
        let scripts: Vec<_> = validated
            .job()
            .steps
            .iter()
            .filter_map(|step| match step.kind.as_ref() {
                Some(step::Kind::Script(script)) => Some(script.clone()),
                _ => None,
            })
            .collect();
        if scripts.is_empty() {
            return None;
        }
        let store = Arc::clone(&self.scripts);
        let checked = tokio::task::spawn_blocking(move || {
            scripts
                .iter()
                .find_map(|script| store.check(script).err())
                .map(|refusal| refusal.reject_reason())
        })
        .await;
        checked.unwrap_or(Some(RejectReason::Internal))
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
        // The spool resends the whole log in order, so the tail would only come first and
        // reorder the lines (FR-LG-07).
        if self.log_spool.holds(&result.job_id) {
            result.log_tail.clear();
        }
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
        let phased = Phased::new(&job.steps);
        let deadline = job_deadline;
        // The repository of the test_access step. The init step runs only when its own repository
        // matches (FR-RP-02, AC-RP-01).
        let mut probe_repository: Option<String> = None;

        // The `before` scripts run first. A failed one stops the sequence (AC-HK-13).
        let before = self
            .run_script_steps(
                &phased.before,
                PhaseMode {
                    stop_on_failure: true,
                    honor_cancel: true,
                    honor_deadline: true,
                    clamp_to_deadline: false,
                },
                &cancel,
                deadline,
                &mut Sink {
                    job_id: &job_id,
                    results: &mut step_results,
                    succeeded: &mut succeeded,
                    failed: &mut failed,
                    log: &mut log,
                },
            )
            .await;
        progress.cancelled |= before.cancelled;
        progress.warned |= before.warned;
        let mut script_failure: Option<ScriptFailure> = before.failure;
        let before_ok = script_failure.is_none() && !progress.cancelled;
        if !before_ok {
            let mut sink = Sink {
                job_id: &job_id,
                results: &mut step_results,
                succeeded: &mut succeeded,
                failed: &mut failed,
                log: &mut log,
            };
            self.skip_steps(&phased.main, &mut sink).await;
        }

        let main = if before_ok {
            phased.main.as_slice()
        } else {
            &[]
        };
        for step in main.iter().copied() {
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
            let mut max = step_timeout(step.timeout_seconds, job.timeout_seconds);
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
                max = Some(max.map_or(remaining, |max| max.min(remaining)));
            }
            if is_init {
                progress.init_ran = true;
            }
            let outcome = self
                .run_step(
                    &validated,
                    step,
                    &cancel,
                    StepLimits::for_step(step, max, job.stall_timeout_seconds),
                    &mut log,
                )
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
            if matches!(
                outcome.error,
                Some(StepError::Stalled | StepError::MaxDuration)
            ) {
                progress.limit = outcome.error;
            }
            if outcome.error == Some(StepError::NotStarted) {
                progress.restic_not_started = true;
            }
            if outcome.error == Some(StepError::ReadOnly) {
                progress.repository_read_only = true;
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

        // The remaining phases (FR-HK-05). A cancel leaves the `finally` scripts only.
        macro_rules! sink {
            () => {
                &mut Sink {
                    job_id: &job_id,
                    results: &mut step_results,
                    succeeded: &mut succeeded,
                    failed: &mut failed,
                    log: &mut log,
                }
            };
        }
        let cleanup = PhaseMode {
            stop_on_failure: false,
            honor_cancel: true,
            honor_deadline: false,
            clamp_to_deadline: true,
        };
        // One budget for the whole cleanup of the job, not one grace for each step (FR-HK-05).
        let cleanup_deadline = deadline.map(|deadline| deadline + self.cleanup_grace);
        let main_ok = before_ok
            && progress.stopped_code.is_none()
            && !progress.init_invalid
            && !progress.cancelled;
        if before_ok && !progress.cancelled {
            let report = self
                .run_script_steps(&phased.after, cleanup, &cancel, cleanup_deadline, sink!())
                .await;
            progress.cancelled |= report.cancelled;
            progress.warned |= report.warned;
            script_failure = script_failure.or(report.failure);
        } else {
            self.skip_steps(&phased.after, sink!()).await;
        }
        let succeeded_so_far = main_ok && script_failure.is_none() && !progress.cancelled;
        let (run_success, run_error) = if progress.cancelled {
            (false, false)
        } else if succeeded_so_far {
            (true, false)
        } else {
            (false, true)
        };
        for (steps, run) in [
            (&phased.on_success, run_success),
            (&phased.on_error, run_error),
        ] {
            if run {
                let report = self
                    .run_script_steps(steps, cleanup, &cancel, cleanup_deadline, sink!())
                    .await;
                progress.cancelled |= report.cancelled;
                progress.warned |= report.warned;
                script_failure = script_failure.or(report.failure);
            } else {
                self.skip_steps(steps, sink!()).await;
            }
        }
        let report = self
            .run_script_steps(
                &phased.finally,
                PhaseMode {
                    honor_cancel: false,
                    ..cleanup
                },
                &cancel,
                cleanup_deadline,
                sink!(),
            )
            .await;
        progress.cancelled |= report.cancelled;
        progress.warned |= report.warned;
        script_failure = script_failure.or(report.failure);
        progress.script_failure = script_failure;

        self.send_log(&mut log).await;
        self.close_log(&mut log);
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
            error: job_error(outcome, &progress),
            steps: step_results.iter().map(compact).collect(),
            log_tail: log.tail(),
        };
        // The result is durable before the job leaves `running`, so a redelivery in between gets the
        // acknowledgement and a later one gets the stored result, never a second run (ADR-0020).
        let snapshot = {
            let mut journal = lock(&self.journal);
            journal.record_completed(&job_id, result.clone());
            let now = now_timestamp();
            journal.keep_until(
                &job_id,
                Timestamp {
                    seconds: now.seconds + RESULT_RETENTION.as_secs() as i64,
                    nanos: 0,
                },
            );
            journal.prune(now);
            journal.snapshot()
        };
        if let Err(error) = self.write_journal(snapshot).await {
            tracing::warn!(%error, "cannot write the job journal");
        }
        self.release(&job_id);
        // The result follows the last lines of the log. The wait is bounded and holds no lock of
        // the stream task, so a cancel and the pings do not wait for it.
        self.wait_log_sent(&job_id).await;
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
    ValidatedJob::new(job).map_err(|rejection| Rejection {
        job_id: rejection.job_id.or_else(|| Some(job_id.clone())),
        reason: rejection.reason,
    })
}

/// True for a job with a script step.
fn runs_programs(job: &restichub_proto::agent::v1::Job) -> bool {
    job.steps
        .iter()
        .any(|step| matches!(step.kind, Some(step::Kind::Script(_))))
}

/// A restart cut the job short, so no step result exists for it (ADR-0037).
fn interrupted_result(job: &restichub_proto::agent::v1::Job) -> JobResult {
    JobResult {
        job_id: job.job_id.clone(),
        status: StepStatus::Failed as i32,
        steps_total: u32::try_from(job.steps.len()).unwrap_or(u32::MAX),
        steps_succeeded: 0,
        steps_failed: 0,
        outcome: JobStatus::Failed as i32,
        error: Some(interrupted_error()),
        steps: Vec::new(),
        log_tail: Vec::new(),
    }
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

/// The maximum duration of one step: its own value, or the job value when the step gives none.
/// Zero in both means no limit (FR-AG-05, ADR-0039).
#[must_use]
fn step_timeout(step_seconds: u32, job_seconds: u32) -> Option<Duration> {
    let seconds = if step_seconds > 0 {
        step_seconds
    } else {
        job_seconds
    };
    (seconds > 0).then(|| Duration::from_secs(u64::from(seconds)))
}

/// How long the journal keeps the result of a finished job for the control plane to collect.
const RESULT_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

/// Bounds `AgentHello.held_job_ids`: the journal entries plus the running jobs.
const MAX_HELD_JOB_IDS: usize = 4096 + 64;

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
