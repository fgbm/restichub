//! Server-Sent Events for the web UI (FR-UI-02, ADR-0009).
//!
//! The control plane publishes a small event for a change of a job, a log, a host or an alert. The
//! endpoint `GET /api/v1/events` sends the events of the tenant of the session to the browser. An
//! event holds identifiers and a status only, never a secret, a log line or a path. The UI reads the
//! data again from the REST API after an event.
//!
//! The bus keeps no history. A client that reconnects, with or without `Last-Event-ID`, receives a
//! `resync` event first and reads the state again. The same event follows a lag of the receiver.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use restichub_db::audit::Actor;
use restichub_db::{AgentId, JobId, JobStatus, Role, TenantId};
use serde::Serialize;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use crate::context::{Credential, Operation, RequestContext, minimum_role};
use crate::error::ApiError;
use crate::server::AppState;

const CHANNEL_CAPACITY: usize = 1024;
const OUTBOX_CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy)]
pub struct EventLimits {
    pub per_session: usize,
    pub per_token: usize,
    pub per_tenant: usize,
    pub per_ip: usize,
    pub global: usize,
    /// A stream ends after this time, so the client signs in again with its cookie.
    pub max_lifetime: Duration,
    pub heartbeat: Duration,
    /// How often an open stream checks its credential, the role and the tenant again. A stream ends
    /// when the access is gone.
    pub recheck: Duration,
    /// A client that does not read for this time loses its stream.
    pub send_timeout: Duration,
}

impl Default for EventLimits {
    fn default() -> Self {
        Self {
            per_session: 4,
            per_token: 4,
            per_tenant: 200,
            per_ip: 100,
            global: 2000,
            max_lifetime: Duration::from_secs(15 * 60),
            heartbeat: Duration::from_secs(15),
            recheck: Duration::from_secs(30),
            send_timeout: Duration::from_secs(10),
        }
    }
}

/// What changed. The kind decides the operation that a reader needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EventKind {
    #[serde(rename = "job.progress")]
    JobProgress,
    #[serde(rename = "job.status")]
    JobStatus,
    #[serde(rename = "job.log")]
    JobLog,
    #[serde(rename = "host.online")]
    HostOnline,
    #[serde(rename = "host.offline")]
    HostOffline,
    #[serde(rename = "alert.changed")]
    AlertChanged,
}

