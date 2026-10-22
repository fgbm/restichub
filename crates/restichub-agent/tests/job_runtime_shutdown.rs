//! The shutdown of the runtime during a job raises no failure (FR-AG-09, ADR-0020, FR-IN-08).
//!
//! The test runs in its own process: the restic children that the shutdown leaves behind are
//! zombies of the process, and they would disturb the zombie checks of the other job tests.

#![allow(clippy::expect_used)]

mod common;

use std::time::Duration;

use common::host::HostFixture;
use common::{AGENT, TENANT, ring, signed, test_access};
use restichub_proto::agent::v1::agent_message;
use tokio::sync::mpsc;

/// A stop of the service or a reboot cancels the job task. The job stays in progress in the
/// journal, and the next start handles it, so the control plane raises no failure alert.
#[test]
fn fr_ag_09_a_runtime_shutdown_during_a_job_stores_no_failure() {
    for round in 0..3 {
        let host = HostFixture::new(&format!("runtime-shutdown-{round}"));
        common::script_exit_codes(&host.repository, 0, 0);
        common::script_sleep(&host.repository, 1500);
        let job = host.job("job-shutdown", vec![test_access("repo-1")]);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("cannot build the runtime");
        let jobs = host.jobs();
        let (tx, mut rx) = mpsc::channel(64);
        runtime.block_on(async {
            jobs.attach(&tx);
            jobs.greeted(false);
            jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;
            tokio::time::sleep(Duration::from_millis(300)).await;
        });
        runtime.shutdown_timeout(Duration::from_secs(5));

        while let Ok(message) = rx.try_recv() {
            assert!(
                !matches!(message.payload, Some(agent_message::Payload::JobResult(_))),
                "a shutdown must send no job result"
            );
        }
        assert!(
            host.jobs().held_job_ids().is_empty(),
            "a shutdown must store no result"
        );
    }
}

/// FR-AG-09, ADR-0020: a cancelled job task is no failure of the agent. The check does not depend on
/// the order in which the runtime stops its tasks.
#[tokio::test(flavor = "multi_thread")]
async fn fr_ag_09_a_cancelled_job_task_sends_no_failure_result() {
    let host = HostFixture::new("cancelled-task");
    common::script_exit_codes(&host.repository, 0, 0);
    common::script_sleep(&host.repository, 1000);
    let job = host.job("job-cancelled", vec![test_access("repo-1")]);
    let jobs = host.jobs().with_abort_job_task();
    let (tx, mut rx) = mpsc::channel(64);
    jobs.attach(&tx);
    jobs.greeted(false);
    jobs.handle(signed(&job), &ring(), AGENT, TENANT).await;

    tokio::time::sleep(Duration::from_millis(1500)).await;
    while let Ok(message) = rx.try_recv() {
        assert!(
            !matches!(message.payload, Some(agent_message::Payload::JobResult(_))),
            "a cancelled task must send no job result"
        );
    }
    assert!(
        host.jobs().held_job_ids().is_empty(),
        "a cancelled task must store no result"
    );
}
