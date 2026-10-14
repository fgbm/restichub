//! The gRPC agent stream (ADR-0008, ADR-0026, FR-ID-04, FR-ID-07, FR-PR-08).
//!
//! One [`AgentService::connect`] stream belongs to one agent. The control plane takes the agent
//! identity only from the client certificate (FR-PR-08): the SAN URI `restichub://tenant/<t>/agent/<a>`
//! and the certificate serial, both checked against the registry row of the tenant (ADR-0026). A
//! revoked or unknown agent is refused, and a revoke closes the open stream at once (FR-ID-04).
//!
//! The stream sends the `ServerHello`, a fresh signing key set and a `Ping` each
//! [`HEARTBEAT_INTERVAL`]. It writes `last_seen_at` in a batch. A `KeyUpdateAck` feeds the retirement
//! of the old key once every known online agent acknowledged the new set (ADR-0026). Job delivery is
//! a typed channel stub for #7; the stream starts no job.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use restichub_db::agents as agent_repo;
use restichub_db::{AgentId, AgentStatus, Database, TenantId};
use restichub_proto::agent::v1::{
    AgentMessage, ControlMessage, Ping, ServerHello, SigningKeyUpdate, agent_message,
    agent_service_server::{AgentService, AgentServiceServer},
    control_message,
};
use tokio::sync::{broadcast, mpsc, watch};
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;
use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};

use crate::job_signing::{JobSigningService, RotationEvent};
use crate::server::AppState;

/// Application heartbeat period (FR-ID-07, spike P4).
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Batch period of the `last_seen_at` write. The stream writes at most once for each period.
pub const LAST_SEEN_INTERVAL: Duration = Duration::from_secs(30);

/// Control messages buffered for one agent.
const OUTBOUND_CHANNEL: usize = 64;

const SAN_PREFIX: &str = "restichub://tenant/";
const SAN_MIDDLE: &str = "/agent/";

/// The identity of a connected agent, taken from the client certificate.
#[derive(Debug, Clone)]
struct PeerIdentity {
    tenant: TenantId,
    agent: AgentId,
    /// Raw bytes of the certificate serial as the registry stores them.
    serial: Vec<u8>,
}

/// One open stream. The generation separates an older stream of the same agent from the new one, so
/// a reconnect removes only its own entry (FR-ID-08).
struct StreamEntry {
    generation: u64,
    close: watch::Sender<bool>,
    out: mpsc::Sender<ControlMessage>,
}

/// The in-process registry of the live streams (ADR-0026, section Revocation; FR-ID-04).
///
/// A revoke closes the stream of an agent at once. A second stream for one agent closes the first.
/// The registry also carries the acknowledgement run of a signing key rotation, so the control plane
/// retires the old key only after every known online agent answered (ADR-0026).
pub struct StreamRegistry {
    inner: Mutex<Inner>,
    next_generation: AtomicU64,
}

#[derive(Default)]
struct Inner {
    streams: HashMap<AgentId, StreamEntry>,
    ack: Option<AckRun>,
}

/// One signing key rotation that waits for the acknowledgements.
struct AckRun {
    /// The old current key that retires when every expected agent acked.
    pending_id: Uuid,
    /// The new current key that the agents acknowledge.
    current_key_id: String,
    expected: HashSet<AgentId>,
    acked: HashSet<AgentId>,
}

impl Default for StreamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for StreamRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StreamRegistry")
            .finish_non_exhaustive()
    }
}

