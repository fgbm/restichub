//! Job logs and hostile agent modes against the in-process test control plane (FR-AG-08,
//! FR-LG-07, AC-PR-05).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use restichub_emulator::agent::Backoff;
use restichub_emulator::control::{AgentEvent, TestControlPlane};
use restichub_emulator::executor::{Executor, Profiled, ProfiledProfile};
use restichub_emulator::identity::{SERVER_NAME, TENANT_ID, TestPki, key_ring};
use restichub_emulator::job::sample_job;
use restichub_emulator::metrics::Summary;
use restichub_emulator::profile::{LoadProfile, Oversize};
use restichub_emulator::runner::{LoadRun, RunConfig};
use tokio::sync::broadcast;

const AGENTS: usize = 3;
const TIMEOUT: Duration = Duration::from_secs(30);
const STEP_SECONDS: f64 = 1.0;

fn config(plane: &TestControlPlane, pki: &TestPki, profile: LoadProfile) -> RunConfig {
    let executor: Arc<dyn Executor> = Arc::new(Profiled {
        profile: ProfiledProfile {
            min_seconds: STEP_SECONDS,
            max_seconds: STEP_SECONDS,
            progress_rate: 0.0,
            failures: Vec::new(),
        },
    });
    RunConfig {
        profile,
        server_url: plane.url(),
        server_name: SERVER_NAME.to_owned(),
        ca_pem: pki.ca_pem().to_owned(),
        tenant_id: TENANT_ID.to_owned(),
        key_ring: key_ring(),
        identities: pki.agents().to_vec(),
        executor,
        backoff: Backoff {
            base: Duration::from_millis(50),
            max: Duration::from_millis(200),
        },
        metrics_addr: None,
    }
}

/// Waits for `connects` streams and all agents, sends one job to each agent and collects the
/// events until every agent reported its result.
async fn run_one_job_each(
    plane: &TestControlPlane,
    events: &mut broadcast::Receiver<AgentEvent>,
    connects: u64,
) -> (HashMap<String, String>, Vec<AgentEvent>) {
    let deadline = Instant::now() + TIMEOUT;
    while (plane.connect_count() < connects || plane.connections() < AGENTS)
        && Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(plane.connections(), AGENTS, "not every agent connected");
    let mut dispatched = HashMap::new();
    for (index, agent_id) in plane.agents().into_iter().enumerate() {
        let job_id = format!("job-{index}");
        assert!(plane.dispatch(&agent_id, &sample_job(&agent_id, &job_id)));
        dispatched.insert(agent_id, job_id);
    }
    let mut seen = Vec::new();
    let mut finished = HashSet::new();
    let deadline = Instant::now() + TIMEOUT;
    while finished.len() < AGENTS && Instant::now() < deadline {
        let Ok(Ok(event)) = tokio::time::timeout(Duration::from_secs(1), events.recv()).await
        else {
            continue;
        };
        if let AgentEvent::Result {
            agent_id, job_id, ..
        } = &event
            && dispatched.get(agent_id) == Some(job_id)
        {
            finished.insert(agent_id.clone());
        }
        seen.push(event);
    }
    assert_eq!(finished.len(), AGENTS, "not every agent reported its job");
    let drain = Instant::now() + Duration::from_millis(500);
    while let Ok(Ok(event)) = tokio::time::timeout(
        drain.saturating_duration_since(Instant::now()),
        events.recv(),
    )
    .await
    {
        seen.push(event);
    }
    (dispatched, seen)
}

async fn stop_when_acknowledged(run: LoadRun) -> Summary {
    let deadline = Instant::now() + TIMEOUT;
    while run.metrics().logs().chunks_unacked > 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    run.stop().await
}

/// FR-AG-08 and FR-LG-07: a step sends its log as chunks at the profile rate, and each
/// acknowledged chunk leaves the spool.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_lg_07_job_logs_are_sent_and_acknowledged() {
    let pki = TestPki::generate(AGENTS).expect("generate the test certificates");
    let plane = TestControlPlane::start(&pki, AGENTS)
        .await
        .expect("start the test control plane");
    let mut events = plane.subscribe();
    let profile = LoadProfile {
        agents: AGENTS,
        duration_seconds: 60.0,
        log_lines_per_second: 400.0,
        log_line_bytes: 100,
        ..LoadProfile::default()
    };
    let run = LoadRun::start(config(&plane, &pki, profile))
        .await
        .expect("start the load run");

    let (dispatched, seen) = run_one_job_each(&plane, &mut events, AGENTS as u64).await;
    let summary = stop_when_acknowledged(run).await;

    for (agent_id, job_id) in &dispatched {
        let sequences: Vec<u64> = seen
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Log {
                    agent_id: from,
                    job_id: job,
                    sequence,
                    ..
                } if from == agent_id && job == job_id => Some(*sequence),
                _ => None,
            })
            .collect();
        assert!(sequences.len() >= 4, "{agent_id}: {sequences:?}");
        assert!(sequences.windows(2).all(|pair| pair[1] == pair[0] + 1));
    }
    let logs = summary.logs;
    assert!(logs.bytes_sent >= (AGENTS * 300 * 100) as u64, "{logs:?}");
    assert_eq!(logs.chunks_acked, logs.chunks_sent, "{logs:?}");
    assert_eq!(logs.chunks_unacked, 0);
}

/// AC-PR-05: a hostile agent sends results for jobs that it never got, a flood of progress
/// messages and one message above the size limit. The other agents keep working.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ac_pr_05_a_hostile_agent_sends_forged_and_oversize_messages() {
    let pki = TestPki::generate(AGENTS).expect("generate the test certificates");
    let hostile_id = pki.agents()[0].agent_id.clone();
    let plane = TestControlPlane::start(&pki, AGENTS)
        .await
        .expect("start the test control plane");
    let mut events = plane.subscribe();
    let mut profile = LoadProfile {
        agents: AGENTS,
        duration_seconds: 60.0,
        ..LoadProfile::default()
    };
    profile.hostile.agents = 1;
    profile.hostile.forged_result_share = 1.0;
    profile.hostile.flood_per_second = 200.0;
    profile.hostile.oversize = Some(Oversize::AboveLimit);
    let run = LoadRun::start(config(&plane, &pki, profile))
        .await
        .expect("start the load run");

    let (dispatched, seen) = run_one_job_each(&plane, &mut events, AGENTS as u64 + 1).await;
    let summary = run.stop().await;

    let forged: Vec<&String> = seen
        .iter()
        .filter_map(|event| match event {
            AgentEvent::Result {
                agent_id, job_id, ..
            } if *agent_id == hostile_id && dispatched.get(agent_id) != Some(job_id) => {
                Some(job_id)
            }
            _ => None,
        })
        .collect();
    assert_eq!(forged.len(), 1, "{forged:?}");
    let flood = seen
        .iter()
        .filter(|event| matches!(event, AgentEvent::Progress { agent_id, .. } if *agent_id == hostile_id))
        .count();
    assert!(flood > 0);
    let hostile = &summary.hostile;
    assert_eq!(
        hostile.forged_results_foreign + hostile.forged_results_unknown,
        1
    );
    assert!(hostile.flood_messages >= flood as u64);
    assert_eq!(hostile.oversize_messages, 1);
    // The control plane drops the stream of a message above the limit; the agent reconnects once.
    assert!(summary.reconnects_total >= 1, "{summary:?}");
}
