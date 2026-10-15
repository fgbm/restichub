//! The gRPC agent stream (ADR-0008, ADR-0026, FR-ID-04, FR-ID-07, FR-PR-08).
//!
//! One [`AgentService::connect`] stream belongs to one agent. The control plane takes the agent
//! identity only from the client certificate (FR-PR-08): the SAN URI `restichub://tenant/<t>/agent/<a>`
//! and the certificate serial, both checked against the registry row of the tenant (ADR-0026). A
//! revoked or unknown agent is refused, and a revoke closes the open stream at once (FR-ID-04).
//!
//! The `ServerHello` and the fresh signing key set are queued before the stream is registered, so a
//! job that the registry delivers always follows them on the wire. The registry check and the
//! guarded `last_seen_at` write run before the registration, so a revoke that lands in the window
//! refuses the open. The stream sends a `Ping` each [`HEARTBEAT_INTERVAL`]; it closes when the
//! agent does not answer for [`IDLE_CLOSE`]. A `KeyUpdateAck` feeds the retirement of the old key
//! once every known online agent acknowledged the new set (ADR-0026). A `JobAck`, a `JobRejected`
//! and a `JobResult` update the job row ([`crate::jobs`]). A `StepProgress`, a `StepResult` and a
//! `LogChunk` feed the job detail and the job log ([`crate::job_data`]).

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use restichub_db::agents as agent_repo;
use restichub_db::audit::{self, Actor, AuditAction, AuditEvent};
use restichub_db::queue as queue_repo;
use restichub_db::{AgentId, AgentStatus, Database, JobId, TenantId};
use restichub_proto::AgentCapability;
use restichub_proto::agent::v1::{
    AgentMessage, CancelJob, CertificateRejectReason, CertificateRejected, ControlMessage, LogAck,
    LogChunk, LogStreamAck, Ping, ServerHello, SigningKeyUpdate, VersionRejectReason,
    VersionRejected, agent_message,
    agent_service_server::{AgentService, AgentServiceServer},
    control_message,
};
use restichub_proto::version::{Version, VersionCheck};
use rustls_pki_types::CertificateDer;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;
use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};

use crate::job_signing::{JobSigningService, RotationEvent};
use crate::rotation::PresentedCertificate;
use crate::server::AppState;

/// Application heartbeat period (FR-ID-07, spike P4).
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Batch period of the `last_seen_at` write. The stream writes at most once for each period.
pub const LAST_SEEN_INTERVAL: Duration = Duration::from_secs(30);

/// The stream closes when the agent does not answer for three ping intervals (FR-ID-07).
pub const IDLE_CLOSE: Duration = Duration::from_secs(3 * HEARTBEAT_INTERVAL.as_secs());

/// The agent must send its `AgentHello` within this time, or the stream closes (FR-PR-09).
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// Control messages buffered for one agent.
/// The buffer of the messages that the control plane sends to one agent.
///
/// It holds the greeting and the whole bounded key chain, so the connect handler never blocks on a
/// full buffer before the response stream reaches the client.
const OUTBOUND_CHANNEL: usize = 128;

/// How often each control plane instance checks the registry for the streams that it holds. A
/// revoke on another instance closes the stream within this time plus the query (FR-ID-04,
/// AC-ID-04).
pub const REVOCATION_SWEEP_INTERVAL: Duration = Duration::from_secs(2);

/// The wait between two requests of one stream, answered or refused. It spares the registry a
/// query for a request flood; the limit that holds is in the registry row (FR-ID-03). A short
/// certificate lifetime shortens the wait.
const CERTIFICATE_REQUEST_COOLDOWN: Duration = Duration::from_secs(30);

/// A quarter of the certificate lifetime, but not more than [`CERTIFICATE_REQUEST_COOLDOWN`].
fn certificate_request_cooldown(state: &AppState) -> Duration {
    let quarter = Duration::try_from(state.agent_cert_lifetime() / 4).unwrap_or_default();
    quarter.min(CERTIFICATE_REQUEST_COOLDOWN)
}

/// The largest accepted key identifier in a `KeyUpdateAck`.
const MAX_KEY_ID_LEN: usize = 64;

const SAN_PREFIX: &str = "restichub://tenant/";
const SAN_MIDDLE: &str = "/agent/";

/// The connection information of one stream: the socket peer and the client certificate chain.
///
/// The listener builds it from the TLS session and inserts it as a request extension, so the handler
/// reads the peer address and the certificates without tonic's own TLS path.
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub remote: Option<SocketAddr>,
    pub certs: Option<Arc<Vec<CertificateDer<'static>>>>,
}

/// The identity of a connected agent, taken from the client certificate.
#[derive(Debug, Clone)]
struct PeerIdentity {
    tenant: TenantId,
    agent: AgentId,
    /// The certificate serial without a leading zero, as the registry guard compares it.
    serial: Vec<u8>,
    /// The certificate of the TLS handshake of this stream. A confirmed rotation
    /// replaces it, so the open stream can rotate again.
    presented: PresentedCertificate,
    /// The certificate of the last rotation that waits for its confirmation.
    issued: Option<PresentedCertificate>,
}

impl PeerIdentity {
    fn serial_hex(&self) -> String {
        hex(&self.serial)
    }
}

