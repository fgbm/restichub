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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderValue, header};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use restichub_db::{AgentId, JobId, JobStatus, Role, SessionId, TenantId};
use serde::Serialize;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::context::{Operation, RequestContext, minimum_role};
use crate::error::ApiError;
use crate::server::AppState;

const CHANNEL_CAPACITY: usize = 1024;
const OUTBOX_CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy)]
pub struct EventLimits {
    pub per_session: usize,
    pub per_tenant: usize,
    /// A stream ends after this time, so the client signs in again with its cookie and a revoked
    /// session cannot keep a stream open for long.
    pub max_lifetime: Duration,
    pub heartbeat: Duration,
}

impl Default for EventLimits {
    fn default() -> Self {
        Self {
            per_session: 4,
            per_tenant: 200,
            max_lifetime: Duration::from_secs(15 * 60),
            heartbeat: Duration::from_secs(15),
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
    tenant: TenantId,
    kind: EventKind,
    payload: Payload,
}

/// The in-process event bus (single control plane process, ADR-0002).
#[derive(Debug)]
pub struct EventBus {
    sender: broadcast::Sender<Arc<Published>>,
    next_id: AtomicU64,
    limits: EventLimits,
    open: Mutex<Open>,
}

#[derive(Debug, Default)]
struct Open {
    by_tenant: HashMap<TenantId, usize>,
    by_session: HashMap<SessionId, usize>,
}

impl EventBus {
    pub fn new(limits: EventLimits) -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            sender,
            next_id: AtomicU64::new(1),
            limits,
            open: Mutex::new(Open::default()),
        }
    }

    fn publish(&self, tenant: TenantId, kind: EventKind, payload: Payload) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        // No receiver is not an error: nobody listens.
        let _ = self.sender.send(Arc::new(Published {
            id,
            tenant,
            kind,
            payload,
        }));
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

    fn admit(self: &Arc<Self>, tenant: TenantId, session: Option<SessionId>) -> Option<Slot> {
        let mut open = self.open.lock().unwrap_or_else(|error| error.into_inner());
        if open.by_tenant.get(&tenant).copied().unwrap_or(0) >= self.limits.per_tenant {
            return None;
        }
        if let Some(session) = session
            && open.by_session.get(&session).copied().unwrap_or(0) >= self.limits.per_session
        {
            return None;
        }
        *open.by_tenant.entry(tenant).or_default() += 1;
        if let Some(session) = session {
            *open.by_session.entry(session).or_default() += 1;
        }
        Some(Slot {
            bus: Arc::clone(self),
            tenant,
            session,
        })
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
    tenant: TenantId,
    session: Option<SessionId>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut open = self
            .bus
            .open
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        release(&mut open.by_tenant, self.tenant);
        if let Some(session) = self.session {
            release(&mut open.by_session, session);
        }
    }
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
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (
            status = 200,
            description = "A `text/event-stream` body. The events are `resync`, `job.progress`, `job.status`, `job.log`, `host.online`, `host.offline` and `alert.changed`. The data of an event holds identifiers and a status only; the client reads the REST API for the rest. The stream ends after at most 15 minutes. The client reconnects and reads the state again after the first `resync` event.",
            content_type = "text/event-stream",
            body = String
        ),
        (status = 429, description = "The session or the tenant has too many open streams.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
    )
)]
pub async fn events(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Response, ApiError> {
    context.authorize(Operation::EventsRead)?;
    let bus = state.events_shared();
    let limits = bus.limits;
    let slot = bus
        .admit(context.tenant_id, context.session_id)
        .ok_or_else(|| ApiError::rate_limited(limits.heartbeat.as_secs().max(1)))?;
    let lifetime = lifetime_of(&state, &context, limits.max_lifetime);
    let (outbox, inbox) = mpsc::channel::<Result<SseEvent, Infallible>>(OUTBOX_CAPACITY);
    let mut receiver = bus.sender.subscribe();
    let tenant = context.tenant_id;
    let role = context.role;
    tokio::spawn(async move {
        let _slot = slot;
        let deadline = tokio::time::sleep(lifetime);
        tokio::pin!(deadline);
        if outbox.send(Ok(resync_event())).await.is_err() {
            return;
        }
        loop {
            let event = tokio::select! {
                () = &mut deadline => return,
                () = outbox.closed() => return,
                received = receiver.recv() => match received {
                    Ok(published) => {
                        if published.tenant != tenant || !may_read(role, published.kind) {
                            continue;
                        }
                        sse_event(&published)
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => resync_event(),
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            };
            if outbox.send(Ok(event)).await.is_err() {
                return;
            }
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
