//! In-process test control plane for the emulator tests and the test server.
//!
//! It stands in for the real control plane before epic #1. It terminates mTLS
//! with the test authority, takes the agent identity from the common name of
//! the client certificate (FR-PR-08), and dispatches signed jobs. The module is
//! behind the `test-util` feature and is not part of the released binary.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use restichub_proto::agent::v1::{
    AgentMessage, CancelJob, ControlMessage, Job, LogAck, LogStreamAck, Ping, ServerHello,
    SignedJob, SigningKeyUpdate, agent_message,
    agent_service_server::{AgentService, AgentServiceServer},
    control_message,
};
use restichub_proto::signing::sign_job;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status, Streaming};

use crate::identity::{KEY_ID, TestPki, common_name, signing_key};

/// Number of control messages buffered for one agent.
const CONTROL_CHANNEL: usize = 64;

/// One message that an emulated agent sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    Hello {
        agent_id: String,
        protocol_major: u32,
    },
    Ack {
        agent_id: String,
        job_id: String,
    },
    Rejected {
        agent_id: String,
        job_id: String,
        reason: i32,
    },
    Result {
        agent_id: String,
        job_id: String,
        status: i32,
    },
    Progress {
        agent_id: String,
        job_id: String,
        step_id: String,
    },
    Pong {
        agent_id: String,
        nonce: u64,
    },
    Log {
        agent_id: String,
        job_id: String,
        sequence: u64,
        bytes: usize,
    },
}

/// One open agent stream. The generation tells the cleanup of an older stream
/// from the current one, so a reconnect survives it (FR-ID-08).
struct Connection {
    generation: u64,
    tx: mpsc::Sender<ControlMessage>,
}

struct State {
    agents: Mutex<HashMap<String, Connection>>,
    events: broadcast::Sender<AgentEvent>,
    connects: AtomicU64,
    rejected: AtomicU64,
    results: AtomicU64,
    next_generation: AtomicU64,
    expected: usize,
}

/// A test control plane that listens on a loopback port.
pub struct TestControlPlane {
    state: Arc<State>,
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl TestControlPlane {
    /// Starts the server with the server identity and the client authority of
    /// `pki`.
    ///
    /// # Errors
    ///
    /// Returns an error when the listener or the TLS configuration fails.
    pub async fn start(pki: &TestPki, expected: usize) -> anyhow::Result<Self> {
        let tls = ServerTlsConfig::new()
            .identity(Identity::from_pem(
                pki.server_cert_pem(),
                pki.server_key_pem(),
            ))
            .client_ca_root(Certificate::from_pem(pki.ca_pem()));
        let (events, _) = broadcast::channel(4096);
        let state = Arc::new(State {
            agents: Mutex::new(HashMap::new()),
            events,
            connects: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            results: AtomicU64::new(0),
            next_generation: AtomicU64::new(0),
            expected,
        });
        let service = ControlPlane {
            state: Arc::clone(&state),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let incoming = TcpListenerStream::new(listener);
        let (shutdown, shutdown_rx) = oneshot::channel();
        let server = Server::builder().tls_config(tls)?.add_service(
            AgentServiceServer::new(service)
                .max_decoding_message_size(restichub_proto::MAX_MESSAGE_BYTES),
        );
        let task = tokio::spawn(async move {
            if let Err(error) = server
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = shutdown_rx.await;
                })
                .await
            {
                eprintln!("test control plane stopped: {error}");
            }
        });
        Ok(Self {
            state,
            addr,
            shutdown: Some(shutdown),
            task,
        })
    }

    /// Address of the listener.
    #[must_use]
    pub fn url(&self) -> String {
        format!("https://127.0.0.1:{}", self.addr.port())
    }

