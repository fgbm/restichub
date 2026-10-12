//! Spike P4 control plane stand-in.
//!
//! The process serves `AgentService.Connect` over tonic and rustls mTLS. It
//! takes the agent identity from the client certificate common name. It keeps
//! a map agent_id -> sender, sends `ServerHello`, sends `Ping` every
//! [`PING_INTERVAL_SECONDS`], and dispatches real signed jobs. It prints
//! counters to stdout as JSON.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use p4_stream_load::{
    GRACE_SECONDS, KEY_ID, PING_INTERVAL_SECONDS, common_name, sample_job, signing_key,
};
use rand::Rng;
use restichub_proto::agent::v1::{
    AgentMessage, ControlMessage, ServerHello, agent_message,
    agent_service_server::{AgentService, AgentServiceServer},
    control_message,
};
use restichub_proto::signing::sign_job;
use restichub_proto::validate::validate_job;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status, Streaming};

/// Number of output messages buffered for one agent.
const OUT_CHANNEL: usize = 64;

/// The job dispatch profile of NFR-PERF-01: one job for each agent every
/// 900 seconds.
const SECONDS_PER_JOB: f64 = 900.0;

/// The shared state of the control plane stand-in.
struct State {
    /// sender for each connected agent.
    agents: Mutex<HashMap<String, mpsc::Sender<ControlMessage>>>,
    /// identifiers that currently have a stream.
    active: Mutex<HashSet<String>>,
    /// dispatch time of each job that waits for an ack.
    pending: Mutex<HashMap<String, Instant>>,
    /// ack latency of each periodic job, in microseconds.
    latencies_us: Mutex<Vec<u64>>,
    /// ack latency of each burst job, in microseconds.
    burst_us: Mutex<Vec<u64>>,
    /// time of each accepted connection.
    connect_times: Mutex<Vec<Instant>>,
    /// number of accepted connections.
    connects: AtomicU64,
    /// number of accepted connections inside the grace period.
    connects_grace: AtomicU64,
    /// number of dispatched jobs.
    dispatched: AtomicU64,
    /// number of job acknowledgements.
    acks: AtomicU64,
    /// number of rejected jobs.
    rejected: AtomicU64,
    /// number of dropped jobs.
    dropped: AtomicU64,
    /// job identifier counter.
    job_counter: AtomicU64,
    /// process start time.
    start: Instant,
    /// number of agents that the run expects.
    expected: usize,
    /// milliseconds until all agents connected, or 0 before that.
    all_connected_ms: AtomicU64,
}

impl State {
    fn new(expected: usize) -> Self {
        Self {
            agents: Mutex::new(HashMap::new()),
            active: Mutex::new(HashSet::new()),
            pending: Mutex::new(HashMap::new()),
            latencies_us: Mutex::new(Vec::new()),
            burst_us: Mutex::new(Vec::new()),
            connect_times: Mutex::new(Vec::new()),
            connects: AtomicU64::new(0),
            connects_grace: AtomicU64::new(0),
            dispatched: AtomicU64::new(0),
            acks: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            job_counter: AtomicU64::new(0),
            start: Instant::now(),
            expected,
            all_connected_ms: AtomicU64::new(0),
        }
    }

    /// Returns the largest number of connections in any one-second window.
    fn peak_rate(&self) -> f64 {
        let times = self.connect_times.lock().unwrap();
        let mut peak = 0usize;
        let mut start = 0usize;
        for (index, time) in times.iter().enumerate() {
            while time.duration_since(times[start]) > Duration::from_secs(1) {
                start += 1;
            }
            peak = peak.max(index - start + 1);
        }
        peak as f64
    }

    /// Returns a percentile (0.0 to 1.0) of a sample set.
    fn percentile(samples: &[u64], fraction: f64) -> u64 {
        if samples.is_empty() {
            return 0;
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let last = sorted.len() - 1;
        let index = ((last as f64) * fraction).round() as usize;
        sorted[index.min(last)]
    }

    /// Builds the periodic status report.
    fn report(&self) -> serde_json::Value {
        let latencies = self.latencies_us.lock().unwrap().clone();
        let burst = self.burst_us.lock().unwrap().clone();
        let connected = self.active.lock().unwrap().len();
        json!({
            "event": "status",
            "t_ms": self.start.elapsed().as_millis() as u64,
            "connected": connected,
            "expected": self.expected,
            "connects": self.connects.load(Ordering::Relaxed),
            "connects_grace": self.connects_grace.load(Ordering::Relaxed),
            "dispatched": self.dispatched.load(Ordering::Relaxed),
            "acks": self.acks.load(Ordering::Relaxed),
            "rejected": self.rejected.load(Ordering::Relaxed),
            "dropped": self.dropped.load(Ordering::Relaxed),
            "all_connected_ms": self.all_connected_ms.load(Ordering::Relaxed),
            "peak_connect_per_s": self.peak_rate(),
            "ack_p50_us": Self::percentile(&latencies, 0.50),
            "ack_p95_us": Self::percentile(&latencies, 0.95),
            "ack_p99_us": Self::percentile(&latencies, 0.99),
            "burst_p50_us": Self::percentile(&burst, 0.50),
            "burst_p95_us": Self::percentile(&burst, 0.95),
            "burst_p99_us": Self::percentile(&burst, 0.99),
            "burst_count": burst.len(),
        })
    }
}

/// The service implementation.
#[derive(Clone)]
struct ControlPlane {
    state: Arc<State>,
}

#[tonic::async_trait]
impl AgentService for ControlPlane {
    type ConnectStream =
        Pin<Box<dyn tokio_stream::Stream<Item = Result<ControlMessage, Status>> + Send>>;

