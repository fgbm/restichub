//! End-to-end load run against the in-process test control plane (AC-NFR-01,
//! NFR-PERF-04).

use std::sync::Arc;
use std::time::{Duration, Instant};

use restichub_emulator::agent::Backoff;
use restichub_emulator::control::TestControlPlane;
use restichub_emulator::executor::{Executor, Profiled, ProfiledProfile};
use restichub_emulator::identity::{SERVER_NAME, TENANT_ID, TestPki, key_ring};
use restichub_emulator::job::sample_job;
use restichub_emulator::profile::LoadProfile;
use restichub_emulator::runner::{LoadRun, RunConfig};

const TIMEOUT: Duration = Duration::from_secs(60);
const AGENTS: usize = 50;
const DURATION: Duration = Duration::from_secs(10);
const STORM_PERCENT: f64 = 20.0;

fn profile() -> LoadProfile {
    let mut profile = LoadProfile {
        agents: AGENTS,
        duration_seconds: DURATION.as_secs_f64(),
        ramp_up_per_second: 100.0,
        ..LoadProfile::default()
    };
    profile.reconnect_storm.at_seconds = 3.0;
    profile.reconnect_storm.percent = STORM_PERCENT;
    profile.job_duration.min_seconds = 0.05;
    profile.job_duration.max_seconds = 0.05;
    profile
}

/// AC-NFR-01 and NFR-PERF-04: a short run of 50 agents reports the connect,
/// the reconnect and the result counts in the summary.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ac_nfr_01_short_load_run_reports_the_summary() {
    let pki = TestPki::generate(AGENTS).expect("generate the test certificates");
    let plane = TestControlPlane::start(&pki, AGENTS)
        .await
        .expect("start the test control plane");
    let executor: Arc<dyn Executor> = Arc::new(Profiled {
        profile: ProfiledProfile {
            min_seconds: 0.05,
            max_seconds: 0.05,
            progress_rate: 0.0,
            failures: Vec::new(),
        },
    });
    let config = RunConfig {
        profile: profile(),
        server_url: plane.url(),
        server_name: SERVER_NAME.to_owned(),
        ca_pem: pki.ca_pem().to_owned(),
        tenant_id: TENANT_ID.to_owned(),
        key_ring: key_ring(),
        identities: pki.agents().to_vec(),
        executor,
        backoff: Backoff {
            base: Duration::from_millis(50),
            max: Duration::from_millis(500),
        },
        metrics_addr: None,
    };

    let started = Instant::now();
    let run = LoadRun::start(config).await.expect("start the load run");
    assert!(
        plane.wait_connected(AGENTS, TIMEOUT).await,
        "only {} of {AGENTS} agents connected",
        plane.connections()
    );

    let deadline = started + DURATION;
    let mut counter = 0u64;
    while Instant::now() < deadline {
        for id in plane.agents() {
            let job = sample_job(&id, &format!("job-{counter:08}"));
            plane.dispatch(&id, &job);
            counter += 1;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    let summary = run.stop().await;

    assert_eq!(summary.agents, AGENTS);
    assert!(
        summary.connects_total >= AGENTS as u64,
        "connects {}",
        summary.connects_total
    );
    let selected = (STORM_PERCENT / 100.0 * AGENTS as f64).round() as u64;
    assert_eq!(
        summary.reconnects_total, selected,
        "reconnects {}",
        summary.reconnects_total
    );
    assert!(
        summary.results_total >= AGENTS as u64,
        "results {}",
        summary.results_total
    );
    assert_eq!(summary.rejected_total, 0);
    assert_eq!(summary.streams_current, 0);
    assert!(summary.connect_seconds.count >= AGENTS as u64);
    assert!(summary.duration_seconds >= DURATION.as_secs_f64() - 1.0);
}