/// One open stream. The generation separates an older stream of the same agent from the new one, so
/// a reconnect removes only its own entry (FR-ID-08).
struct StreamEntry {
    generation: u64,
    tenant: TenantId,
    /// The serial that the stream authenticated with, in lower-case hexadecimal. A confirmed
    /// rotation replaces it.
    serial_hex: String,
    close: watch::Sender<bool>,
    out: mpsc::Sender<ControlMessage>,
    /// Set by the `AgentHello` of this connection. An entry without it is not online for the
    /// dispatcher (FR-PR-09).
    capabilities: Option<Vec<AgentCapability>>,
    /// Taken from the same hello as the capabilities (FR-RS-03).
    restore_roots: Vec<String>,
}

/// What a job send did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobSend {
    Sent,
    /// The agent holds no ready stream, or its buffer is full.
    Offline,
    /// The live connection lacks a capability that the message needs.
    CapabilityMissing,
}

/// The in-process registry of the live streams (ADR-0026, section Revocation; FR-ID-04).
///
/// A revoke closes the stream of an agent at once. A second stream for one agent closes the first.
/// The registry carries the acknowledgement run of a signing key rotation and the batch buffer of the
/// `last_seen_at` writes.
pub struct StreamRegistry {
    inner: Mutex<Inner>,
    next_generation: AtomicU64,
}

