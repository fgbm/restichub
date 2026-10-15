//! The delay between a progress message of an agent and the event in the stream (AC-UI-02).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Body;
use http_body_util::BodyExt;
use restichub_emulator::agent::ProgressSink;
use restichub_emulator::executor::{Executor, StepOutcome};
use restichub_proto::agent::v1::Step;

use queue_support::{Env, eventually, test_queue};

const REPORTS: usize = 40;
const BUDGET: Duration = Duration::from_secs(2);

/// Reports `REPORTS` progress messages and keeps the time of each send.
struct Reporting {
    sent: Arc<Mutex<Vec<Instant>>>,
}

#[async_trait]
impl Executor for Reporting {
    async fn run_step(&self, _step: &Step, progress: &ProgressSink) -> StepOutcome {
        for index in 0..REPORTS {
            self.sent
                .lock()
                .expect("the lock is not poisoned")
                .push(Instant::now());
            progress
                .report(index as f64 / REPORTS as f64, index as f64)
                .await;
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        StepOutcome::success()
    }
}

// The stream never ends on its own, so the reader stops at the first idle second after `done`.
async fn arrivals(mut body: Body, done: Arc<std::sync::atomic::AtomicBool>) -> Vec<Instant> {
    let mut times = Vec::new();
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(1), body.frame()).await;
        match frame {
            Ok(Some(Ok(frame))) => {
                if let Ok(data) = frame.into_data() {
                    let text = String::from_utf8_lossy(&data);
                    let now = Instant::now();
                    for _ in 0..text.matches("event: job.progress").count() {
                        times.push(now);
                    }
                }
            }
            Ok(_) => break,
            Err(_) => {
                if done.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
            }
        }
    }
    times
}

/// AC-UI-02: the p95 delay from the progress message of a real agent stream to the event in the
/// browser stream stays under two seconds.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ui_02_progress_reaches_the_stream_within_two_seconds_at_p95() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let sent = Arc::new(Mutex::new(Vec::new()));
    let emulator = env
        .start_emulator(
            "latency-host",
            Arc::new(Reporting {
                sent: Arc::clone(&sent),
            }),
        )
        .await;
    let repository = env.create_repository(emulator.id, "pw").await;
    let profile = env.create_profile(repository, serde_json::json!({})).await;
    let plan = env.create_plan(profile, emulator.id).await;

    let response = common::send_response(
        &env.app,
        common::bearer_request("GET", "/api/v1/events", &env.viewer, None),
    )
    .await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader = tokio::spawn(arrivals(response.into_body(), Arc::clone(&done)));

    let job = env.run_plan(plan).await;
    eventually(|| async {
        env.pass().await;
        matches!(env.job_status(job).await.as_str(), "succeeded" | "failed")
    })
    .await;
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let arrived = reader.await.expect("the reader task");
    let sent = sent.lock().expect("the lock is not poisoned").clone();
    assert_eq!(sent.len(), REPORTS, "the executor reported every step");

    // The first arrival at or after each send is the event of that send. The events of the
    // step result come later and never lower a delay.
    let mut delays: Vec<Duration> = sent
        .iter()
        .map(|at| {
            arrived
                .iter()
                .filter(|arrival| *arrival >= at)
                .map(|arrival| arrival.duration_since(*at))
                .min()
                .expect("an event follows every progress message")
        })
        .collect();
    delays.sort();
    let p95 = delays[(delays.len() * 95).div_ceil(100) - 1];
    assert!(p95 < BUDGET, "the p95 delay is {p95:?}");
}

/// AC-UI-02: a host that connects and a host that leaves each give an event to the tenant stream.
#[tokio::test(flavor = "multi_thread")]
async fn ac_ui_02_a_host_that_connects_and_leaves_gives_events() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let response = common::send_response(
        &env.app,
        common::bearer_request("GET", "/api/v1/events", &env.viewer, None),
    )
    .await;
    let mut body = response.into_body();
    let emulator = env
        .start_emulator(
            "event-host",
            Arc::new(Reporting {
                sent: Arc::default(),
            }),
        )
        .await;

    let mut seen = String::new();
    while !seen.contains("event: host.online") {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("no event in time")
            .expect("the stream ended")
            .expect("the stream failed");
        seen.push_str(&String::from_utf8_lossy(&frame.into_data().expect("data")));
    }
    assert!(seen.contains(&emulator.id.to_string()));

    assert!(env.state.agent_registry().close_by_agent(emulator.id));
    while !seen.contains("event: host.offline") {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("no event in time")
            .expect("the stream ended")
            .expect("the stream failed");
        seen.push_str(&String::from_utf8_lossy(&frame.into_data().expect("data")));
    }
}
