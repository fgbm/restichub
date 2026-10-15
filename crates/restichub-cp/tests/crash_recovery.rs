//! Crash recovery of the real control plane binary (AC-NFR-02, NFR-REL-01).
//!
//! The test starts `restichub-cp serve` on a test database and connects emulated agents to its
//! agent listener. Fifty backup jobs run on ten agents. The test kills the process with SIGKILL
//! while the jobs run, starts it again and waits for the final status of every job. The emulated
//! agents stay up, as the agents of a real host do when the control plane restarts.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use restichub_db::test_util::TestDatabase;
use restichub_emulator::agent::ProgressSink;
use restichub_emulator::executor::{Executor, StepOutcome};
use restichub_proto::agent::v1::{Step, step};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use queue_support::{Env, test_queue};

const AGENTS: usize = 10;
const JOBS_PER_AGENT: usize = 5;
/// Longer than the time between the first dispatch and the kill, so every job is in flight.
const BACKUP_SECONDS: u64 = 8;
const FINAL_STATUSES: [&str; 4] = ["succeeded", "failed", "expired", "cancelled"];

/// Finishes each backup step after a pause and counts the steps that started.
struct Counting {
    backups: Arc<AtomicUsize>,
}

#[async_trait]
impl Executor for Counting {
    async fn run_step(&self, step: &Step, _progress: &ProgressSink) -> StepOutcome {
        if matches!(step.kind, Some(step::Kind::Backup(_))) {
            self.backups.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_secs(BACKUP_SECONDS)).await;
        }
        StepOutcome::success()
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("cannot bind a free port")
        .local_addr()
        .expect("the local address")
        .port()
}

/// A login that the binary uses; the test database only exposes the password of its own login.
async fn create_login(db: &TestDatabase, role: &str, password: &str) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(db.admin_options())
        .await
        .expect("cannot connect as the administrator");
    for statement in [
        format!("CREATE ROLE {role} LOGIN PASSWORD '{password}' NOSUPERUSER NOBYPASSRLS"),
        format!("GRANT restichub_app TO {role}"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .expect("cannot prepare the login");
    }
    pool.close().await;
}

async fn drop_login(db: &TestDatabase, role: &str) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(db.admin_options())
        .await
        .expect("cannot connect as the administrator");
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP ROLE IF EXISTS {role}")))
        .execute(&pool)
        .await
        .expect("cannot drop the login");
    pool.close().await;
}

struct Process {
    child: Child,
}

impl Process {
    fn start(binary: &str, log: &Path, environment: &[(&str, String)]) -> Self {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .expect("cannot open the log");
        let child = Command::new(binary)
            .arg("serve")
            .env_clear()
            .envs(environment.iter().map(|(name, value)| (*name, value)))
            .stdin(Stdio::null())
            .stdout(log.try_clone().expect("cannot clone the log handle"))
            .stderr(log)
            .spawn()
            .expect("cannot start the control plane");
        Self { child }
    }

    /// SIGKILL on Unix: the process gets no chance to clean up.
    fn kill(&mut self) {
        self.child.kill().expect("cannot kill the control plane");
        self.child.wait().expect("cannot reap the control plane");
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn wait_ready(port: u16, log: &Path) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!(
        "the control plane did not listen: {}",
        std::fs::read_to_string(log).unwrap_or_default()
    );
}

async fn statuses(env: &Env, jobs: &[Uuid]) -> Vec<String> {
    let mut statuses = Vec::with_capacity(jobs.len());
    for job in jobs {
        statuses.push(env.job_status(*job).await);
    }
    statuses
}

/// AC-NFR-02: after a SIGKILL of the control plane during 50 jobs, every job gets a final status
/// and no job runs twice.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn ac_nfr_02_a_sigkill_of_the_control_plane_loses_and_repeats_no_job() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let temp = std::env::temp_dir().join(format!("restichub-crash-{}", Uuid::now_v7()));
    std::fs::create_dir_all(&temp).expect("cannot create the temp directory");
    let log = temp.join("cp.log");

