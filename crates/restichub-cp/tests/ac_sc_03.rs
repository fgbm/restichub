//! AC-SC-03: a prune waits for the running backups of its repository, and the backups queued during
//! the prune wait for it (FR-SC-04). The test runs the real dispatcher loop and an emulated agent
//! on the maintenance job that the control plane builds (FR-RP-08).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use restichub_emulator::agent::ProgressSink;
use restichub_emulator::executor::{Executor, StepOutcome};
use restichub_proto::agent::v1::{Step, step};
use serde_json::json;
use tokio::sync::watch;

use queue_support::{Env, eventually, test_queue};

/// Logs the start of each step. A backup waits for the first gate and a maintenance step for the
/// second one, so the test controls the order.
struct Gated {
    log: Arc<Mutex<Vec<&'static str>>>,
    backups: watch::Receiver<bool>,
    maintenance: watch::Receiver<bool>,
}

impl Gated {
    async fn wait(mut gate: watch::Receiver<bool>) {
        while !*gate.borrow() {
            if gate.changed().await.is_err() {
                return;
            }
        }
    }
}

#[async_trait]
impl Executor for Gated {
    async fn run_step(&self, step: &Step, _progress: &ProgressSink) -> StepOutcome {
        let (name, gate) = match step.kind {
            Some(step::Kind::Backup(_)) => ("backup", self.backups.clone()),
            Some(step::Kind::Forget(_)) => ("forget", self.maintenance.clone()),
            Some(step::Kind::Prune(_)) => ("prune", self.maintenance.clone()),
            _ => ("other", self.maintenance.clone()),
        };
        self.log.lock().expect("the step log").push(name);
        Self::wait(gate).await;
        StepOutcome::success()
    }
}

fn log_of(log: &Arc<Mutex<Vec<&'static str>>>) -> Vec<&'static str> {
    log.lock().expect("the step log").clone()
}

#[tokio::test]
async fn ac_sc_03_a_prune_waits_for_three_backups_and_the_later_backups_wait_for_it() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let log = Arc::new(Mutex::new(Vec::new()));
    let (backup_gate, backups) = watch::channel(false);
    let (maintenance_gate, maintenance) = watch::channel(false);
    let emulator = env
        .start_emulator(
            "ac-sc-03-host",
            Arc::new(Gated {
                log: Arc::clone(&log),
                backups,
                maintenance,
            }),
        )
        .await;
    let repository = env.create_repository(emulator.id, "pw").await;
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE repositories SET initialized = true WHERE id = $1")
        .bind(repository)
        .execute(&mut *tx)
        .await
        .expect("cannot mark the repository initialized");
    tx.commit().await.expect("cannot commit the mark");
    let retention_profile = env
        .create_profile(repository, json!({"retention": {"keep_last": 2}}))
        .await;
    env.create_plan(retention_profile, emulator.id).await;
    let dispatcher = restichub_cp::queue::spawn(env.state.clone(), env.state.auth().clock.clone());

    for _ in 0..3 {
        env.run_fresh_plan(repository, emulator.id).await;
    }
    eventually(|| async { log_of(&log) == ["backup"; 3] }).await;

    let (status, prune) = env
        .call(
            &env.operator,
            "POST",
            &format!("/api/v1/repositories/{repository}/maintenance/run"),
            Some(json!({"task": "retention"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{prune}");
    for _ in 0..2 {
        env.run_fresh_plan(repository, emulator.id).await;
    }
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        log_of(&log),
        ["backup"; 3],
        "neither the prune nor the later backups start while three backups run"
    );

    backup_gate.send(true).expect("open the backup gate");
    eventually(|| async { log_of(&log).contains(&"forget") }).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        log_of(&log),
        ["backup", "backup", "backup", "forget"],
        "the backups queued during the prune wait for it"
    );

    maintenance_gate
        .send(true)
        .expect("open the maintenance gate");
    eventually(|| async { log_of(&log).len() == 7 }).await;
    assert_eq!(
        log_of(&log),
        [
            "backup", "backup", "backup", "forget", "prune", "backup", "backup"
        ]
    );
    dispatcher.abort();
    emulator.stop().await;
    env.db.cleanup().await;
}
