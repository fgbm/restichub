//! End-to-end tests of the emulated agent loop against the in-process test
//! control plane. The test helper lives in `common`.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{agent_backoff, collect, spawn_agents, test_executor};
use restichub_emulator::control::AgentEvent;
use restichub_emulator::executor::SimulatedProfile;
use restichub_emulator::identity::{KEY_ID, TestPki, signing_key};
use restichub_emulator::job::sample_job;
use restichub_proto::agent::v1::{RejectReason, StepStatus};
use restichub_proto::signing::sign_job;

const TIMEOUT: Duration = Duration::from_secs(60);

/// AC-ID-06: N agents connect over the real protocol, receive a signed job and
/// report a result.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ac_id_06_fifty_agents_connect_and_report_results() {
    let count = 50;
    let pki = TestPki::generate(count).expect("generate the test certificates");
    let plane = common::start(&pki, count).await;
    let mut events = plane.subscribe();

    let (shutdown, tasks) =
        spawn_agents(&pki, &plane.url(), count, test_executor(), agent_backoff());

    assert!(
        plane.wait_connected(count, TIMEOUT).await,
        "only {} of {count} agents connected",
        plane.connections()
    );

    for id in plane.agents() {
        let job = sample_job(&id, &format!("job-{id}"));
        assert!(plane.dispatch(&id, &job), "cannot dispatch a job to {id}");
    }

    let results = collect(&mut events, count, TIMEOUT).await;
    assert_eq!(
        results.len(),
        count,
        "the agents reported {} results",
        results.len()
    );
    for event in &results {
        assert!(
            matches!(event, AgentEvent::Result { status, .. } if *status == StepStatus::Success as i32),
            "unexpected result: {event:?}"
        );
    }
    assert_eq!(plane.rejected_count(), 0);

    let _ = shutdown.send(true);
    for task in tasks {
        let _ = task.await;
    }
}

/// AC-PR-01: an agent refuses a forged, tampered or unknown-key job and starts
/// no process for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ac_pr_01_forged_job_is_refused() {
    let count = 1;
    let pki = TestPki::generate(count).expect("generate the test certificates");
    let plane = common::start(&pki, count).await;
    let mut events = plane.subscribe();

    let (shutdown, tasks) =
        spawn_agents(&pki, &plane.url(), count, test_executor(), agent_backoff());
    assert!(plane.wait_connected(count, TIMEOUT).await);
    let id = plane.agents().into_iter().next().expect("one agent");

    // forged signature: a different key with a known key_id
    let forged = sign_job(
        &sample_job(&id, "forged-signature"),
        KEY_ID,
        &ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]),
    );
    assert!(plane.send_raw(&id, forged));

    // forged key: an unknown key_id
    let unknown = sign_job(&sample_job(&id, "forged-key"), "job-key-9", &signing_key());
    assert!(plane.send_raw(&id, unknown));

    // tampered bytes under a valid signature
    let mut tampered = sign_job(&sample_job(&id, "forged-bytes"), KEY_ID, &signing_key());
    tampered.job[4] ^= 0xff;
    assert!(plane.send_raw(&id, tampered));

    let rejected = common::collect_rejected(&mut events, 3, TIMEOUT).await;
    assert_eq!(
        rejected.len(),
        3,
        "the agent refused {} jobs",
        rejected.len()
    );
    let reasons: Vec<i32> = rejected
        .iter()
        .map(|event| match event {
            AgentEvent::Rejected { reason, .. } => *reason,
            other => panic!("unexpected event: {other:?}"),
        })
        .collect();
    assert!(reasons.contains(&(RejectReason::BadSignature as i32)));
    assert!(reasons.contains(&(RejectReason::UnknownKey as i32)));
    assert_eq!(plane.result_count(), 0, "no job may run");

    let _ = shutdown.send(true);
    for task in tasks {
        let _ = task.await;
    }
}

/// FR-SC-09 and FR-AG-10: a cancel stops the running job and reports the
/// cancelled status.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_sc_09_cancel_stops_a_running_job() {
    let count = 1;
    let pki = TestPki::generate(count).expect("generate the test certificates");
    let plane = common::start(&pki, count).await;
    let mut events = plane.subscribe();

    let executor: Arc<dyn restichub_emulator::executor::Executor> =
        Arc::new(restichub_emulator::executor::Simulated {
            profile: SimulatedProfile {
                step_seconds: 300.0,
                progress_ticks: 0,
            },
        });
    let (shutdown, tasks) = spawn_agents(&pki, &plane.url(), count, executor, agent_backoff());
    assert!(plane.wait_connected(count, TIMEOUT).await);
    let id = plane.agents().into_iter().next().expect("one agent");

    let job_id = "job-cancel";
    assert!(plane.dispatch(&id, &sample_job(&id, job_id)));
    let acked = common::collect_acks(&mut events, 1, TIMEOUT).await;
    assert_eq!(acked.len(), 1, "the job was not acknowledged");

    assert!(plane.cancel(&id, job_id, 0));
    let results = collect(&mut events, 1, TIMEOUT).await;
    assert_eq!(results.len(), 1, "no result after the cancel");
    assert!(
        matches!(&results[0], AgentEvent::Result { status, .. } if *status == StepStatus::Cancelled as i32),
        "unexpected result: {:?}",
        results[0]
    );

    let _ = shutdown.send(true);
    for task in tasks {
        let _ = task.await;
    }
}

/// FR-ID-08: an agent reconnects after the control plane drops the stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_id_08_agents_reconnect_after_a_disconnect() {
    let count = 3;
    let pki = TestPki::generate(count).expect("generate the test certificates");
    let plane = common::start(&pki, count).await;

    let (shutdown, tasks) =
        spawn_agents(&pki, &plane.url(), count, test_executor(), agent_backoff());
    assert!(plane.wait_connected(count, TIMEOUT).await);

    let before = plane.connect_count();
    plane.disconnect_all();

    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        if plane.connect_count() >= before + count as u64 && plane.connections() == count {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the agents did not reconnect: {} connects, {} streams",
            plane.connect_count(),
            plane.connections()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let _ = shutdown.send(true);
    for task in tasks {
        let _ = task.await;
    }
}