impl EventKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::JobProgress => "job.progress",
            Self::JobStatus => "job.status",
            Self::JobLog => "job.log",
            Self::HostOnline => "host.online",
            Self::HostOffline => "host.offline",
            Self::AlertChanged => "alert.changed",
        }
    }

    pub fn operation(self) -> Operation {
        match self {
            Self::JobProgress | Self::JobStatus | Self::JobLog => Operation::JobsRead,
            Self::HostOnline | Self::HostOffline => Operation::AgentsRead,
            Self::AlertChanged => Operation::AlertsRead,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct Payload {
    #[serde(skip_serializing_if = "Option::is_none")]
    job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'static str>,
}

#[derive(Debug, Clone)]
struct Published {
    id: u64,
    kind: EventKind,
    payload: Payload,
}

/// The in-process event bus (single control plane process, ADR-0002).
///
/// Each tenant has its own channel, so a burst of one tenant cannot make the streams of another
/// tenant lag (ADR-0006).
#[derive(Debug)]
pub struct EventBus {
    next_id: AtomicU64,
    limits: EventLimits,
    shutdown: watch::Sender<bool>,
    open: Mutex<Open>,
}

#[derive(Debug, Default)]
struct Open {
    total: usize,
    by_tenant: HashMap<TenantId, usize>,
    by_holder: HashMap<Uuid, usize>,
    by_ip: HashMap<IpAddr, usize>,
    channels: HashMap<TenantId, broadcast::Sender<Arc<Published>>>,
}

/// Who opens a stream, for the limits.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Opener {
    pub tenant: TenantId,
    /// The session or the API token, with its limit.
    pub holder: Option<(Uuid, usize)>,
    pub ip: Option<IpAddr>,
}

impl EventBus {
    pub fn new(limits: EventLimits) -> Self {
        Self {
            next_id: AtomicU64::new(1),
            limits,
            shutdown: watch::channel(false).0,
            open: Mutex::new(Open::default()),
        }
    }

    /// Ends every open stream and refuses new ones, for the shutdown of the server.
    pub fn close(&self) {
        self.shutdown.send_replace(true);
    }

    fn publish(&self, tenant: TenantId, kind: EventKind, payload: Payload) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let open = self.open.lock().unwrap_or_else(|error| error.into_inner());
        // A tenant with no open stream has no channel: nobody listens.
        if let Some(sender) = open.channels.get(&tenant) {
            let _ = sender.send(Arc::new(Published { id, kind, payload }));
        }
    }

    pub fn job_progress(&self, tenant: TenantId, job: JobId) {
        self.publish(tenant, EventKind::JobProgress, job_payload(job, None));
    }

    pub fn job_log(&self, tenant: TenantId, job: JobId) {
        self.publish(tenant, EventKind::JobLog, job_payload(job, None));
    }

    pub fn job_status(&self, tenant: TenantId, job: JobId, status: JobStatus) {
        self.publish(
            tenant,
            EventKind::JobStatus,
            job_payload(job, Some(status.as_str())),
        );
    }

    pub fn host(&self, tenant: TenantId, agent: AgentId, online: bool) {
        let kind = if online {
            EventKind::HostOnline
        } else {
            EventKind::HostOffline
        };
        self.publish(
            tenant,
            kind,
            Payload {
                job_id: None,
                agent_id: Some(agent.to_string()),
                status: None,
            },
        );
    }

    pub fn alert_changed(&self, tenant: TenantId) {
        self.publish(
            tenant,
            EventKind::AlertChanged,
            Payload {
                job_id: None,
                agent_id: None,
                status: None,
            },
        );
    }

    fn admit(
        self: &Arc<Self>,
        opener: Opener,
    ) -> Option<(Slot, broadcast::Receiver<Arc<Published>>)> {
        if *self.shutdown.borrow() {
            return None;
        }
        let limits = &self.limits;
        let mut open = self.open.lock().unwrap_or_else(|error| error.into_inner());
        if open.total >= limits.global
            || at_limit(&open.by_tenant, &opener.tenant, limits.per_tenant)
            || opener
                .holder
                .is_some_and(|(holder, limit)| at_limit(&open.by_holder, &holder, limit))
            || opener
                .ip
                .is_some_and(|ip| at_limit(&open.by_ip, &ip, limits.per_ip))
        {
            return None;
        }
        open.total += 1;
        *open.by_tenant.entry(opener.tenant).or_default() += 1;
        if let Some((holder, _)) = opener.holder {
            *open.by_holder.entry(holder).or_default() += 1;
        }
        if let Some(ip) = opener.ip {
            *open.by_ip.entry(ip).or_default() += 1;
        }
        let receiver = open
            .channels
            .entry(opener.tenant)
            .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0)
            .subscribe();
        Some((
            Slot {
                bus: Arc::clone(self),
                opener,
            },
            receiver,
        ))
    }

    pub fn open_streams(&self, tenant: TenantId) -> usize {
        let open = self.open.lock().unwrap_or_else(|error| error.into_inner());
        open.by_tenant.get(&tenant).copied().unwrap_or(0)
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(EventLimits::default())
    }
}

fn job_payload(job: JobId, status: Option<&'static str>) -> Payload {
    Payload {
        job_id: Some(job.to_string()),
        agent_id: None,
        status,
    }
}

/// One admitted stream. The drop gives the slot back.
struct Slot {
    bus: Arc<EventBus>,
    opener: Opener,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut open = self
            .bus
            .open
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        open.total -= 1;
        release(&mut open.by_tenant, self.opener.tenant);
        if let Some((holder, _)) = self.opener.holder {
            release(&mut open.by_holder, holder);
        }
        if let Some(ip) = self.opener.ip {
            release(&mut open.by_ip, ip);
        }
        if !open.by_tenant.contains_key(&self.opener.tenant) {
            open.channels.remove(&self.opener.tenant);
        }
    }
}

fn at_limit<K: std::hash::Hash + Eq>(map: &HashMap<K, usize>, key: &K, limit: usize) -> bool {
    map.get(key).copied().unwrap_or(0) >= limit
}

fn release<K: std::hash::Hash + Eq>(map: &mut HashMap<K, usize>, key: K) {
    if let Some(count) = map.get_mut(&key) {
        *count -= 1;
        if *count == 0 {
            map.remove(&key);
        }
    }
}

fn sse_event(published: &Published) -> SseEvent {
    SseEvent::default()
        .id(published.id.to_string())
        .event(published.kind.name())
        .json_data(&published.payload)
        .unwrap_or_else(|_| SseEvent::default().event(published.kind.name()))
}

fn resync_event() -> SseEvent {
    SseEvent::default().event("resync").data("{}")
}