    async fn connect(
        &self,
        request: Request<Streaming<AgentMessage>>,
    ) -> Result<Response<Self::ConnectStream>, Status> {
        let peer = request
            .peer_certs()
            .ok_or_else(|| Status::unauthenticated("no client certificate"))?;
        let leaf = peer
            .first()
            .ok_or_else(|| Status::unauthenticated("empty client certificate chain"))?;
        let agent_id = common_name(leaf.as_ref())
            .map_err(|error| Status::unauthenticated(format!("bad certificate: {error}")))?;

        let mut inbound = request.into_inner();
        let (tx, rx) = mpsc::channel(OUT_CHANNEL);

        // Register the agent and record the connection.
        self.state
            .agents
            .lock()
            .unwrap()
            .insert(agent_id.clone(), tx.clone());
        self.state.active.lock().unwrap().insert(agent_id.clone());
        self.state.connects.fetch_add(1, Ordering::Relaxed);
        let elapsed = self.state.start.elapsed();
        self.state
            .connect_times
            .lock()
            .unwrap()
            .push(Instant::now());
        if elapsed < Duration::from_secs(GRACE_SECONDS) {
            self.state.connects_grace.fetch_add(1, Ordering::Relaxed);
        }
        let connected = self.state.active.lock().unwrap().len();
        if connected >= self.state.expected
            && self.state.all_connected_ms.load(Ordering::Relaxed) == 0
        {
            let ms = elapsed.as_millis() as u64;
            if self.state.all_connected_ms.swap(ms, Ordering::Relaxed) == 0 {
                println!(
                    "{}",
                    json!({
                        "event": "all_connected",
                        "ms": ms,
                        "peak_connect_per_s": self.state.peak_rate(),
                    })
                );
            }
        }

        // Send ServerHello as the first message.
        let hello = ControlMessage {
            payload: Some(control_message::Payload::ServerHello(ServerHello {
                protocol_version_major: 1,
                protocol_version_minor: 0,
                accepted_capabilities: vec!["backup".to_owned()],
            })),
        };
        let _ = tx.try_send(hello);

        // Handle the inbound messages in a background task.
        let state = Arc::clone(&self.state);
        let id = agent_id.clone();
        tokio::spawn(async move {
            while let Some(message) = inbound.message().await.transpose() {
                let Ok(message) = message else { break };
                handle_agent_message(&state, &message);
            }
            state.agents.lock().unwrap().remove(&id);
            state.active.lock().unwrap().remove(&id);
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx).map(Ok))))
    }
}

