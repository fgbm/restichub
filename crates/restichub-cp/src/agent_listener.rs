//! The agent listener: the gRPC stream and the enrollment endpoint over TLS (ADR-0026).
//!
//! The listener has its own port. It serves the bidirectional `AgentService.Connect` stream and the
//! enrollment endpoint `POST /agent/v1/enroll` on one port (ADR-0026). The TLS layer requests a
//! client certificate but does not require one; the enrollment handler accepts a request without a
//! certificate and the `Connect` handler enforces mTLS. The listener speaks TLS 1.3 with rustls and
//! offers the ALPN protocols `h2` (the stream) and `http/1.1` (the enrollment).
//!
//! The control plane terminates TLS before the gRPC server, so the peer address and the client
//! certificates reach the handler through a request extension. The client chain is verified against
//! the installation root CA at the handshake; the handler then applies the per-agent registry check
//! (FR-PR-08).
//!
//! The listener bounds the unauthenticated load : a TLS handshake timeout, a semaphore for
//! the concurrent handshakes, a cap on the established connections, an idle timeout for a connection
//! without a client certificate and the HTTP/2 limits of the gRPC server.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use axum::extract::{ConnectInfo, DefaultBodyLimit};
use axum::middleware::{self, Next};
use axum::routing::post;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::server::WebPkiClientVerifier;
use tokio_rustls::rustls::{self, RootCertStore, ServerConfig};
use tokio_rustls::server::TlsStream;
use tonic::transport::server::Connected;

use crate::agent_stream::PeerInfo;
use crate::client_ip::IgnoreForwarded;
use crate::config::Config;
use crate::listener_stats::RefusalReason;
use crate::pki::IssuedServerCertificate;
use crate::server::{AppState, BODY_LIMIT};

/// The TLS handshake of one connection must finish within this time.
pub const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// A connection without a client certificate must send its request within this time.
pub const ANONYMOUS_IDLE_TIMEOUT: Duration = Duration::from_secs(10);

/// The total lifetime of a connection without a client certificate.
pub const ANONYMOUS_LIFETIME: Duration = Duration::from_secs(30);

/// The largest number of concurrent TLS handshakes.
const HANDSHAKE_CONCURRENCY: usize = 256;

/// The largest number of concurrent handshakes from one address.
const HANDSHAKES_PER_IP: usize = 16;

/// The largest number of concurrent connections without a client certificate from one address
/// ( ).
const ANONYMOUS_PER_IP: usize = 16;

/// Connections without a client certificate may use at most this fraction of the cap.
const ANONYMOUS_SHARE_DIVISOR: usize = 10;

/// The default cap on the established connections. It covers 2000 agents and a margin.
pub const DEFAULT_MAX_CONNECTIONS: usize = 2200;

/// Descriptors for the HTTP API requests and the files, beside the agent sockets, the event streams
/// and the database pool.
const OPEN_FILE_HEADROOM: usize = 256;

/// The open file limit that the connection cap, the event streams and the database pool need.
pub fn open_files_needed(
    max_connections: usize,
    event_streams: usize,
    database_pool: u32,
) -> usize {
    let pool = usize::try_from(database_pool).unwrap_or(usize::MAX);
    max_connections
        .saturating_add(HANDSHAKE_CONCURRENCY)
        .saturating_add(event_streams)
        .saturating_add(pool)
        .saturating_add(OPEN_FILE_HEADROOM)
}

/// The load bounds of the listener, checked at start.
#[derive(Debug, Clone, Copy)]
pub struct ListenerLimits {
    pub max_connections: usize,
    pub handshake_timeout: Duration,
    pub anonymous_idle_timeout: Duration,
    pub anonymous_lifetime: Duration,
    pub handshakes_per_ip: usize,
    pub anonymous_per_ip: usize,
}

impl ListenerLimits {
    /// The number of connections without a client certificate that the cap allows.
    fn anonymous_share(&self) -> usize {
        (self.max_connections / ANONYMOUS_SHARE_DIVISOR).max(1)
    }
}