/// Streams the events of the tenant of the session.
///
/// The stream sends a `resync` event first, then one event for each change that the role of the
/// reader may see. A comment line keeps the connection alive. The stream ends after a fixed
/// lifetime; the browser reconnects with its cookie.
#[utoipa::path(
    get,
    path = "/api/v1/events",
    tags = ["system"],
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (
            status = 200,
            description = "A `text/event-stream` body. The events are `resync`, `job.progress`, `job.status`, `job.log`, `host.online`, `host.offline` and `alert.changed`. The data of an event holds identifiers and a status only; the client reads the REST API for the rest. The stream ends after at most 15 minutes, and earlier when the session, the token, the account, the role or the tenant no longer gives the access. The client reconnects and reads the state again after the first `resync` event.",
            content_type = "text/event-stream",
            body = String
        ),
        (status = 429, description = "The session, the token, the address, the tenant or the server has too many open streams.", body = crate::error::ProblemBody, content_type = "application/problem+json"),
        crate::error::ProtectedResponses,
    )
)]
pub async fn events(
    State(state): State<AppState>,
    context: RequestContext,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    context.authorize(Operation::EventsRead)?;
    let bus = state.events_shared();
    let limits = bus.limits;
    let opener = Opener {
        tenant: context.tenant_id,
        holder: match (context.session_id, &context.actor) {
            (Some(session), _) => Some((session.as_uuid(), limits.per_session)),
            (None, Actor::ApiToken(token)) => Some((token.as_uuid(), limits.per_token)),
            (None, _) => None,
        },
        ip: context.source_ip,
    };
    let (slot, mut receiver) = bus
        .admit(opener)
        .ok_or_else(|| ApiError::rate_limited(limits.heartbeat.as_secs().max(1)))?;
    let credential = Credential::of(&state, &headers);
    let lifetime = lifetime_of(&state, &context, limits.max_lifetime);
    let (outbox, inbox) = mpsc::channel::<Result<SseEvent, Infallible>>(OUTBOX_CAPACITY);
    let mut shutdown = bus.shutdown.subscribe();
    let role = context.role;
    tokio::spawn(async move {
        let _slot = slot;
        let deadline = tokio::time::sleep(lifetime);
        tokio::pin!(deadline);
        let mut recheck =
            tokio::time::interval_at(tokio::time::Instant::now() + limits.recheck, limits.recheck);
        recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut next = Some(resync_event());
        loop {
            if let Some(event) = next.take() {
                // The send waits for a client that does not read, so the deadline, the shutdown and
                // a send timeout bound it.
                let sent = tokio::select! {
                    () = &mut deadline => return,
                    () = closed(&mut shutdown) => return,
                    sent = tokio::time::timeout(limits.send_timeout, outbox.send(Ok(event))) => sent,
                };
                if !matches!(sent, Ok(Ok(()))) {
                    return;
                }
            }
            next = tokio::select! {
                () = &mut deadline => return,
                () = closed(&mut shutdown) => return,
                () = outbox.closed() => return,
                _ = recheck.tick() => {
                    if !access_remains(&state, credential.as_ref(), &context, role).await {
                        return;
                    }
                    None
                }
                received = receiver.recv() => match received {
                    Ok(published) => {
                        if may_read(role, published.kind) {
                            Some(sse_event(&published))
                        } else {
                            None
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => Some(resync_event()),
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            };
        }
    });
    let mut response = Sse::new(ReceiverStream::new(inbox))
        .keep_alive(KeepAlive::new().interval(limits.heartbeat))
        .into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    // Tells nginx not to buffer the stream.
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    Ok(response)
}

async fn closed(shutdown: &mut watch::Receiver<bool>) {
    // An error means that the bus is gone, which also ends the stream.
    let _ = shutdown.wait_for(|closed| *closed).await;
}

/// False when the credential, the account, the role or the tenant no longer gives the access that
/// the stream was opened with. A failure of the database keeps the stream.
async fn access_remains(
    state: &AppState,
    credential: Option<&Credential>,
    opened: &RequestContext,
    role: Role,
) -> bool {
    let Some(credential) = credential else {
        return true;
    };
    match credential.recheck(state).await {
        Ok(now) => now.tenant_id == opened.tenant_id && now.role == role,
        Err(error) => !error.is_unauthenticated(),
    }
}

fn lifetime_of(state: &AppState, context: &RequestContext, maximum: Duration) -> Duration {
    let Some(expires) = context.session_expires_at else {
        return maximum;
    };
    let left = expires - state.auth().clock.now();
    Duration::try_from(left)
        .unwrap_or(Duration::ZERO)
        .min(maximum)
}

fn may_read(role: Role, kind: EventKind) -> bool {
    role.allows(minimum_role(kind.operation()))
}
