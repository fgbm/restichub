//! AC-SC-09: a manual run for an online agent reaches the start of restic within three seconds
//! (95th percentile of 100 runs). The test uses the real dispatcher loop and the emulated agent
//! (FR-SC-09, NFR-PERF-02).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use restichub_emulator::agent::ProgressSink;
use restichub_emulator::executor::{Executor, StepOutcome};
use restichub_proto::agent::v1::Step;
use serde_json::json;

use queue_support::{Env, eventually, test_queue};

/// Records the instant at which each step starts: the moment of the restic start.
struct Recording {
    starts: Arc<Mutex<Vec<Instant>>>,
}

#[async_trait]
impl Executor for Recording {
    async fn run_step(&self, _step: &Step, _progress: &ProgressSink) -> StepOutcome {
        self.starts
            .lock()
            .expect("the start log")
            .push(Instant::now());
        StepOutcome::success()
    }
}

#[tokio::test]
async fn ac_sc_09_the_manual_run_reaches_restic_within_three_seconds_at_p95() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let starts = Arc::new(Mutex::new(Vec::new()));
    let emulator = env
        .start_emulator(
            "latency-host",
            Arc::new(Recording {
                starts: Arc::clone(&starts),
            }),
        )
        .await;
    let repository = env.create_repository(emulator.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, emulator.id, repository).await;
    let dispatcher = restichub_cp::queue::spawn(env.state.clone(), env.state.auth().clock.clone());

    let mut latencies = Vec::new();
    for run in 0..100 {
        let click = Instant::now();
        let job = env.run_plan(plan).await;
        eventually(|| async { starts.lock().expect("the start log").len() > run }).await;
        let started = starts.lock().expect("the start log")[run];
        latencies.push(started.duration_since(click));
        eventually(|| async { env.job_status(job).await == "succeeded" }).await;
    }
    dispatcher.abort();
    emulator.stop().await;

    latencies.sort();
    let p95 = latencies[94];
    assert!(
        p95 <= Duration::from_secs(3),
        "the 95th percentile is {p95:?}, the limit is 3 s"
    );
    env.db.cleanup().await;
}