impl StreamRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            next_generation: AtomicU64::new(0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Registers a new stream for an agent and closes the previous one (FR-ID-08).
    fn open(
        &self,
        agent: AgentId,
        close: watch::Sender<bool>,
        out: mpsc::Sender<ControlMessage>,
    ) -> u64 {
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let mut inner = self.lock();
        if let Some(previous) = inner.streams.insert(
            agent,
            StreamEntry {
                generation,
                close,
                out,
            },
        ) {
            let _ = previous.close.send(true);
        }
        generation
    }

    /// Removes the entry of a stream, unless a newer stream replaced it (FR-ID-08).
    fn remove(&self, agent: AgentId, generation: u64) {
        let mut inner = self.lock();
        if inner
            .streams
            .get(&agent)
            .is_some_and(|entry| entry.generation == generation)
        {
            inner.streams.remove(&agent);
        }
    }

    /// Closes the open stream of an agent at once (FR-ID-04).
    ///
    /// True when a stream was open.
    pub fn close_by_agent(&self, agent: AgentId) -> bool {
        self.lock()
            .streams
            .get(&agent)
            .map(|entry| entry.close.send(true).is_ok())
            .unwrap_or(false)
    }

    /// Sends a control message to the open stream of an agent. False when the agent is offline.
    ///
    /// The job dispatcher of #7 delivers `SignedJob` messages through this call.
    pub fn send_to_agent(&self, agent: AgentId, message: ControlMessage) -> bool {
        self.lock()
            .streams
            .get(&agent)
            .map(|entry| entry.out.try_send(message).is_ok())
            .unwrap_or(false)
    }

    #[cfg(test)]
    fn online_agents(&self) -> Vec<AgentId> {
        self.lock().streams.keys().copied().collect()
    }

    /// Records a signing key rotation and the agents that must acknowledge it (ADR-0026).
    ///
    /// The first stream that sees the rotation sets the expected set; a later stream of the same
    /// rotation changes nothing.
    fn note_rotation(&self, pending_id: Uuid, current_key_id: &str) {
        let mut inner = self.lock();
        if inner
            .ack
            .as_ref()
            .is_some_and(|ack| ack.pending_id == pending_id)
        {
            return;
        }
        let expected = inner.streams.keys().copied().collect();
        inner.ack = Some(AckRun {
            pending_id,
            current_key_id: current_key_id.to_owned(),
            expected,
            acked: HashSet::new(),
        });
    }

    /// Records one acknowledgement. Returns the pending key id when every expected agent acked.
    fn ack(&self, agent: AgentId, current_key_id: &str) -> Option<Uuid> {
        let mut inner = self.lock();
        let ack = inner.ack.as_mut()?;
        if ack.current_key_id != current_key_id {
            return None;
        }
        ack.acked.insert(agent);
        if ack.expected.is_subset(&ack.acked) {
            let pending = ack.pending_id;
            inner.ack = None;
            return Some(pending);
        }
        None
    }
}

/// Parses the SAN URI and the serial of the client certificate (ADR-0026, section Identity).
///
/// The chain already validated against the installation root CA during the TLS handshake.
fn peer_identity(der: &[u8]) -> Result<PeerIdentity, Status> {
    let (_, cert) = X509Certificate::from_der(der)
        .map_err(|_| Status::unauthenticated("the client certificate is not valid"))?;
    let san = cert
        .subject_alternative_name()
        .ok()
        .flatten()
        .ok_or_else(|| {
            Status::unauthenticated("the client certificate holds no subject alternative name")
        })?;
    let uri = san
        .value
        .general_names
        .iter()
        .find_map(|name| match name {
            GeneralName::URI(uri) => Some(*uri),
            _ => None,
        })
        .ok_or_else(|| Status::unauthenticated("the client certificate holds no identity URI"))?;
    let (tenant, agent) = parse_identity_uri(uri)
        .ok_or_else(|| Status::unauthenticated("the identity URI is not valid"))?;
    Ok(PeerIdentity {
        tenant,
        agent,
        serial: cert.raw_serial().to_vec(),
    })
}

/// Parses `restichub://tenant/<tenant>/agent/<agent>` (ADR-0026).
fn parse_identity_uri(uri: &str) -> Option<(TenantId, AgentId)> {
    let rest = uri.strip_prefix(SAN_PREFIX)?;
    let (tenant, agent) = rest.split_once(SAN_MIDDLE)?;
    let tenant = Uuid::parse_str(tenant).ok()?;
    let agent = Uuid::parse_str(agent).ok()?;
    Some((TenantId::from_uuid(tenant), AgentId::from_uuid(agent)))
}