    /// Subscribes to the messages of the connected agents.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.state.events.subscribe()
    }

    /// Number of agents that hold a stream at this moment.
    #[must_use]
    pub fn connections(&self) -> usize {
        crate::lock(&self.state.agents).len()
    }

    /// Number of accepted connections since the start.
    #[must_use]
    pub fn connect_count(&self) -> u64 {
        self.state.connects.load(Ordering::Relaxed)
    }

    /// Number of agents in the run.
    #[must_use]
    pub fn expected(&self) -> usize {
        self.state.expected
    }

    /// Number of jobs that the agents rejected.
    #[must_use]
    pub fn rejected_count(&self) -> u64 {
        self.state.rejected.load(Ordering::Relaxed)
    }

    /// Number of job results that the agents reported.
    #[must_use]
    pub fn result_count(&self) -> u64 {
        self.state.results.load(Ordering::Relaxed)
    }

    /// Identifiers of the connected clients.
    #[must_use]
    pub fn agents(&self) -> Vec<String> {
        crate::lock(&self.state.agents).keys().cloned().collect()
    }

    /// Blocks until `count` agents hold a stream, or until the timeout.
    pub async fn wait_connected(&self, count: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.connections() >= count {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.connections() >= count
    }

    /// Signs a job with the test key and sends it to one connected agent
    /// (FR-PR-03).
    pub fn dispatch(&self, agent_id: &str, job: &Job) -> bool {
        let signed = sign_job(job, KEY_ID, &signing_key());
        self.send_raw(agent_id, signed)
    }

    /// Sends a signed key set update to one agent (FR-PR-11).
    pub fn deliver_key_update(&self, agent_id: &str, update: SigningKeyUpdate) -> bool {
        let sender = crate::lock(&self.state.agents)
            .get(agent_id)
            .map(|connection| connection.tx.clone());
        let Some(sender) = sender else {
            return false;
        };
        let message = ControlMessage {
            payload: Some(control_message::Payload::SigningKeyUpdate(update)),
        };
        sender.try_send(message).is_ok()
    }

    /// Sends a signed job without a check, for a forged or tampered job
    /// (AC-PR-01).
    pub fn send_raw(&self, agent_id: &str, signed: SignedJob) -> bool {
        let sender = crate::lock(&self.state.agents)
            .get(agent_id)
            .map(|connection| connection.tx.clone());
        let Some(sender) = sender else {
            return false;
        };
        let message = ControlMessage {
            payload: Some(control_message::Payload::SignedJob(signed)),
        };
        sender.try_send(message).is_ok()
    }

    /// Stops a running job of one agent (FR-SC-09).
    pub fn cancel(&self, agent_id: &str, job_id: &str, grace_seconds: u32) -> bool {
        let sender = crate::lock(&self.state.agents)
            .get(agent_id)
            .map(|connection| connection.tx.clone());
        let Some(sender) = sender else {
            return false;
        };
        let message = ControlMessage {
            payload: Some(control_message::Payload::CancelJob(CancelJob {
                job_id: job_id.to_owned(),
                grace_period_seconds: grace_seconds,
            })),
        };
        sender.try_send(message).is_ok()
    }

    /// Sends a heartbeat to every connected agent (FR-ID-07).
    pub fn ping(&self, nonce: u64) {
        let senders: Vec<mpsc::Sender<ControlMessage>> = crate::lock(&self.state.agents)
            .values()
            .map(|connection| connection.tx.clone())
            .collect();
        for sender in senders {
            let _ = sender.try_send(ControlMessage {
                payload: Some(control_message::Payload::Ping(Ping { nonce })),
            });
        }
    }

    /// Closes every agent stream. The agents reconnect with backoff (FR-ID-08).
    pub fn disconnect_all(&self) {
        crate::lock(&self.state.agents).clear();
    }
}