#[derive(Default)]
struct Inner {
    streams: HashMap<AgentId, StreamEntry>,
    ack: Option<AckRun>,
    /// The agents that reported activity since the last flush, with the serial to guard.
    last_seen: HashMap<TenantId, HashMap<AgentId, String>>,
    /// The last acknowledgement that arrived before its rotation was recorded, one per agent.
    pending_acks: HashMap<AgentId, String>,
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
        tenant: TenantId,
        serial_hex: String,
        close: watch::Sender<bool>,
        out: mpsc::Sender<ControlMessage>,
    ) -> u64 {
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let mut inner = self.lock();
        if let Some(previous) = inner.streams.insert(
            agent,
            StreamEntry {
                generation,
                tenant,
                serial_hex,
                close,
                out,
                capabilities: None,
                restore_roots: Vec::new(),
            },
        ) {
            let _ = previous.close.send(true);
        }
        generation
    }

    /// Records the hello facts of the live connection and makes the stream ready for the
    /// dispatcher. A newer stream of the agent keeps its own entry (FR-PR-09).
    fn set_hello(
        &self,
        agent: AgentId,
        generation: u64,
        capabilities: Vec<AgentCapability>,
        restore_roots: Vec<String>,
    ) -> bool {
        let mut inner = self.lock();
        match inner.streams.get_mut(&agent) {
            Some(entry) if entry.generation == generation => {
                entry.capabilities = Some(capabilities);
                entry.restore_roots = restore_roots;
                true
            }
            _ => false,
        }
    }

    /// Reads the hello of the live connection. `None` when the agent is not online. The job builder
    /// reads it here and never from the stored agent row, because a downgraded agent keeps its
    /// identity (FR-PR-09).
    pub fn capabilities(&self, agent: AgentId) -> Option<Vec<AgentCapability>> {
        self.lock()
            .streams
            .get(&agent)
            .and_then(|entry| entry.capabilities.clone())
    }

    /// Roots of the live connection, or `None` when the agent is not online (FR-RS-03).
    pub fn restore_roots(&self, agent: AgentId) -> Option<Vec<String>> {
        let inner = self.lock();
        let entry = inner.streams.get(&agent)?;
        entry.capabilities.as_ref()?;
        Some(entry.restore_roots.clone())
    }

    /// Records the serial that a confirmed rotation made the enrolled one, so the sweep does not
    /// close the stream (ADR-0026).
    fn set_serial(&self, agent: AgentId, generation: u64, serial_hex: String) {
        if let Some(entry) = self.lock().streams.get_mut(&agent)
            && entry.generation == generation
        {
            entry.serial_hex = serial_hex;
        }
    }

    /// The open streams with their tenant and serial, for the revocation sweep (FR-ID-04).
    fn open_streams(&self) -> Vec<(TenantId, AgentId, String)> {
        self.lock()
            .streams
            .iter()
            .map(|(agent, entry)| (entry.tenant, *agent, entry.serial_hex.clone()))
            .collect()
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

    /// Closes the open streams that still hold the serial of the snapshot. The batch flush calls it
    /// for a revoked or an expired row (FR-ID-04, FR-ID-07). A confirmation that changed the serial
    /// after the snapshot keeps its stream, because the registry query saw the old serial.
    fn close_many_with_serial(&self, streams: &[(AgentId, String)]) {
        let inner = self.lock();
        for (agent, serial) in streams {
            if let Some(entry) = inner.streams.get(agent)
                && entry.serial_hex == *serial
            {
                let _ = entry.close.send(true);
            }
        }
    }

    /// Like [`cancel_jobs_and_close`], for a stream that still holds the serial of the snapshot.
    /// The check and the sends share one lock, so a confirmation cannot slip in between and the
    /// agent gets no cancellation on a stream that stays (FR-ID-03, FR-ID-04).
    fn cancel_jobs_and_close_with_serial(
        &self,
        agent: AgentId,
        serial: &str,
        jobs: &[JobId],
    ) -> bool {
        let inner = self.lock();
        let Some(entry) = inner.streams.get(&agent) else {
            return false;
        };
        if entry.serial_hex != serial {
            return false;
        }
        for job in jobs {
            let _ = entry.out.try_send(cancel_message(*job));
        }
        entry.close.send(true).is_ok()
    }

    /// Sends a control message to the open stream of an agent. False when the agent is offline.
    ///
    /// The queue dispatcher delivers `SignedJob` messages through this call.
    pub fn send_to_agent(&self, agent: AgentId, message: ControlMessage) -> bool {
        self.send_requiring(agent, message, &[]) == JobSend::Sent
    }

    /// Like [`Self::send_to_agent`], and refuses when the live connection lacks one of the
    /// capabilities. The check and the send share one lock, so a reconnect cannot slip in between
    /// (FR-PR-09).
    pub fn send_requiring(
        &self,
        agent: AgentId,
        message: ControlMessage,
        required: &[AgentCapability],
    ) -> JobSend {
        let inner = self.lock();
        let Some(entry) = inner.streams.get(&agent) else {
            return JobSend::Offline;
        };
        let Some(capabilities) = &entry.capabilities else {
            return JobSend::Offline;
        };
        if required.iter().any(|need| !capabilities.contains(need)) {
            return JobSend::CapabilityMissing;
        }
        if entry.out.try_send(message).is_ok() {
            JobSend::Sent
        } else {
            JobSend::Offline
        }
    }

    /// True when the agent holds an open stream at this moment.
    ///
    /// The on-demand job dispatcher checks it before it reads a secret, so an offline agent gives a
    /// clear error and no delivery record.
    pub fn is_online(&self, agent: AgentId) -> bool {
        self.lock()
            .streams
            .get(&agent)
            .is_some_and(|entry| entry.capabilities.is_some())
    }

    /// The agents that hold an open stream at this moment. The queue dispatcher sends to these only.
    pub fn online_agents(&self) -> Vec<AgentId> {
        self.lock()
            .streams
            .iter()
            .filter(|(_, entry)| entry.capabilities.is_some())
            .map(|(agent, _)| *agent)
            .collect()
    }

    /// Buffers one `last_seen_at` write. The flush task writes the buffer in one statement per tenant.
    pub fn mark_last_seen(&self, tenant: TenantId, agent: AgentId, serial_hex: String) {
        self.lock()
            .last_seen
            .entry(tenant)
            .or_default()
            .insert(agent, serial_hex);
    }

    /// Takes the buffered `last_seen_at` writes, grouped by tenant.
    fn take_last_seen(&self) -> Vec<(TenantId, Vec<(AgentId, String)>)> {
        let mut inner = self.lock();
        std::mem::take(&mut inner.last_seen)
            .into_iter()
            .map(|(tenant, agents)| (tenant, agents.into_iter().collect()))
            .collect()
    }

    /// Records a signing key rotation and the agents that must acknowledge it (ADR-0026).
    ///
    /// The first stream that sees the rotation sets the expected set; a later stream of the same
    /// rotation changes nothing. An acknowledgement that arrived first is applied now. Returns the
    /// pending key id when the acknowledgements are already complete.
    fn note_rotation(&self, pending_id: Uuid, current_key_id: &str) -> Option<Uuid> {
        let mut inner = self.lock();
        if inner
            .ack
            .as_ref()
            .is_some_and(|ack| ack.pending_id == pending_id)
        {
            return None;
        }
        let expected: HashSet<AgentId> = inner.streams.keys().copied().collect();
        let acked: HashSet<AgentId> = inner
            .pending_acks
            .iter()
            .filter(|(_, key_id)| key_id.as_str() == current_key_id)
            .map(|(agent, _)| *agent)
            .collect();
        inner
            .pending_acks
            .retain(|_, key_id| key_id.as_str() != current_key_id);
        if expected.is_subset(&acked) {
            inner.ack = None;
            return Some(pending_id);
        }
        inner.ack = Some(AckRun {
            pending_id,
            current_key_id: current_key_id.to_owned(),
            expected,
            acked,
        });
        None
    }

    /// Records one acknowledgement. Returns the pending key id when every expected agent acked.
    ///
    /// An acknowledgement that arrives before the rotation is recorded is kept for the rotation.
    fn ack(&self, agent: AgentId, current_key_id: &str) -> Option<Uuid> {
        let mut inner = self.lock();
        if let Some(run) = inner.ack.as_mut()
            && run.current_key_id == current_key_id
        {
            run.acked.insert(agent);
            if run.expected.is_subset(&run.acked) {
                let pending = run.pending_id;
                inner.ack = None;
                return Some(pending);
            }
            return None;
        }
        inner.pending_acks.insert(agent, current_key_id.to_owned());
        None
    }

    #[cfg(test)]
    fn pending_ack_count(&self) -> usize {
        self.lock().pending_acks.len()
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
    let serial = normalise_serial(cert.raw_serial()).to_vec();
    let presented = PresentedCertificate {
        serial: serial.clone(),
        public_key: cert.public_key().subject_public_key.data.to_vec(),
        not_before: cert.validity().not_before.to_datetime(),
        not_after: cert.validity().not_after.to_datetime(),
    };
    Ok(PeerIdentity {
        tenant,
        agent,
        serial,
        presented,
        issued: None,
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
/// not be expired. An unknown or revoked agent is refused. A certificate that a rotation issued
/// and the agent did not confirm yet is accepted too: the agent proves with it that it stored the
/// new key, so the check makes it the enrolled one and the old serial is refused from then on
/// (FR-ID-03).
async fn check_registry(
    db: &Database,
    identity: &PeerIdentity,
    now: time::OffsetDateTime,
) -> Result<(), Status> {
    let unavailable =
        |_: restichub_db::DbError| Status::internal("the agent registry is not available");
    let mut tx = db
        .begin_tenant(identity.tenant)
        .await
        .map_err(unavailable)?;
    let agent = agent_repo::get_for_update(&mut tx, identity.agent)
        .await
        .map_err(unavailable)?;
    let Some(agent) = agent else {
        return Err(Status::permission_denied("the agent is not enrolled"));
    };
    if agent.status != AgentStatus::Active {
        return Err(Status::permission_denied("the agent is revoked"));
    }
    if normalise_serial(&agent.certificate_serial) == identity.serial.as_slice() {
        if agent.certificate_expires_at <= now {
            return Err(Status::permission_denied(
                "the agent certificate is expired",
            ));
        }
    } else {
        let pending = agent
            .pending_certificate_serial
            .as_deref()
            .is_some_and(|serial| normalise_serial(serial) == identity.serial.as_slice());
        let promoted = pending
            && agent_repo::promote_pending_certificate(
                &mut tx,
                identity.agent,
                &identity.serial,
                now,
            )
            .await
            .map_err(unavailable)?;
        if !promoted {
            if report_superseded_use(&mut tx, identity, &agent, now).await {
                let _ = tx.commit().await;
            }
            return Err(Status::permission_denied(
                "the certificate serial is not the enrolled one",
            ));
        }
    }
    tx.commit().await.map_err(unavailable)?;
    Ok(())
}

/// How long one superseded-certificate event silences the next one for the same agent.
const SUPERSEDED_EVENT_QUIET: time::Duration = time::Duration::hours(1);

/// Records a connection that presented the serial the last rotation replaced (T17).
///
/// The honest agent drops the old certificate after the confirmation, so this connection means a
/// copy of the old key or an agent that lost its new identity. The refusal stays; the audit event
/// and the Hosts mark tell the operator. `true` when the transaction holds a change to commit.
async fn report_superseded_use(
    tx: &mut restichub_db::TenantTx<'_>,
    identity: &PeerIdentity,
    agent: &agent_repo::Agent,
    now: time::OffsetDateTime,
) -> bool {
    let marked = agent_repo::mark_superseded_certificate_used(
        tx,
        identity.agent,
        &identity.serial,
        now,
        now - SUPERSEDED_EVENT_QUIET,
    )
    .await;
    if !matches!(marked, Ok(true)) {
        return false;
    }
    let mut event = AuditEvent::new(AuditAction::AgentSupersededCertificateUsed, "agent");
    event.actor = Actor::Agent(identity.agent.as_uuid());
    event.object_id = Some(identity.agent.as_uuid());
    event.diff = Some(audit::diff(
        None,
        Some(&serde_json::json!({
            "presented_certificate_serial": identity.serial_hex(),
            "enrolled_certificate_serial": hex(&agent.certificate_serial),
        })),
        &[],
    ));
    if let Err(error) = audit::insert(tx, &event).await {
        tracing::warn!(agent = %identity.agent, cause = ?error, "cannot write the superseded certificate event");
        return false;
    }
    tracing::warn!(agent = %identity.agent, "a superseded agent certificate was presented");
    true
}

/// Writes the first `last_seen_at` under the registry guard.
///
/// The single statement re-checks the status, the serial and the expiry. `false` refuses the stream.
async fn touch_connected(
    db: &Database,
    identity: &PeerIdentity,
    now: time::OffsetDateTime,
) -> bool {
    let Ok(mut tx) = db.begin_tenant(identity.tenant).await else {
        return false;
    };
    match agent_repo::touch_last_seen_guarded(&mut tx, identity.agent, &identity.serial_hex(), now)
        .await
    {
        Ok(true) => {
            let _ = tx.commit().await;
            true
        }
        Ok(false) | Err(_) => false,
    }
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

/// Lower-case hexadecimal, as the registry guard compares the serial.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
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
        let info = request
            .extensions()
            .get::<PeerInfo>()
            .cloned()
            .ok_or_else(|| Status::unauthenticated("the connection holds no client identity"))?;
        let certs = info
            .certs
            .ok_or_else(|| Status::unauthenticated("no client certificate"))?;
        let leaf = certs
            .first()
            .ok_or_else(|| Status::unauthenticated("no client certificate"))?;
        let identity = peer_identity(leaf.as_ref())?;

        // Subscribe before the fresh key set, so a rotation in between is not lost.
        let rotation = self.state.job_signing().map(JobSigningService::subscribe);

        let now = self.state.auth().clock.now();
        // Check the registry before the registration: a refused connection must not touch the live
        // stream of the same agent (FR-ID-04).
        check_registry(self.state.db(), &identity, now).await?;

        let (tx, rx) = mpsc::channel::<ControlMessage>(OUTBOUND_CHANNEL);
        // Queue the ServerHello and the fresh key set before the stream is registered, so a job
        // that the registry delivers always follows them on the wire (FR-PR-09, ADR-0030).
        let hello = ControlMessage {
            payload: Some(control_message::Payload::ServerHello(ServerHello {
                protocol_version_major: restichub_proto::PROTOCOL_VERSION_MAJOR,
                protocol_version_minor: restichub_proto::PROTOCOL_VERSION_MINOR,
                accepted_capabilities: vec![AgentCapability::LogAckV1.as_str().to_owned()],
                control_plane_version: crate::agent_version::CONTROL_PLANE_VERSION.to_owned(),
            })),
        };
        let _ = tx.try_send(hello);

        // The whole key chain on every connect, so an agent that missed several rotations re-syncs
        // (ADR-0030). The agent applies the steps in order. The chain is bounded and the buffer
        // holds it, so the awaited send never blocks before the response stream reaches the client.
        for update in self.fresh_key_updates().await {
            let _ = tx
                .send(ControlMessage {
                    payload: Some(control_message::Payload::SigningKeyUpdate(update)),
                })
                .await;
        }

        // The guarded write re-checks the status, the serial and the expiry in one statement. A
        // revoke or an expiry in the window refuses the open, so no stream is registered (FR-ID-04).
        if !touch_connected(self.state.db(), &identity, now).await {
            return Err(Status::permission_denied("the agent is revoked or expired"));
        }

        let (close_tx, close_rx) = watch::channel(false);
        let generation = self.state.agent_registry().open(
            identity.agent,
            identity.tenant,
            identity.serial_hex(),
            close_tx,
            tx.clone(),
        );

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

/// The most restore roots of one agent and the longest root that the control plane keeps.
const MAX_RESTORE_ROOTS: usize = 32;
const MAX_ROOT_CHARS: usize = 1_024;

/// Bounds the roots that an agent reported. A value that the control plane cannot show is dropped.
fn clean_roots(roots: &[String]) -> Vec<String> {
    roots
        .iter()
        .filter(|root| !root.is_empty() && root.chars().count() <= MAX_ROOT_CHARS)
        .filter(|root| !restichub_proto::validate::has_forbidden(root))
        .take(MAX_RESTORE_ROOTS)
        .cloned()
        .collect()
}

/// Runs the message loop of one stream: the heartbeat, the `last_seen_at` batch, the inbound
/// messages and the rotation delivery (FR-ID-07, ADR-0026).
///
/// The stream closes when the agent sends nothing for [`IDLE_CLOSE`]. `last_seen_at` is
/// buffered only for a stream that reported activity since the last tick.
#[allow(clippy::too_many_arguments)]
async fn run_stream(
    state: AppState,
    mut identity: PeerIdentity,
    agent: AgentId,
    generation: u64,
    inbound: &mut Streaming<AgentMessage>,
    tx: mpsc::Sender<ControlMessage>,
    mut close_rx: watch::Receiver<bool>,
    mut rotation: Option<broadcast::Receiver<RotationEvent>>,
) {
    // The dispatcher ignores the stream until the hello told which capabilities this connection
    // has (FR-PR-09).
    let hello = tokio::select! {
        message = tokio::time::timeout(HELLO_TIMEOUT, inbound.message()) => match message {
            Ok(Ok(Some(AgentMessage {
                payload: Some(agent_message::Payload::AgentHello(hello)),
            }))) => Some(hello),
            _ => None,
        },
        _ = close_rx.changed() => None,
    };
    let Some(hello) = hello else {
        state.agent_registry().remove(agent, generation);
        return;
    };
    let capabilities = AgentCapability::parse_list(&hello.capabilities);
    let restore_roots = clean_roots(&hello.restore_roots);
    let scripts = crate::scripts::from_hello(&hello, &capabilities);
    // The registry keeps the facts of a refused hello too, so the Hosts screen can mark the agent
    // as unsupported (FR-PR-09).
    crate::agents::record_hello_facts(
        &state,
        identity.tenant,
        agent,
        &hello,
        &capabilities,
        &restore_roots,
        &scripts,
    )
    .await;
    let version = Version {
        major: hello.protocol_version_major,
        minor: hello.protocol_version_minor,
    };
    if let Some(reason) = refusal_reason(Version::CURRENT.check_agent(version)) {
        tracing::warn!(
            %agent,
            agent_major = version.major,
            agent_minor = version.minor,
            "the agent protocol version is refused"
        );
        let _ = tx.try_send(ControlMessage {
            payload: Some(control_message::Payload::VersionRejected(VersionRejected {
                reason: reason as i32,
                server_protocol_version_major: Version::CURRENT.major,
                server_protocol_version_minor: Version::CURRENT.minor,
                minimum_protocol_version_minor: Version::CURRENT.minimum_minor(),
            })),
        });
        state.agent_registry().remove(agent, generation);
        return;
    }
    if !state.agent_registry().set_hello(
        agent,
        generation,
        capabilities.clone(),
        restore_roots.clone(),
    ) {
        return;
    }
    // An older agent sends no list, so its silence says nothing about its jobs (ADR-0039).
    if AgentCapability::JobLimitsV1.is_listed(&hello.capabilities) {
        crate::queue::stale::fail_unlisted_running(
            &state,
            identity.tenant,
            agent,
            &hello.held_job_ids,
        )
        .await;
    }

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_seen = tokio::time::interval(LAST_SEEN_INTERVAL);
    last_seen.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick of `interval` fires at once. Skip it.
    heartbeat.tick().await;
    last_seen.tick().await;
    let mut last_activity = tokio::time::Instant::now();
    let mut last_request: Option<tokio::time::Instant> = None;
    let mut log_acks = LogAcks::new(capabilities.contains(&AgentCapability::LogAckV1));
    let mut active = false;

    loop {
        tokio::select! {
            message = inbound.message() => match message {
                Ok(Some(message)) => {
                    last_activity = tokio::time::Instant::now();
                    active = true;
                    handle_message(
                        &state,
                        &mut identity,
                        generation,
                        &tx,
                        &mut last_request,
                        &mut log_acks,
                        message,
                    )
                    .await;
                }
                Ok(None) | Err(_) => break,
            },
            _ = heartbeat.tick() => {
                if last_activity.elapsed() >= IDLE_CLOSE {
                    break;
                }
                let nonce = now_nonce();
                let ping = ControlMessage {
                    payload: Some(control_message::Payload::Ping(Ping { nonce })),
                };
                if tx.try_send(ping).is_err() {
                    break;
                }
            }
            _ = last_seen.tick() => {
                if active {
                    state.agent_registry().mark_last_seen(
                        identity.tenant,
                        agent,
                        identity.serial_hex(),
                    );
                    active = false;
                }
            }
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

/// Delivers the fresh key set after a rotation and records the acknowledgement run (ADR-0026).
///
/// The delivery uses the stored step of the new next key, so it is the same signed message that a
/// later re-sync delivers (ADR-0030).
async fn deliver_rotation(state: &AppState, agent: AgentId, event: RotationEvent) {
    let Some(signing) = state.job_signing() else {
        return;
    };
    let Ok(update) = signing.current_step(state.db()).await else {
        return;
    };
    if let Some(pending_key_id) = event.pending_key_id
        && let Some(pending) = state
            .agent_registry()
            .note_rotation(pending_key_id, &event.current_key_id)
    {
        retire(state, pending);
    }
    let message = ControlMessage {
        payload: Some(control_message::Payload::SigningKeyUpdate(update)),
    };
    // The delivering stream may be superseded between the event and this call.
    let _ = state.agent_registry().send_to_agent(agent, message);
}

/// The acknowledgements of log chunks on one connection (FR-LG-07).
///
/// An acknowledgement covers every chunk of the stream up to its sequence, because the agent sends
/// the chunks in order. A chunk that fails to store therefore stops the acknowledgements of its job
/// on this connection; the agent sends the chunks again on the next one.
struct LogAcks {
    enabled: bool,
    failed: HashSet<String>,
}

impl LogAcks {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            failed: HashSet::new(),
        }
    }

    fn after_store(&mut self, tx: &mpsc::Sender<ControlMessage>, chunk: &LogChunk, stored: bool) {
        if !self.enabled {
            return;
        }
        if !stored {
            if self.failed.len() < MAX_FAILED_LOG_JOBS {
                self.failed.insert(chunk.job_id.clone());
            }
            return;
        }
        if self.failed.contains(&chunk.job_id) {
            return;
        }
        // A full buffer drops the acknowledgement. The agent sends the chunk again later.
        let _ = tx.try_send(ControlMessage {
            payload: Some(control_message::Payload::LogAck(LogAck {
                job_id: chunk.job_id.clone(),
                streams: vec![LogStreamAck {
                    stream: chunk.stream,
                    sequence: chunk.sequence,
                }],
            })),
        });
    }
}

/// The most jobs with a failed log write that one connection remembers.
const MAX_FAILED_LOG_JOBS: usize = 1024;

/// Handles one inbound message (FR-PR-08, ADR-0026).
///
/// The identity comes from the certificate, never from the message. An acknowledgement is accepted
/// only for the current key identifier of the signing service and within the length bound, so a
/// flood of other identifiers cannot grow the registry.
async fn handle_message(
    state: &AppState,
    identity: &mut PeerIdentity,
    generation: u64,
    tx: &mpsc::Sender<ControlMessage>,
    last_request: &mut Option<tokio::time::Instant>,
    log_acks: &mut LogAcks,
    message: AgentMessage,
) {
    match message.payload {
        Some(agent_message::Payload::KeyUpdateAck(ack)) => {
            if ack.key_id.len() > MAX_KEY_ID_LEN {
                return;
            }
            let Some(signing) = state.job_signing() else {
                return;
            };
            if signing.current_key_id(state.db()).await.as_deref() != Some(ack.key_id.as_str()) {
                return;
            }
            if let Some(pending) = state.agent_registry().ack(identity.agent, &ack.key_id) {
                retire(state, pending);
            }
        }
        Some(agent_message::Payload::CertificateSigningRequest(request)) => {
            if last_request.is_some_and(|at| at.elapsed() < certificate_request_cooldown(state)) {
                reject_certificate(tx, CertificateRejectReason::TooEarly);
                return;
            }
            *last_request = Some(tokio::time::Instant::now());
            let now = state.auth().clock.now();
            let payload = match crate::rotation::issue(
                state,
                identity.tenant,
                identity.agent,
                &identity.presented,
                &request.csr_der,
                now,
            )
            .await
            {
                Ok((issued, next)) => {
                    identity.issued = Some(next);
                    control_message::Payload::CertificateIssued(issued)
                }
                Err(reason) => {
                    tracing::info!(agent = %identity.agent, ?reason, "the certificate rotation is refused");
                    control_message::Payload::CertificateRejected(CertificateRejected {
                        reason: reason as i32,
                    })
                }
            };
            let _ = tx.try_send(ControlMessage {
                payload: Some(payload),
            });
        }
        Some(agent_message::Payload::CertificateInstalled(installed)) => {
            let serial = normalise_serial(&installed.certificate_serial).to_vec();
            let now = state.auth().clock.now();
            match crate::rotation::confirm(state, identity.tenant, identity.agent, &serial, now)
                .await
            {
                Ok(true) => {
                    // The open stream keeps running under the new serial; the old certificate is
                    // refused for every later connection (ADR-0026).
                    state
                        .agent_registry()
                        .set_serial(identity.agent, generation, hex(&serial));
                    if let Some(next) = identity.issued.take()
                        && next.serial == serial
                    {
                        identity.presented = next;
                    }
                    identity.serial = serial;
                }
                Ok(false) => {
                    tracing::info!(agent = %identity.agent, "the certificate confirmation names no pending certificate");
                }
                Err(error) => {
                    tracing::warn!(agent = %identity.agent, cause = ?error, "the certificate confirmation failed");
                }
            }
        }
        // The first message was the hello. Capabilities do not change within a connection.
        Some(agent_message::Payload::AgentHello(_)) => {}
        Some(agent_message::Payload::Pong(_)) => {}
        Some(agent_message::Payload::JobAck(ack)) => {
            crate::jobs::handle_job_ack(state, identity.tenant, identity.agent, &ack).await;
        }
        Some(agent_message::Payload::JobRejected(rejected)) => {
            crate::jobs::handle_job_rejected(state, identity.tenant, identity.agent, &rejected)
                .await;
        }
        Some(agent_message::Payload::JobResult(result)) => {
            crate::job_data::handle_job_result_data(
                state,
                identity.tenant,
                identity.agent,
                &result,
            )
            .await;
            crate::jobs::handle_job_result(state, identity.tenant, identity.agent, &result).await;
        }
        Some(agent_message::Payload::StepResult(result)) => {
            crate::job_data::handle_step_result(state, identity.tenant, identity.agent, &result)
                .await;
            crate::queue::stale::handle_step_result(
                state,
                identity.tenant,
                identity.agent,
                &result,
            )
            .await;
        }
        Some(agent_message::Payload::StepProgress(progress)) => {
            crate::job_data::handle_progress(state, identity.tenant, identity.agent, &progress)
                .await;
        }
        Some(agent_message::Payload::LogChunk(chunk)) => {
            let stored =
                crate::job_data::handle_log_chunk(state, identity.tenant, identity.agent, &chunk)
                    .await;
            log_acks.after_store(tx, &chunk, stored);
        }
        None => {}
    }
}

fn reject_certificate(tx: &mpsc::Sender<ControlMessage>, reason: CertificateRejectReason) {
    let _ = tx.try_send(ControlMessage {
        payload: Some(control_message::Payload::CertificateRejected(
            CertificateRejected {
                reason: reason as i32,
            },
        )),
    });
}

fn refusal_reason(check: VersionCheck) -> Option<VersionRejectReason> {
    match check {
        VersionCheck::Compatible => None,
        VersionCheck::MajorMismatch => Some(VersionRejectReason::MajorMismatch),
        VersionCheck::MinorTooOld => Some(VersionRejectReason::MinorTooOld),
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
    /// Builds the key chain for a connecting agent (ADR-0030, FR-PR-11).
    ///
    /// The chain starts at the first key, so an agent that missed several rotations catches up. An
    /// agent that already holds the earlier keys skips those steps.
    async fn fresh_key_updates(&self) -> Vec<SigningKeyUpdate> {
        let Some(signing) = self.state.job_signing() else {
            return Vec::new();
        };
        match signing.key_chain(self.state.db()).await {
            Ok(chain) => chain,
            Err(error) => {
                tracing::warn!(%error, "cannot build the job signing key chain");
                Vec::new()
            }
        }
    }
}

/// Sends a cancellation for each running job of an agent and closes its stream (FR-ID-04,
/// AC-ID-04).
///
/// The messages are queued before the close, and the response stream drains its buffer before it
/// ends, so the agent gets the cancellations first. True when a stream was open.
pub fn cancel_jobs_and_close(registry: &StreamRegistry, agent: AgentId, jobs: &[JobId]) -> bool {
    for job in jobs {
        let _ = registry.send_to_agent(agent, cancel_message(*job));
    }
    registry.close_by_agent(agent)
}

fn cancel_message(job: JobId) -> ControlMessage {
    ControlMessage {
        payload: Some(control_message::Payload::CancelJob(CancelJob {
            job_id: job.to_string(),
            grace_period_seconds: crate::job_api::CANCEL_GRACE_SECONDS,
        })),
    }
}

/// Closes the open streams that the registry no longer allows (FR-ID-04).
///
/// A revoke on another control plane instance reaches this instance only through the registry
/// row, so every instance checks the rows of its own streams. The check also catches a stream
/// whose serial a rotation on another instance replaced.
pub async fn sweep_revoked(state: &AppState) {
    let mut by_tenant: HashMap<TenantId, Vec<(AgentId, String)>> = HashMap::new();
    for (tenant, agent, serial) in state.agent_registry().open_streams() {
        by_tenant.entry(tenant).or_default().push((agent, serial));
    }
    for (tenant, streams) in by_tenant {
        let ids: Vec<Uuid> = streams.iter().map(|(agent, _)| agent.as_uuid()).collect();
        let serials: Vec<String> = streams.iter().map(|(_, serial)| serial.clone()).collect();
        let result = async {
            let mut tx = state.db().begin_tenant(tenant).await?;
            let stale = agent_repo::disallowed_streams(&mut tx, &ids, &serials).await?;
            let mut jobs = Vec::with_capacity(stale.len());
            for id in stale {
                let agent = AgentId::from_uuid(id);
                jobs.push((
                    agent,
                    queue_repo::active_job_ids_for_agent(&mut tx, agent).await?,
                ));
            }
            tx.commit().await?;
            Ok::<_, restichub_db::DbError>(jobs)
        }
        .await;
        match result {
            Ok(closing) => {
                for (agent, jobs) in closing {
                    let Some((_, serial)) = streams.iter().find(|(id, _)| *id == agent) else {
                        continue;
                    };
                    state
                        .agent_registry()
                        .cancel_jobs_and_close_with_serial(agent, serial, &jobs);
                }
            }
            Err(error) => tracing::warn!(cause = ?error, "the revocation sweep failed"),
        }
    }
}

/// Starts the periodic revocation sweep of the streams of this instance (FR-ID-04).
pub fn spawn_revocation_sweep(state: AppState, interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The first tick of `interval` fires immediately. Skip it.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            sweep_revoked(&state).await;
            if let Some(signing) = state.job_signing() {
                signing.announce_remote_change(state.db()).await;
            }
        }
    })
}

/// Flushes the buffered `last_seen_at` writes in one statement for each tenant.
///
/// A row that the guarded statement did not update is revoked, expired or no longer the enrolled
/// serial; the flush closes its stream.
pub async fn flush_last_seen(db: &Database, registry: &StreamRegistry, now: time::OffsetDateTime) {
    for (tenant, entries) in registry.take_last_seen() {
        let ids: Vec<Uuid> = entries.iter().map(|(agent, _)| agent.as_uuid()).collect();
        let serials: Vec<String> = entries.iter().map(|(_, serial)| serial.clone()).collect();
        let Ok(mut tx) = db.begin_tenant(tenant).await else {
            continue;
        };
        match agent_repo::touch_last_seen_batch(&mut tx, &ids, &serials, now).await {
            Ok(updated) => {
                let _ = tx.commit().await;
                let updated: HashSet<Uuid> = updated.into_iter().collect();
                let stale: Vec<(AgentId, String)> = entries
                    .into_iter()
                    .filter(|(agent, _)| !updated.contains(&agent.as_uuid()))
                    .collect();
                if !stale.is_empty() {
                    registry.close_many_with_serial(&stale);
                }
            }
            Err(error) => {
                tracing::warn!(%error, "the last_seen batch failed");
            }
        }
    }
}

/// Starts the periodic flush of the buffered `last_seen_at` writes.
pub fn spawn_last_seen_flush(
    db: Database,
    registry: Arc<StreamRegistry>,
    clock: Arc<dyn crate::clock::Clock>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // The first tick of `interval` fires immediately. Skip it.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            flush_last_seen(&db, &registry, clock.now()).await;
        }
    })
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
        .max_decoding_message_size(restichub_proto::MAX_MESSAGE_BYTES)
        .max_encoding_message_size(restichub_proto::MAX_MESSAGE_BYTES)
}

#[cfg(test)]
mod tests {
    use super::{StreamRegistry, hex, normalise_serial, parse_identity_uri};
    use restichub_db::{AgentId, JobId, TenantId};
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
        assert_eq!(hex(&[0x0a, 0xff]), "0aff");
    }

    #[test]
    fn a_second_stream_closes_the_first() {
        let registry = StreamRegistry::new();
        let agent = AgentId::new();
        let (close_a, mut a) = tokio::sync::watch::channel(false);
        let (tx_a, _rx_a) = tokio::sync::mpsc::channel(1);
        registry.open(agent, TenantId::new(), String::new(), close_a, tx_a);
        let (close_b, _b) = tokio::sync::watch::channel(false);
        let (tx_b, _rx_b) = tokio::sync::mpsc::channel(1);
        let generation = registry.open(agent, TenantId::new(), String::new(), close_b, tx_b);
        assert!(*a.borrow_and_update(), "the first stream must close");
        assert!(registry.online_agents().is_empty(), "no hello yet");
        assert!(registry.set_hello(agent, generation, Vec::new(), Vec::new()));
        assert_eq!(registry.online_agents(), vec![agent]);
        assert!(registry.close_by_agent(agent));
    }

    /// FR-ID-03: a confirmation that lands between the snapshot and the registry query keeps the
    /// stream alive and sends no cancellation.
    #[test]
    fn fr_id_03_a_confirmation_after_the_snapshot_keeps_the_stream() {
        let registry = StreamRegistry::new();
        let agent = AgentId::new();
        let (close, mut closed) = tokio::sync::watch::channel(false);
        let (tx, mut out) = tokio::sync::mpsc::channel(4);
        let generation = registry.open(agent, TenantId::new(), "aa".to_owned(), close, tx);
        let snapshot = registry.open_streams();
        assert_eq!(snapshot[0].2, "aa");

        registry.set_serial(agent, generation, "bb".to_owned());
        let jobs = [JobId::new()];
        assert!(!registry.cancel_jobs_and_close_with_serial(agent, "aa", &jobs));
        registry.close_many_with_serial(&[(agent, "aa".to_owned())]);
        assert!(!*closed.borrow_and_update(), "the stream must stay open");
        assert!(out.try_recv().is_err(), "no cancellation may be sent");

        assert!(registry.cancel_jobs_and_close_with_serial(agent, "bb", &jobs));
        assert!(*closed.borrow_and_update());
        assert!(
            out.try_recv().is_ok(),
            "the cancellation precedes the close"
        );
    }

    #[test]
    fn an_acknowledgement_before_the_rotation_is_applied() {
        let registry = StreamRegistry::new();
        let agent = AgentId::new();
        let (close, _rx) = tokio::sync::watch::channel(false);
        let (tx, _out) = tokio::sync::mpsc::channel(1);
        registry.open(agent, TenantId::new(), String::new(), close, tx);
        // The acknowledgement arrives before the rotation is recorded.
        assert!(registry.ack(agent, "key-2").is_none());
        let pending = Uuid::now_v7();
        assert_eq!(registry.note_rotation(pending, "key-2"), Some(pending));
    }

    /// Many acknowledgements keep at most one pending entry for each agent.
    #[test]
    fn many_acknowledgements_keep_one_pending_entry_for_each_agent() {
        let registry = StreamRegistry::new();
        let agent = AgentId::new();
        for index in 0..10_000u32 {
            registry.ack(agent, &format!("key-{index}"));
        }
        assert_eq!(registry.pending_ack_count(), 1);
    }
}