/// Checks the registry row of an agent at every stream open (ADR-0026, section Revocation).
///
/// The row must exist in the tenant of the identity URI, be active, list the certificate serial and
/// not be expired. An unknown or revoked agent is refused.
async fn check_registry(
    db: &Database,
    identity: &PeerIdentity,
    now: time::OffsetDateTime,
) -> Result<(), Status> {
    let mut tx = db
        .begin_tenant(identity.tenant)
        .await
        .map_err(|_| Status::internal("the agent registry is not available"))?;
    let agent = agent_repo::get(&mut tx, identity.agent)
        .await
        .map_err(|_| Status::internal("the agent registry is not available"))?;
    tx.commit()
        .await
        .map_err(|_| Status::internal("the agent registry is not available"))?;
    let Some(agent) = agent else {
        return Err(Status::permission_denied("the agent is not enrolled"));
    };
    if agent.status != AgentStatus::Active {
        return Err(Status::permission_denied("the agent is revoked"));
    }
    if normalise_serial(&agent.certificate_serial) != normalise_serial(&identity.serial) {
        return Err(Status::permission_denied(
            "the certificate serial is not the enrolled one",
        ));
    }
    if agent.certificate_expires_at <= now {
        return Err(Status::permission_denied(
            "the agent certificate is expired",
        ));
    }
    Ok(())
}

/// Removes the leading zero bytes of a DER integer, so the stored serial and the certificate serial
/// compare equal.
fn normalise_serial(serial: &[u8]) -> &[u8] {
    let start = serial
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(serial.len());
    &serial[start..]
}

/// The gRPC service of the agent stream (ADR-0008).
#[derive(Clone)]
pub struct AgentStreamService {
    state: AppState,
}

impl AgentStreamService {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl AgentService for AgentStreamService {
    type ConnectStream = Pin<Box<dyn Stream<Item = Result<ControlMessage, Status>> + Send>>;