impl Default for ListenerLimits {
    fn default() -> Self {
        Self {
            max_connections: DEFAULT_MAX_CONNECTIONS,
            handshake_timeout: TLS_HANDSHAKE_TIMEOUT,
            anonymous_idle_timeout: ANONYMOUS_IDLE_TIMEOUT,
            anonymous_lifetime: ANONYMOUS_LIFETIME,
            handshakes_per_ip: HANDSHAKES_PER_IP,
            anonymous_per_ip: ANONYMOUS_PER_IP,
        }
    }
}

/// Errors of the agent listener (ADR-0026).
#[derive(Debug, thiserror::Error)]
pub enum ListenerError {
    #[error("cannot bind the agent listener")]
    Bind(#[source] std::io::Error),
    #[error("the agent listener stopped with an error")]
    Serve(#[from] tonic::transport::Error),
    #[error("the agent listener ended without a shutdown signal")]
    Stopped,
    #[error("cannot build the TLS configuration of the agent listener")]
    Tls(#[from] rustls::Error),
    #[error("the client certificate verifier of the agent listener is not valid")]
    Verifier(#[from] rustls::server::VerifierBuilderError),
    #[error("the configured server certificate file `{path}` is not valid")]
    Certificate { path: String },
    #[error("the configured server key file `{path}` is not valid")]
    Key { path: String },
    #[error("cannot read the configured file `{path}`")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Builds the routes of the agent listener: the gRPC service and the enrollment route (ADR-0026).
///
/// The tonic `Routes` wraps an axum router, so the gRPC service and the enrollment endpoint share
/// one listener. The enrollment route keeps the 16 KiB body limit.
pub fn agent_routes(state: AppState) -> tonic::service::Routes {
    let mut routes = tonic::service::Routes::new(crate::agent_stream::agent_service(state.clone()));
    let enrollment = Router::new()
        .route(
            "/agent/v1/enroll",
            post(crate::enrollment::enroll).layer(DefaultBodyLimit::max(BODY_LIMIT)),
        )
        .with_state(state);
    let router = routes.axum_router_mut();
    *router = std::mem::take(router)
        .merge(enrollment)
        .layer(middleware::from_fn(inject_peer));
    routes
}

/// Builds the listener routes for a test that drives them without TLS.
pub fn agent_router(state: AppState) -> Router {
    agent_routes(state).into_axum_router()
}

/// Maps the connection information of the gRPC server to the extractors of the routes.
///
/// The enrollment handler reads the client address through [`crate::client_ip::ClientIp`]. The
/// listener has no proxy in front, so the middleware marks the request to ignore
/// `X-Forwarded-For`.
async fn inject_peer(mut request: axum::extract::Request, next: Next) -> axum::response::Response {
    request.extensions_mut().insert(IgnoreForwarded);
    if request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_none()
        && let Some(info) = request.extensions().get::<PeerInfo>()
        && let Some(addr) = info.remote
    {
        request.extensions_mut().insert(ConnectInfo(addr));
    }
    next.run(request).await
}

/// The subject alternative names of the server certificate (ADR-0026).
///
/// The host of the public endpoint comes first, so an agent verifies the name it dials. The listen
/// address covers a direct connection. An unspecified address (`0.0.0.0`, `::`) is not a name and is
/// left out.
pub fn server_cert_sans(endpoint: Option<&str>, listen: SocketAddr) -> Vec<String> {
    let mut sans = Vec::new();
    if let Some(host) = endpoint.and_then(endpoint_host) {
        sans.push(host);
    }
    let ip = listen.ip();
    if !ip.is_unspecified() {
        sans.push(ip.to_string());
        if ip.is_loopback() {
            sans.push("localhost".to_owned());
        }
    }
    sans.sort();
    sans.dedup();
    sans
}

/// The host name of a URL such as `https://agents.example:8443`.
fn endpoint_host(endpoint: &str) -> Option<String> {
    let rest = endpoint
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(endpoint);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(authority);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    (!host.is_empty()).then(|| host.to_owned())
}

/// Builds the rustls configuration of the listener.
///
/// The chain is the server certificate and its issuers; the root CA verifies the client
/// certificates. `allow_unauthenticated` requests a client certificate without requiring one, so the
/// enrollment handler accepts a request without a certificate (ADR-0026).
pub fn server_tls_config(
    chain_der: &[Vec<u8>],
    key_der: &[u8],
    root_ca_der: &[u8],
) -> Result<Arc<ServerConfig>, ListenerError> {
    let mut roots = RootCertStore::empty();
    roots.add_parsable_certificates([CertificateDer::from(root_ca_der.to_vec())]);
    let verifier = WebPkiClientVerifier::builder(roots.into())
        .allow_unauthenticated()
        .build()?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            chain_der
                .iter()
                .map(|der| CertificateDer::from(der.clone()))
                .collect(),
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der.to_vec())),
        )?;
    // The stream uses HTTP/2 and the enrollment uses HTTP/1.1 on the same port (ADR-0026).
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Builds the TLS configuration from the issued server certificate of the installation CA.
pub fn tls_config_from_issued(
    issued: &IssuedServerCertificate,
    root_ca_der: &[u8],
) -> Result<Arc<ServerConfig>, ListenerError> {
    let mut chain = Vec::with_capacity(issued.chain_der.len() + 1);
    chain.push(issued.cert_der.clone());
    chain.extend(issued.chain_der.iter().cloned());
    server_tls_config(&chain, &issued.key_der, root_ca_der)
}

/// Reads the configured server certificate and key files. `None` when the configuration sets no
/// file.
///
/// The certificate file may hold the leaf and its issuers. The key file holds one PKCS#8 key.
pub fn configured_tls_config(
    config: &Config,
    root_ca_der: &[u8],
) -> Result<Option<Arc<ServerConfig>>, ListenerError> {
    let (Some(cert_path), Some(key_path)) = (
        config.agent_tls_cert_file.as_deref(),
        config.agent_tls_key_file.as_deref(),
    ) else {
        return Ok(None);
    };
    let cert_pem = read(cert_path)?;
    let key_pem = read(key_path)?;
    let chain = pem_blocks(&cert_pem, "CERTIFICATE");
    if chain.is_empty() {
        return Err(ListenerError::Certificate {
            path: cert_path.display().to_string(),
        });
    }
    let key = pem_blocks(&key_pem, "PRIVATE KEY")
        .into_iter()
        .next()
        .ok_or_else(|| ListenerError::Key {
            path: key_path.display().to_string(),
        })?;
    Ok(Some(server_tls_config(&chain, &key, root_ca_der)?))
}

fn read(path: &Path) -> Result<String, ListenerError> {
    std::fs::read_to_string(path).map_err(|source| ListenerError::Read {
        path: path.display().to_string(),
        source,
    })
}

/// Decodes the DER of each PEM block with the given label.
fn pem_blocks(text: &str, label: &str) -> Vec<Vec<u8>> {
    use base64::Engine as _;
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(&begin) {
        let after = &rest[start + begin.len()..];
        let Some(stop) = after.find(&end) else {
            break;
        };
        let body: String = after[..stop]
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        if let Ok(der) = base64::engine::general_purpose::STANDARD.decode(body) {
            blocks.push(der);
        }
        rest = &after[stop + end.len()..];
    }
    blocks
}

/// Encodes a DER certificate as PEM.
pub fn der_to_pem(label: &str, der: &[u8]) -> String {
    use base64::Engine as _;
    use std::fmt::Write as _;
    let body = base64::engine::general_purpose::STANDARD.encode(der);
    let mut text = format!("-----BEGIN {label}-----\n");
    for chunk in body.as_bytes().chunks(64) {
        let _ = text.write_str(std::str::from_utf8(chunk).unwrap_or_default());
        text.push('\n');
    }
    let _ = writeln!(text, "-----END {label}-----");
    text
}

/// The live counters of the listener, used by the guards.
struct Load {
    connections: AtomicUsize,
    anonymous: AtomicUsize,
    anonymous_by_ip: Mutex<HashMap<IpAddr, usize>>,
    handshakes_by_ip: Mutex<HashMap<IpAddr, usize>>,
    max_connections: usize,
    anonymous_share: usize,
    anonymous_per_ip: usize,
    handshakes_per_ip: usize,
}

impl Load {
    fn new(limits: &ListenerLimits) -> Self {
        Self {
            connections: AtomicUsize::new(0),
            anonymous: AtomicUsize::new(0),
            anonymous_by_ip: Mutex::new(HashMap::new()),
            handshakes_by_ip: Mutex::new(HashMap::new()),
            max_connections: limits.max_connections,
            anonymous_share: limits.anonymous_share(),
            anonymous_per_ip: limits.anonymous_per_ip,
            handshakes_per_ip: limits.handshakes_per_ip,
        }
    }

    /// Reserves a handshake slot for an address. `None` when the address holds its per-IP limit.
    fn try_handshake(self: &Arc<Self>, ip: IpAddr) -> Option<HandshakeGuard> {
        let mut by_ip = self
            .handshakes_by_ip
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let count = by_ip.entry(ip).or_insert(0);
        if *count >= self.handshakes_per_ip {
            return None;
        }
        *count += 1;
        Some(HandshakeGuard {
            load: Arc::clone(self),
            ip,
        })
    }

    /// Reserves a connection slot, or names the cap or the anonymous limit that is reached.
    fn try_connect(
        self: &Arc<Self>,
        ip: IpAddr,
        authenticated: bool,
    ) -> Result<ConnectGuard, RefusalReason> {
        if self.connections.load(Ordering::Acquire) >= self.max_connections {
            return Err(RefusalReason::Cap);
        }
        if !authenticated {
            if self.anonymous.load(Ordering::Acquire) >= self.anonymous_share {
                return Err(RefusalReason::AnonymousShare);
            }
            let mut by_ip = self
                .anonymous_by_ip
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let count = by_ip.entry(ip).or_insert(0);
            if *count >= self.anonymous_per_ip {
                return Err(RefusalReason::AnonymousPerIp);
            }
            *count += 1;
            self.anonymous.fetch_add(1, Ordering::AcqRel);
        }
        self.connections.fetch_add(1, Ordering::AcqRel);
        Ok(ConnectGuard {
            load: Arc::clone(self),
            ip,
            anonymous: !authenticated,
        })
    }
}

/// One in-progress handshake slot of an address.
struct HandshakeGuard {
    load: Arc<Load>,
    ip: IpAddr,
}

impl Drop for HandshakeGuard {
    fn drop(&mut self) {
        let mut by_ip = self
            .load
            .handshakes_by_ip
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(count) = by_ip.get_mut(&self.ip) {
            *count -= 1;
            if *count == 0 {
                by_ip.remove(&self.ip);
            }
        }
    }
}

/// One established connection slot.
struct ConnectGuard {
    load: Arc<Load>,
    ip: IpAddr,
    anonymous: bool,
}

impl Drop for ConnectGuard {
    fn drop(&mut self) {
        self.load.connections.fetch_sub(1, Ordering::AcqRel);
        if self.anonymous {
            self.load.anonymous.fetch_sub(1, Ordering::AcqRel);
            let mut by_ip = self
                .load
                .anonymous_by_ip
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(count) = by_ip.get_mut(&self.ip) {
                *count -= 1;
                if *count == 0 {
                    by_ip.remove(&self.ip);
                }
            }
        }
    }
}

/// One accepted connection with its peer information and its load guards.
///
/// The type closes a connection without a client certificate at the idle timeout and at the total
/// lifetime cap, whatever the HTTP/2 keepalive does.
struct AgentConnection {
    inner: TlsStream<TcpStream>,
    peer: PeerInfo,
    _guard: ConnectGuard,
    /// The deadline for the first read of an anonymous connection; `None` once a byte arrives.
    first_byte: Option<Pin<Box<tokio::time::Sleep>>>,
    /// The total lifetime of an anonymous connection; always armed for one.
    lifetime: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl AgentConnection {
    /// Polls the deadlines. An elapsed deadline closes the connection.
    fn check_deadlines(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        if let Some(deadline) = self.first_byte.as_mut()
            && deadline.as_mut().poll(cx).is_ready()
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the connection sent no request",
            ));
        }
        if let Some(deadline) = self.lifetime.as_mut()
            && deadline.as_mut().poll(cx).is_ready()
        {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the connection reached its lifetime",
            ));
        }
        Ok(())
    }
}