impl Drop for TestControlPlane {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

#[derive(Clone)]
struct ControlPlane {
    state: Arc<State>,
}

#[tonic::async_trait]
impl AgentService for ControlPlane {
    type ConnectStream = Pin<Box<dyn Stream<Item = Result<ControlMessage, Status>> + Send>>;

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
        let (tx, rx) = mpsc::channel(CONTROL_CHANNEL);
        let generation = self.state.next_generation.fetch_add(1, Ordering::Relaxed);
        crate::lock(&self.state.agents).insert(
            agent_id.clone(),
            Connection {
                generation,
                tx: tx.clone(),
            },
        );
        self.state.connects.fetch_add(1, Ordering::Relaxed);

        let hello = ControlMessage {
            payload: Some(control_message::Payload::ServerHello(ServerHello {
                protocol_version_major: restichub_proto::PROTOCOL_VERSION_MAJOR,
                protocol_version_minor: restichub_proto::PROTOCOL_VERSION_MINOR,
                accepted_capabilities: vec![
                    "backup".to_owned(),
                    restichub_proto::AgentCapability::LogAckV1
                        .as_str()
                        .to_owned(),
                ],
                control_plane_version: env!("CARGO_PKG_VERSION").to_owned(),
            })),
        };
        let _ = tx.try_send(hello);

        let state = Arc::clone(&self.state);
        let id = agent_id.clone();
        tokio::spawn(async move {
            while let Ok(Some(message)) = inbound.message().await {
                handle_message(&state, &id, &message, &tx);
            }
            // A reconnect can insert the next stream before this task runs. The
            // remove touches only the stream that this task opened.
            let mut agents = crate::lock(&state.agents);
            if agents
                .get(&id)
                .is_some_and(|connection| connection.generation == generation)
            {
                agents.remove(&id);
            }
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx).map(Ok))))
    }
}

fn handle_message(
    state: &Arc<State>,
    agent_id: &str,
    message: &AgentMessage,
    tx: &mpsc::Sender<ControlMessage>,
) {
    let event = match &message.payload {
        Some(agent_message::Payload::AgentHello(hello)) => Some(AgentEvent::Hello {
            agent_id: agent_id.to_owned(),
            protocol_major: hello.protocol_version_major,
        }),
        Some(agent_message::Payload::JobAck(ack)) => Some(AgentEvent::Ack {
            agent_id: agent_id.to_owned(),
            job_id: ack.job_id.clone(),
        }),
        Some(agent_message::Payload::JobRejected(rejected)) => {
            state.rejected.fetch_add(1, Ordering::Relaxed);
            Some(AgentEvent::Rejected {
                agent_id: agent_id.to_owned(),
                job_id: rejected.job_id.clone(),
                reason: rejected.reason,
            })
        }
        Some(agent_message::Payload::JobResult(result)) => {
            state.results.fetch_add(1, Ordering::Relaxed);
            Some(AgentEvent::Result {
                agent_id: agent_id.to_owned(),
                job_id: result.job_id.clone(),
                status: result.status,
            })
        }
        Some(agent_message::Payload::StepProgress(progress)) => Some(AgentEvent::Progress {
            agent_id: agent_id.to_owned(),
            job_id: progress.job_id.clone(),
            step_id: progress.step_id.clone(),
        }),
        Some(agent_message::Payload::Pong(pong)) => Some(AgentEvent::Pong {
            agent_id: agent_id.to_owned(),
            nonce: pong.nonce,
        }),
        Some(agent_message::Payload::LogChunk(chunk)) => {
            let _ = tx.try_send(ControlMessage {
                payload: Some(control_message::Payload::LogAck(LogAck {
                    job_id: chunk.job_id.clone(),
                    streams: vec![LogStreamAck {
                        stream: chunk.stream,
                        sequence: chunk.sequence,
                    }],
                })),
            });
            Some(AgentEvent::Log {
                agent_id: agent_id.to_owned(),
                job_id: chunk.job_id.clone(),
                sequence: chunk.sequence,
                bytes: chunk.data.len(),
            })
        }
        _ => None,
    };
    if let Some(event) = event {
        let _ = state.events.send(event);
    }
}