    async fn connect(
        &self,
        request: Request<Streaming<AgentMessage>>,
    ) -> Result<Response<Self::ConnectStream>, Status> {
        let certs = request
            .peer_certs()
            .ok_or_else(|| Status::unauthenticated("no client certificate"))?;
        let leaf = certs
            .first()
            .ok_or_else(|| Status::unauthenticated("no client certificate"))?;
        let identity = peer_identity(leaf.as_ref())?;
        let now = self.state.auth().clock.now();
        check_registry(self.state.db(), &identity, now).await?;

        let (tx, rx) = mpsc::channel::<ControlMessage>(OUTBOUND_CHANNEL);
        let (close_tx, close_rx) = watch::channel(false);

        let hello = ControlMessage {
            payload: Some(control_message::Payload::ServerHello(ServerHello {
                protocol_version_major: restichub_proto::PROTOCOL_VERSION_MAJOR,
                protocol_version_minor: restichub_proto::PROTOCOL_VERSION_MINOR,
                accepted_capabilities: Vec::new(),
            })),
        };
        let _ = tx.try_send(hello);

        // A fresh key set on every connect, so an agent that was offline re-syncs (ADR-0030).
        if let Some(update) = self.fresh_key_update(now).await {
            let _ = tx.try_send(ControlMessage {
                payload: Some(control_message::Payload::SigningKeyUpdate(update)),
            });
        }

        let generation = self
            .state
            .agent_registry()
            .open(identity.agent, close_tx, tx.clone());
        let rotation = self.state.job_signing().map(JobSigningService::subscribe);

        let mut inbound = request.into_inner();
        let state = self.state.clone();
        let agent = identity.agent;
        let task_identity = identity.clone();
        tokio::spawn(async move {
            run_stream(
                state,
                task_identity,
                agent,
                generation,
                &mut inbound,
                tx,
                close_rx,
                rotation,
            )
            .await;
        });

        let outbound = ReceiverStream::new(rx).map(Ok::<_, Status>);
        Ok(Response::new(Box::pin(outbound)))
    }
}

/// Runs the message loop of one stream: the heartbeat, the `last_seen_at` batch, the inbound
/// messages and the rotation delivery (FR-ID-07, ADR-0026).
#[allow(clippy::too_many_arguments)]
async fn run_stream(
    state: AppState,
    identity: PeerIdentity,
    agent: AgentId,
    generation: u64,
    inbound: &mut Streaming<AgentMessage>,
    tx: mpsc::Sender<ControlMessage>,
    mut close_rx: watch::Receiver<bool>,
    mut rotation: Option<broadcast::Receiver<RotationEvent>>,
) {
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_seen = tokio::time::interval(LAST_SEEN_INTERVAL);
    last_seen.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick of `interval` fires at once. Skip it, so the connect write is the only one.
    heartbeat.tick().await;
    last_seen.tick().await;
    write_last_seen(&state, identity.tenant, agent).await;

    loop {
        tokio::select! {
            message = inbound.message() => match message {
                Ok(Some(message)) => handle_message(&state, &identity, message),
                Ok(None) | Err(_) => break,
            },
            _ = heartbeat.tick() => {
                let nonce = now_nonce();
                let ping = ControlMessage {
                    payload: Some(control_message::Payload::Ping(Ping { nonce })),
                };
                if tx.try_send(ping).is_err() {
                    break;
                }
            }
            _ = last_seen.tick() => write_last_seen(&state, identity.tenant, agent).await,
            event = recv_rotation(&mut rotation) => {
                if let Some(event) = event {
                    deliver_rotation(&state, agent, event).await;
                }
            }
            result = close_rx.changed() => {
                if result.is_err() || *close_rx.borrow() {
                    break;
                }
            }
        }
    }
    // The removal drops the last outbound sender, so the response stream ends (FR-ID-04).
    state.agent_registry().remove(agent, generation);
}

/// Waits for a rotation event. It never resolves when the task is not subscribed or the sender
/// stopped.
async fn recv_rotation(
    rotation: &mut Option<broadcast::Receiver<RotationEvent>>,
) -> Option<RotationEvent> {
    let Some(receiver) = rotation.as_mut() else {
        std::future::pending::<()>().await;
        return None;
    };
    match receiver.recv().await {
        Ok(event) => Some(event),
        // A lagging receiver misses an event; the pending retirement still holds the 7-day bound.
        Err(broadcast::error::RecvError::Lagged(_)) => None,
        Err(broadcast::error::RecvError::Closed) => {
            std::future::pending::<()>().await;
            None
        }
    }
}

/// Delivers a fresh key set after a rotation and records the acknowledgement run (ADR-0026).
async fn deliver_rotation(state: &AppState, agent: AgentId, event: RotationEvent) {
    let Some(signing) = state.job_signing() else {
        return;
    };
    let now = state.auth().clock.now();
    let Ok(update) = signing.key_update(state.db(), now).await else {
        return;
    };
    state
        .agent_registry()
        .note_rotation(event.pending_key_id, &event.current_key_id);
    let message = ControlMessage {
        payload: Some(control_message::Payload::SigningKeyUpdate(update)),
    };
    // The delivering stream may be superseded between the event and this call.
    let _ = state.agent_registry().send_to_agent(agent, message);
}

/// Handles one inbound message (FR-PR-08, ADR-0026).
///
/// The identity comes from the certificate, never from the message. Job messages belong to #7; the
/// stream records an acknowledgement of the key set and ignores a job message.
fn handle_message(state: &AppState, identity: &PeerIdentity, message: AgentMessage) {
    match message.payload {
        Some(agent_message::Payload::KeyUpdateAck(ack)) => {
            if let Some(pending) = state.agent_registry().ack(identity.agent, &ack.key_id) {
                retire(state, pending);
            }
        }
        Some(agent_message::Payload::AgentHello(_) | agent_message::Payload::Pong(_)) => {}
        Some(
            agent_message::Payload::JobAck(_)
            | agent_message::Payload::JobRejected(_)
            | agent_message::Payload::StepProgress(_)
            | agent_message::Payload::LogChunk(_)
            | agent_message::Payload::StepResult(_)
            | agent_message::Payload::JobResult(_)
            | agent_message::Payload::CertificateSigningRequest(_),
        ) => {
            // Job progress, results and certificate rotation belong to #7 and #51.
        }
        None => {}
    }
}

/// Retires the old key after every known online agent acknowledged the new set (ADR-0026).
fn retire(state: &AppState, pending: Uuid) {
    let Some(signing) = state.job_signing() else {
        return;
    };
    let signing = signing.clone();
    let db = state.db().clone();
    let now = state.auth().clock.now();
    tokio::spawn(async move {
        if let Err(error) = signing.retire_acknowledged(&db, pending, now).await {
            tracing::warn!(%error, "the acknowledged key retirement failed");
        }
    });
}

impl AgentStreamService {
    /// Builds the fresh signed key set for a connecting agent (FR-PR-11).
    async fn fresh_key_update(&self, now: time::OffsetDateTime) -> Option<SigningKeyUpdate> {
        let signing = self.state.job_signing()?;
        signing.key_update(self.state.db(), now).await.ok()
    }
}

/// Writes `last_seen_at` in its own transaction (FR-ID-07). A failure is logged, not fatal.
async fn write_last_seen(state: &AppState, tenant: TenantId, agent: AgentId) {
    let now = state.auth().clock.now();
    let Ok(mut tx) = state.db().begin_tenant(tenant).await else {
        return;
    };
    if agent_repo::touch_last_seen(&mut tx, agent, now)
        .await
        .is_ok()
    {
        let _ = tx.commit().await;
    }
}

/// A nonce for the heartbeat. The value is not secret.
fn now_nonce() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0)
}

