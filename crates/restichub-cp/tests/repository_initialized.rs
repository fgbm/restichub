//! The initialized flag follows the location fingerprint of the job (FR-RP-08, ADR-0021).
//!
//! A job without a fingerprint makes no claim about the location of the repository. It neither sets
//! the flag nor reports that the location changed.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::io;
use std::sync::{Arc, Mutex, Once, OnceLock};

use axum::http::StatusCode;
use queue_support::{Env, FakeAgent, decode_job_id, id_of, test_queue};
use serde_json::json;
use uuid::Uuid;

/// The warning of `apply_initialized` about a location that a job no longer matches.
const LOCATION_CHANGED: &str = "the repository location changed";

static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();

fn logs() -> &'static Arc<Mutex<Vec<u8>>> {
    LOGS.get_or_init(|| Arc::new(Mutex::new(Vec::new())))
}

struct BufferWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for BufferWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut guard = self
            .0
            .lock()
            .map_err(|_| io::Error::other("the log buffer is poisoned"))?;
        guard.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferWriter {
    type Writer = BufferWriter;

    fn make_writer(&'a self) -> Self::Writer {
        BufferWriter(self.0.clone())
    }
}

fn install_log_capture() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(BufferWriter(logs().clone()))
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("cannot install the subscriber");
    });
}

fn captured_logs() -> String {
    let guard = logs().lock().unwrap_or_else(|error| error.into_inner());
    String::from_utf8_lossy(&guard).into_owned()
}

async fn run(env: &Env, agent: &mut FakeAgent, job: Uuid) {
    env.pass().await;
    let id = decode_job_id(&agent.job().await);
    assert_eq!(id, job.to_string(), "the queued job reaches the agent");
    agent.start(env, &id).await;
    agent.finish(env, &id).await;
}

async fn test_access(env: &Env, repository: Uuid) -> Uuid {
    let (status, body) = env
        .call(
            &env.admin,
            "POST",
            &format!("/api/v1/repositories/{repository}/test-access"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    id_of(&body)
}

async fn initialized(env: &Env, repository: Uuid) -> bool {
    let mut tx = env.scoped().await;
    let (flag,): (bool,) = sqlx::query_as("SELECT initialized FROM repositories WHERE id = $1")
        .bind(repository)
        .fetch_one(&mut *tx)
        .await
        .expect("cannot read the repository");
    tx.commit().await.expect("commit");
    flag
}

async fn set_initialized(env: &Env, repository: Uuid, flag: bool) {
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE repositories SET initialized = $2 WHERE id = $1")
        .bind(repository)
        .bind(flag)
        .execute(&mut *tx)
        .await
        .expect("cannot set the flag");
    tx.commit().await.expect("commit");
}

/// FR-RP-08: a plan backup and a test_access of the same unchanged location behave as before, and a
/// location that changes during a job still gives the warning.
#[tokio::test]
async fn fr_rp_08_a_job_without_a_fingerprint_reports_no_location_change() {
    let Some(db) = common::test_db().await else {
        return;
    };
    install_log_capture();
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("plan-host").await;
    let backup_repository = env.create_repository(agent.id, "pw").await;
    // Only an init sets the flag; the test seeds the state that the init leaves.
    set_initialized(&env, backup_repository, true).await;

    // A plan backup carries no fingerprint, so it makes no claim about the location.
    let backup = env.run_fresh_plan(backup_repository, agent.id).await;
    run(&env, &mut agent, backup).await;
    assert!(
        !captured_logs().contains(LOCATION_CHANGED),
        "a job without a fingerprint reports no location change"
    );
    assert!(
        initialized(&env, backup_repository).await,
        "a plan backup changes no initialized flag"
    );

    // A test_access carries the fingerprint of the unchanged location, so it sets the flag silently.
    let repository = env.create_repository(agent.id, "pw-2").await;
    let job = test_access(&env, repository).await;
    run(&env, &mut agent, job).await;
    assert!(
        initialized(&env, repository).await,
        "a test_access sets the flag"
    );
    assert!(
        !captured_logs().contains(LOCATION_CHANGED),
        "an unchanged location gives no warning"
    );

    // A location that changes between the job and its result still gives the warning.
    let job = test_access(&env, repository).await;
    env.pass().await;
    let id = decode_job_id(&agent.job().await);
    assert_eq!(id, job.to_string(), "the queued job reaches the agent");
    agent.start(&env, &id).await;
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE repositories SET local_path = '/srv/moved' WHERE id = $1")
        .bind(repository)
        .execute(&mut *tx)
        .await
        .expect("cannot move the repository");
    tx.commit().await.expect("commit");
    agent.finish(&env, &id).await;
    assert!(
        captured_logs().contains(LOCATION_CHANGED),
        "a changed location still gives the warning"
    );
    env.db.cleanup().await;
}
