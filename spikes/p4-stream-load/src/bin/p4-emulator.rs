//! Spike P4 agent emulator.
//!
//! One process spawns N emulated agents as tokio tasks. Each agent has its own
//! client certificate. It connects, sends `AgentHello`, verifies each received
//! job with `verify_job` and `validate_job`, answers `JobAck` and a fake
//! `JobResult`, and answers `Pong`. On a disconnect it reconnects with
//! exponential backoff and full jitter (FR-ID-08).

use std::path::Path;
use std::time::Duration;

use p4_stream_load::{
    SERVER_NAME, TENANT_ID, generate_certs, key_ring, now_timestamp, read_client, write_certs,
};
use rand::Rng;
use restichub_proto::agent::v1::{
    AgentHello, AgentMessage, JobAck, JobRejected, JobResult, Pong, StepStatus, agent_message,
    agent_service_client::AgentServiceClient, control_message,
};
use restichub_proto::signing::{Expected, verify_job};
use restichub_proto::validate::validate_job;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};

/// Base backoff, in seconds (FR-ID-08).
const BACKOFF_BASE_SECONDS: u64 = 1;

/// Maximum backoff, in seconds (FR-ID-08).
const BACKOFF_MAX_SECONDS: u64 = 60;

/// Number of outbound messages buffered for one agent.
const OUT_CHANNEL: usize = 64;

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

/// Runs one agent until the server closes the stream.
///
/// # Errors
///
/// Returns an error when the connection, the TLS handshake or a send fails.
async fn run_agent(
    ca: String,
    cert: String,
    key: String,
    agent_id: String,
    address: String,
) -> anyhow::Result<()> {
    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(ca))
        .identity(Identity::from_pem(cert, key))
        .domain_name(SERVER_NAME);
    let endpoint = Endpoint::from_shared(format!("https://{address}"))?.tls_config(tls)?;
    let channel: Channel = endpoint.connect().await?;
    let mut client = AgentServiceClient::new(channel);

    let (tx, rx) = mpsc::channel(OUT_CHANNEL);
    let response = client.connect(ReceiverStream::new(rx)).await?;
    let mut inbound = response.into_inner();

    let hello = AgentMessage {
        payload: Some(agent_message::Payload::AgentHello(AgentHello {
            protocol_version_major: 1,
            protocol_version_minor: 0,
            agent_version: "0.0.0-spike".to_owned(),
            restic_version: "0.19.0".to_owned(),
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            supported_step_types: vec!["backup".to_owned()],
            ..AgentHello::default()
        })),
    };
    tx.send(hello).await?;

    let ring = key_ring();
    while let Some(message) = inbound.message().await? {
        match message.payload {
            Some(control_message::Payload::Ping(ping)) => {
                let pong = AgentMessage {
                    payload: Some(agent_message::Payload::Pong(Pong { nonce: ping.nonce })),
                };
                tx.send(pong).await?;
            }
            Some(control_message::Payload::SignedJob(signed)) => {
                handle_job(&tx, &ring, &agent_id, signed).await?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Verifies a signed job, answers with a `JobAck` and a fake `JobResult`, or
/// with a `JobRejected`.
async fn handle_job(
    tx: &mpsc::Sender<AgentMessage>,
    ring: &restichub_proto::signing::KeyRing,
    agent_id: &str,
    signed: restichub_proto::agent::v1::SignedJob,
) -> anyhow::Result<()> {
    let expected = Expected {
        agent_id: agent_id.to_owned(),
        tenant_id: TENANT_ID.to_owned(),
        now: now_timestamp(),
    };
    match verify_job(&signed, ring, &expected) {
        Ok(job) => {
            if let Err(rejection) = validate_job(&job) {
                let message = AgentMessage {
                    payload: Some(agent_message::Payload::JobRejected(JobRejected {
                        job_id: rejection.job_id.unwrap_or_default(),
                        reason: rejection.reason as i32,
                    })),
                };
                tx.send(message).await?;
            } else {
                let ack = AgentMessage {
                    payload: Some(agent_message::Payload::JobAck(JobAck {
                        job_id: job.job_id.clone(),
                    })),
                };
                tx.send(ack).await?;
                let result = AgentMessage {
                    payload: Some(agent_message::Payload::JobResult(JobResult {
                        job_id: job.job_id,
                        status: StepStatus::Success as i32,
                        steps_total: 1,
                        steps_succeeded: 1,
                        steps_failed: 0,
                    })),
                };
                tx.send(result).await?;
            }
        }
        Err(rejection) => {
            let message = AgentMessage {
                payload: Some(agent_message::Payload::JobRejected(JobRejected {
                    job_id: rejection.job_id.unwrap_or_default(),
                    reason: rejection.reason as i32,
                })),
            };
            tx.send(message).await?;
        }
    }
    Ok(())
}

/// Runs one agent with reconnect and exponential backoff with full jitter.
async fn agent_loop(ca: String, cert: String, key: String, agent_id: String, address: String) {
    let mut attempt: u32 = 0;
    loop {
        let result = run_agent(
            ca.clone(),
            cert.clone(),
            key.clone(),
            agent_id.clone(),
            address.clone(),
        )
        .await;
        if result.is_ok() {
            attempt = 0;
        }
        attempt += 1;
        let shift = attempt.min(6);
        let cap = (BACKOFF_BASE_SECONDS << shift).min(BACKOFF_MAX_SECONDS);
        let wait = rand::rng().random_range(0.0..cap as f64);
        tokio::time::sleep(Duration::from_secs_f64(wait)).await;
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("run");

    if command == "gencerts" {
        let dir = args.get(2).cloned().unwrap_or_else(|| "certs".to_owned());
        let count: usize = args
            .get(3)
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let set = generate_certs(count)?;
        write_certs(Path::new(&dir), &set)?;
        println!("{}", json!({"event": "certs", "count": count, "dir": dir}));
        return Ok(());
    }

    let dir = args.get(2).cloned().unwrap_or_else(|| "certs".to_owned());
    let address = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:50051".to_owned());
    let duration: u64 = arg_value(&args, "--duration")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);

    let ca = std::fs::read_to_string(Path::new(&dir).join("ca.pem"))?;
    let agents: Vec<String> = std::fs::read_to_string(Path::new(&dir).join("agents.txt"))?
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();

    println!(
        "{}",
        json!({"event": "emulator_start", "agents": agents.len(), "address": address})
    );

    for agent_id in &agents {
        let (cert, key) = read_client(Path::new(&dir), agent_id)?;
        let ca = ca.clone();
        let agent_id = agent_id.clone();
        let address = address.clone();
        tokio::spawn(async move {
            agent_loop(ca, cert, key, agent_id, address).await;
        });
    }

    if duration > 0 {
        tokio::time::sleep(Duration::from_secs(duration)).await;
        println!(
            "{}",
            json!({"event": "emulator_done", "agents": agents.len()})
        );
    } else {
        std::future::pending::<()>().await;
    }
    Ok(())
}