/// Builds the `AgentServiceServer` of the listener.
pub fn agent_service(state: AppState) -> AgentServiceServer<AgentStreamService> {
    AgentServiceServer::new(AgentStreamService::new(state))
}

#[cfg(test)]
mod tests {
    use super::{StreamRegistry, normalise_serial, parse_identity_uri};
    use restichub_db::AgentId;
    use uuid::Uuid;

    #[test]
    fn the_identity_uri_binds_the_tenant_and_the_agent() {
        let tenant = Uuid::from_u128(0x0189_1234_5678_9abc_def0_1122_3344_5566);
        let agent = Uuid::from_u128(0x0189_aaaa_bbbb_cccc_dddd_eeee_ffff_0000);
        let uri = format!("restichub://tenant/{tenant}/agent/{agent}");
        let parsed = parse_identity_uri(&uri).expect("the URI must parse");
        assert_eq!(parsed.0.as_uuid(), tenant);
        assert_eq!(parsed.1.as_uuid(), agent);
        assert!(parse_identity_uri("restichub://tenant/not-a-uuid/agent/x").is_none());
    }

    #[test]
    fn a_leading_zero_byte_does_not_change_the_serial() {
        assert_eq!(normalise_serial(&[0, 1, 2, 3]), &[1, 2, 3]);
        assert_eq!(normalise_serial(&[1, 2, 3]), &[1, 2, 3]);
        assert!(normalise_serial(&[]).is_empty());
    }

    #[test]
    fn a_second_stream_closes_the_first() {
        let registry = StreamRegistry::new();
        let agent = AgentId::new();
        let (close_a, mut a) = tokio::sync::watch::channel(false);
        let (tx_a, _rx_a) = tokio::sync::mpsc::channel(1);
        registry.open(agent, close_a, tx_a);
        let (close_b, _b) = tokio::sync::watch::channel(false);
        let (tx_b, _rx_b) = tokio::sync::mpsc::channel(1);
        registry.open(agent, close_b, tx_b);
        assert!(*a.borrow_and_update(), "the first stream must close");
        assert_eq!(registry.online_agents(), vec![agent]);
        assert!(registry.close_by_agent(agent));
    }
}