/// Handles one message from an agent.
fn handle_agent_message(state: &Arc<State>, message: &AgentMessage) {
    match &message.payload {
        Some(agent_message::Payload::JobAck(ack)) => {
            state.acks.fetch_add(1, Ordering::Relaxed);
            if let Some(sent) = state.pending.lock().unwrap().remove(&ack.job_id) {
                let micros = sent.elapsed().as_micros() as u64;
                if ack.job_id.starts_with("burst-") {
                    state.burst_us.lock().unwrap().push(micros);
                } else {
                    state.latencies_us.lock().unwrap().push(micros);
                }
            }
        }
        Some(agent_message::Payload::JobRejected(_)) => {
            state.rejected.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
}

/// Sends one signed job to a random connected agent. Returns `false` when no
/// agent is connected.
fn dispatch_one(state: &Arc<State>, burst: bool) -> bool {
    let id = {
        let agents = state.agents.lock().unwrap();
        if agents.is_empty() {
            return false;
        }
        let mut rng = rand::rng();
        let index = rng.random_range(0..agents.len());
        agents.keys().nth(index).cloned().unwrap()
    };

    let counter = state.job_counter.fetch_add(1, Ordering::Relaxed);
    let job_id = if burst {
        format!("burst-{counter:012}")
    } else {
        format!("job-{counter:012}")
    };
    let job = sample_job(&id, &job_id);
    if validate_job(&job).is_err() {
        // The sample job is valid; this guards against a change in the contract.
        return false;
    }
    let signed = sign_job(&job, KEY_ID, &signing_key());
    let message = ControlMessage {
        payload: Some(control_message::Payload::SignedJob(signed)),
    };

    let sender = state.agents.lock().unwrap().get(&id).cloned();
    let Some(sender) = sender else { return false };
    state
        .pending
        .lock()
        .unwrap()
        .insert(job_id.clone(), Instant::now());
    match sender.try_send(message) {
        Ok(()) => {
            state.dispatched.fetch_add(1, Ordering::Relaxed);
            true
        }
        Err(_) => {
            state.pending.lock().unwrap().remove(&job_id);
            state.dropped.fetch_add(1, Ordering::Relaxed);
            false
        }
    }
}

/// Sends `Ping` to every connected agent.
fn send_pings(state: &Arc<State>, nonce: u64) {
    let senders: Vec<mpsc::Sender<ControlMessage>> =
        state.agents.lock().unwrap().values().cloned().collect();
    for sender in senders {
        let ping = ControlMessage {
            payload: Some(control_message::Payload::Ping(
                restichub_proto::agent::v1::Ping { nonce },
            )),
        };
        let _ = sender.try_send(ping);
    }
}

/// Reads the value of `--name value` or `--name=value`.
fn arg_value(args: &[String], name: &str) -> Option<String> {
    for (index, arg) in args.iter().enumerate() {
        if arg == name {
            return args.get(index + 1).cloned();
        }
        if let Some(rest) = arg.strip_prefix(&format!("{name}=")) {
            return Some(rest.to_owned());
        }
    }
    None
}

/// Resolves when the process gets SIGINT or SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let certs = arg_value(&args, "--certs").unwrap_or_else(|| "certs".to_owned());
    let port: u16 = arg_value(&args, "--port")
        .and_then(|value| value.parse().ok())
        .unwrap_or(50051);
    let expected: usize = arg_value(&args, "--agents")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let burst_at: u64 = arg_value(&args, "--burst-at")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);

    let dir = std::path::Path::new(&certs);
    let ca = std::fs::read_to_string(dir.join("ca.pem"))?;
    let server_cert = std::fs::read_to_string(dir.join("server.pem"))?;
    let server_key = std::fs::read_to_string(dir.join("server.key"))?;

    let state = Arc::new(State::new(expected));
    println!(
        "{}",
        json!({
            "event": "server_start",
            "t_ms": 0,
            "port": port,
            "expected": expected,
            "pid": std::process::id(),
        })
    );

    // Ping task (FR-ID-07).
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(PING_INTERVAL_SECONDS));
            let mut nonce = 0u64;
            loop {
                ticker.tick().await;
                nonce += 1;
                send_pings(&state, nonce);
            }
        });
    }

    // Dispatch task at the NFR-PERF-01 rate.
    if expected > 0 {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let period = SECONDS_PER_JOB / expected as f64;
            let mut ticker = tokio::time::interval(Duration::from_secs_f64(period));
            loop {
                ticker.tick().await;
                dispatch_one(&state, false);
            }
        });
    }

    // Burst task: 100 jobs at once after `burst_at` seconds.
    if burst_at > 0 {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(burst_at)).await;
            for _ in 0..100 {
                dispatch_one(&state, true);
            }
            println!("{}", json!({"event": "burst_sent", "count": 100}));
        });
    }

    // Report task: JSON status every 10 seconds.
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(10));
            ticker.tick().await;
            loop {
                ticker.tick().await;
                println!("{}", state.report());
            }
        });
    }

    // Grace watcher: the count of missing agents after the grace period.
    if expected > 0 {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(GRACE_SECONDS)).await;
            let connected = state.active.lock().unwrap().len();
            let missing = expected.saturating_sub(connected);
            println!(
                "{}",
                json!({"event": "grace_alerts", "n": missing, "grace_s": GRACE_SECONDS})
            );
        });
    }

    let tls = ServerTlsConfig::new()
        .identity(Identity::from_pem(server_cert, server_key))
        .client_ca_root(Certificate::from_pem(ca));
    let address = format!("127.0.0.1:{port}").parse()?;
    println!(
        "{}",
        json!({"event": "listening", "addr": format!("127.0.0.1:{port}")})
    );

    Server::builder()
        .tls_config(tls)?
        .add_service(AgentServiceServer::new(ControlPlane { state }))
        .serve_with_shutdown(address, shutdown_signal())
        .await?;

    Ok(())
}
