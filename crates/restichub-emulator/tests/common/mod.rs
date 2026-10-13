//! Helpers for the emulator integration tests.

use std::sync::Arc;
use std::time::{Duration, Instant};

use restichub_emulator::agent::{AgentConfig, Backoff, run_agent};
use restichub_emulator::control::{AgentEvent, TestControlPlane};
use restichub_emulator::executor::{Executor, Simulated, SimulatedProfile};
use restichub_emulator::identity::{SERVER_NAME, TENANT_ID, TestPki, key_ring};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

/// Makes a test control plane that expects `count` agents.
pub async fn start(pki: &TestPki, count: usize) -> TestControlPlane {
    match TestControlPlane::start(pki, count).await {
        Ok(plane) => plane,
        Err(error) => panic!("cannot start the test control plane: {error:#}"),
    }
}

/// Short reconnect backoff for the tests.
#[must_use]
pub fn agent_backoff() -> Backoff {
    Backoff {
        base: Duration::from_millis(50),
        max: Duration::from_millis(500),
    }
}

/// Default simulated executor.
#[must_use]
pub fn test_executor() -> Arc<dyn Executor> {
    Arc::new(Simulated {
        profile: SimulatedProfile::default(),
    })
}

/// Makes `count` emulated agents connect to `url`.
pub fn spawn_agents(
    pki: &TestPki,
    url: &str,
    count: usize,
    executor: Arc<dyn Executor>,
    backoff: Backoff,
) -> (watch::Sender<bool>, Vec<JoinHandle<()>>) {
    let (shutdown, receiver) = watch::channel(false);
    let mut tasks = Vec::with_capacity(count);
    for identity in pki.agents().iter().take(count).cloned() {
        let config = Arc::new(AgentConfig {
            tenant_id: TENANT_ID.to_owned(),
            server_url: url.to_owned(),
            server_name: SERVER_NAME.to_owned(),
            ca_pem: pki.ca_pem().to_owned(),
            identity,
            key_ring: key_ring(),
            executor: Arc::clone(&executor),
            backoff,
        });
        tasks.push(tokio::spawn(run_agent(config, receiver.clone())));
    }
    (shutdown, tasks)
}

/// Waits for `count` job results.
pub async fn collect(
    events: &mut broadcast::Receiver<AgentEvent>,
    count: usize,
    timeout: Duration,
) -> Vec<AgentEvent> {
    collect_where(events, count, timeout, |event| {
        matches!(event, AgentEvent::Result { .. })
    })
    .await
}

/// Collects `count` job acknowledgements.
pub async fn collect_acks(
    events: &mut broadcast::Receiver<AgentEvent>,
    count: usize,
    timeout: Duration,
) -> Vec<AgentEvent> {
    collect_where(events, count, timeout, |event| {
        matches!(event, AgentEvent::Ack { .. })
    })
    .await
}

/// Waits for `count` rejected jobs.
pub async fn collect_rejected(
    events: &mut broadcast::Receiver<AgentEvent>,
    count: usize,
    timeout: Duration,
) -> Vec<AgentEvent> {
    collect_where(events, count, timeout, |event| {
        matches!(event, AgentEvent::Rejected { .. })
    })
    .await
}

async fn collect_where(
    events: &mut broadcast::Receiver<AgentEvent>,
    count: usize,
    timeout: Duration,
    predicate: impl Fn(&AgentEvent) -> bool,
) -> Vec<AgentEvent> {
    let deadline = Instant::now() + timeout;
    let mut found = Vec::new();
    while found.len() < count {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(event)) if predicate(&event) => found.push(event),
            Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => break,
        }
    }
    found
}