impl AsyncRead for AgentConnection {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Err(error) = this.check_deadlines(cx) {
            return Poll::Ready(Err(error));
        }
        let before = buf.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() > before {
            this.first_byte = None;
        }
        result
    }
}

impl AsyncWrite for AgentConnection {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if let Err(error) = this.check_deadlines(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut this.inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

impl Connected for AgentConnection {
    type ConnectInfo = PeerInfo;

    fn connect_info(&self) -> PeerInfo {
        self.peer.clone()
    }
}

/// Applies TCP keepalive to an accepted socket, because the gRPC server ignores its own setting on
/// a pre-terminated stream.
fn set_keepalive(stream: &TcpStream) {
    let socket = socket2::SockRef::from(stream);
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(60))
        .with_interval(Duration::from_secs(20));
    let _ = socket.set_tcp_keepalive(&keepalive);
}

/// Serves the agent listener on a bound listener until the shutdown future resolves (ADR-0026).
///
/// The control plane terminates TLS on its own, so the gRPC server receives the already-decrypted
/// streams. This keeps the ALPN protocols of the enrollment and the stream on one port and passes
/// the client certificate to the handler.
pub async fn serve_listener(
    listener: TcpListener,
    state: AppState,
    tls: Arc<ServerConfig>,
    limits: ListenerLimits,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ListenerError> {
    let acceptor = TlsAcceptor::from(tls);
    let handshakes = Arc::new(Semaphore::new(HANDSHAKE_CONCURRENCY));
    let load = Arc::new(Load::new(&limits));
    let stats = Arc::clone(state.listener_stats());
    let report = tokio::spawn({
        let load = Arc::clone(&load);
        Arc::clone(&stats).report(
            move || load.connections.load(Ordering::Acquire),
            limits.max_connections,
        )
    });
    let (tx, rx) = tokio::sync::mpsc::channel::<io::Result<AgentConnection>>(64);
    let accept = tokio::spawn(async move {
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(error) => {
                    stats.record_accept_error(&error);
                    // A transient error such as EMFILE must not stop the listener, and the pause
                    // keeps the loop from spinning while the descriptors stay exhausted.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
            set_keepalive(&stream);
            let ip = peer.ip();
            // A handshake over the global or the per-IP limit drops the connection, so a flood
            // cannot exhaust memory and one address cannot hold every slot.
            let Ok(permit) = handshakes.clone().try_acquire_owned() else {
                stats.record_refusal(RefusalReason::Handshakes);
                drop(stream);
                continue;
            };
            let Some(handshake) = load.try_handshake(ip) else {
                stats.record_refusal(RefusalReason::HandshakesPerIp);
                drop(stream);
                continue;
            };
            let acceptor = acceptor.clone();
            let tx = tx.clone();
            let load = Arc::clone(&load);
            let stats = Arc::clone(&stats);
            tokio::spawn(async move {
                let _permit = permit;
                let _handshake = handshake;
                let tls =
                    match tokio::time::timeout(limits.handshake_timeout, acceptor.accept(stream))
                        .await
                    {
                        Ok(Ok(tls)) => tls,
                        // A failed or a slow handshake drops the connection.
                        _ => return,
                    };
                let certs = tls
                    .get_ref()
                    .1
                    .peer_certificates()
                    .map(|certs| Arc::new(certs.to_vec()));
                let authenticated = certs.is_some();
                // The cap and the anonymous share of it bound the established connections.
                let guard = match load.try_connect(ip, authenticated) {
                    Ok(guard) => guard,
                    Err(reason) => {
                        stats.record_refusal(reason);
                        return;
                    }
                };
                // A connection without a client certificate may only enroll, so it must send its
                // request promptly and it lives for a bounded time.
                let first_byte = (!authenticated)
                    .then(|| Box::pin(tokio::time::sleep(limits.anonymous_idle_timeout)));
                let lifetime = (!authenticated)
                    .then(|| Box::pin(tokio::time::sleep(limits.anonymous_lifetime)));
                let connection = AgentConnection {
                    inner: tls,
                    peer: PeerInfo {
                        remote: Some(peer),
                        certs,
                    },
                    _guard: guard,
                    first_byte,
                    lifetime,
                };
                let _ = tx.send(Ok(connection)).await;
            });
        }
    });

    let mut builder = tonic::transport::Server::builder();
    builder = builder
        .accept_http1(true)
        .http2_keepalive_interval(Some(Duration::from_secs(30)))
        .http2_keepalive_timeout(Some(Duration::from_secs(10)))
        .max_concurrent_streams(Some(16))
        .max_frame_size(Some(1 << 20))
        .http2_max_header_list_size(Some(64 * 1024))
        .concurrency_limit_per_connection(8)
        .load_shed(true);
    let router = builder.add_routes(agent_routes(state));
    let served = router
        .serve_with_incoming_shutdown(tokio_stream::wrappers::ReceiverStream::new(rx), shutdown)
        .await;
    report.abort();
    // The accept loop owns the socket, so a restart of the listener can bind the address again.
    accept.abort();
    served?;
    Ok(())
}

/// Binds the listen address and serves the agent listener until the shutdown future resolves.
pub async fn serve(
    state: AppState,
    addr: SocketAddr,
    tls: Arc<ServerConfig>,
    limits: ListenerLimits,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ListenerError> {
    let listener = TcpListener::bind(addr).await.map_err(ListenerError::Bind)?;
    serve_listener(listener, state, tls, limits, shutdown).await
}

#[cfg(test)]
mod tests {
    use super::{endpoint_host, server_cert_sans};
    use std::net::SocketAddr;

    #[test]
    fn the_endpoint_host_ignores_the_scheme_and_the_port() {
        assert_eq!(
            endpoint_host("https://agents.example:8443").as_deref(),
            Some("agents.example")
        );
        assert_eq!(
            endpoint_host("agents.example:8443").as_deref(),
            Some("agents.example")
        );
        assert_eq!(
            endpoint_host("https://[2001:db8::1]:8443").as_deref(),
            Some("2001:db8::1")
        );
        assert_eq!(endpoint_host("").as_deref(), None);
    }

    #[test]
    fn the_sans_hold_the_endpoint_host_and_the_listen_address() {
        let listen: SocketAddr = "127.0.0.1:8443".parse().expect("a socket address");
        let sans = server_cert_sans(Some("https://agents.example:8443"), listen);
        assert!(sans.contains(&"agents.example".to_owned()));
        assert!(sans.contains(&"127.0.0.1".to_owned()));
    }

    #[test]
    fn an_unspecified_listen_address_is_not_a_name() {
        let listen: SocketAddr = "0.0.0.0:8443".parse().expect("a socket address");
        let sans = server_cert_sans(None, listen);
        assert!(!sans.contains(&"0.0.0.0".to_owned()));
        assert!(sans.is_empty());
    }

    #[test]
    fn a_loopback_listener_adds_localhost() {
        let listen: SocketAddr = "127.0.0.1:8443".parse().expect("a socket address");
        let sans = server_cert_sans(None, listen);
        assert!(sans.contains(&"127.0.0.1".to_owned()));
        assert!(sans.contains(&"localhost".to_owned()));
    }
}