    let role = format!("restichub_crash_{}", Uuid::now_v7().simple());
    let password = Uuid::now_v7().simple().to_string();
    create_login(&env.db, &role, &password).await;
    let options = env.db.runtime_options();
    let http_port = free_port();
    let agent_port = free_port();
    let environment = vec![
        (
            "RESTICHUB_DATABASE_URL",
            format!(
                "postgres://{role}@{}:{}/{}",
                options.get_host(),
                options.get_port(),
                options.get_database().expect("the database name")
            ),
        ),
        ("RESTICHUB_DATABASE_PASSWORD", password.clone()),
        (
            "RESTICHUB_KEK",
            base64::engine::general_purpose::STANDARD.encode([7_u8; 32]),
        ),
        ("RESTICHUB_LISTEN_ADDR", format!("127.0.0.1:{http_port}")),
        ("RESTICHUB_AGENT_LISTEN", format!("127.0.0.1:{agent_port}")),
        (
            "RESTICHUB_AGENT_ENDPOINT",
            format!("https://localhost:{agent_port}"),
        ),
        ("RESTICHUB_TENANCY_MODE", "multi".to_owned()),
        ("RESTICHUB_QUEUE_LEASE_SECS", "15".to_owned()),
        ("RESTICHUB_QUEUE_AGENT_LIMIT", JOBS_PER_AGENT.to_string()),
        (
            "RESTICHUB_QUEUE_REPOSITORY_LIMIT",
            JOBS_PER_AGENT.to_string(),
        ),
        ("RESTICHUB_QUEUE_GLOBAL_LIMIT", "100".to_owned()),
        ("RESTICHUB_SERVER_INSECURE_COOKIES", "true".to_owned()),
    ];
    let binary = env!("CARGO_BIN_EXE_restichub-cp");
    let mut process = Process::start(binary, &log, &environment);
    wait_ready(agent_port, &log).await;

    let backups = Arc::new(AtomicUsize::new(0));
    let executor: Arc<dyn Executor> = Arc::new(Counting {
        backups: Arc::clone(&backups),
    });
    let url = format!("https://127.0.0.1:{agent_port}");
    let mut emulators = Vec::new();
    let mut jobs = Vec::new();
    for index in 0..AGENTS {
        let identity = env.enroll_agent(&format!("crash-host-{index}")).await;
        let emulator = env
            .run_emulator(identity, Arc::clone(&executor), url.clone(), true)
            .await;
        let repository = env.create_repository(emulator.id, "pw").await;
        // The queue allows one waiting run for each plan, so each job has its own plan.
        for _ in 0..JOBS_PER_AGENT {
            let profile = env.create_profile(repository, serde_json::json!({})).await;
            let plan = env.create_plan(profile, emulator.id).await;
            jobs.push(env.run_plan(plan).await);
        }
        emulators.push(emulator);
    }
    assert_eq!(jobs.len(), AGENTS * JOBS_PER_AGENT);

    // Wait until the dispatcher has taken every job from the queue, then kill the process.
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let states = statuses(&env, &jobs).await;
        if states.iter().all(|state| state != "queued") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the dispatcher did not send every job: {states:?}\n{}",
            std::fs::read_to_string(&log).unwrap_or_default()
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let before = statuses(&env, &jobs).await;
    let in_flight = before
        .iter()
        .filter(|state| !FINAL_STATUSES.contains(&state.as_str()))
        .count();
    assert!(
        in_flight >= jobs.len() * 4 / 5,
        "the kill must hit jobs in flight: {before:?}"
    );
    process.kill();

    let mut process = Process::start(binary, &log, &environment);
    wait_ready(agent_port, &log).await;

    let deadline = Instant::now() + Duration::from_secs(150);
    loop {
        let states = statuses(&env, &jobs).await;
        if states
            .iter()
            .all(|state| FINAL_STATUSES.contains(&state.as_str()))
        {
            let succeeded = states.iter().filter(|state| *state == "succeeded").count();
            assert_eq!(succeeded, jobs.len(), "every job must succeed: {states:?}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "a job has no final status after the restart: {states:?}\n{}",
            std::fs::read_to_string(&log).unwrap_or_default()
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(
        backups.load(Ordering::SeqCst),
        jobs.len(),
        "each job must run its backup step exactly once"
    );

    process.kill();
    for emulator in emulators {
        emulator.stop().await;
    }
    drop_login(&env.db, &role).await;
    let _ = std::fs::remove_dir_all(&temp);
    env.db.cleanup().await;
}
