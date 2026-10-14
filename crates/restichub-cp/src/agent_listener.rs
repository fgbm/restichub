//! The agent listener: the gRPC stream and the enrollment endpoint over TLS (ADR-0026).
//!
//! The listener has its own port. It serves the bidirectional `AgentService.Connect` stream and the
//! enrollment endpoint `POST /agent/v1/enroll` on one port (ADR-0026). The TLS layer requests a
//! client certificate but does not require one; the enrollment handler accepts a request without a
//! certificate and the `Connect` handler enforces mTLS. The listener speaks TLS 1.3 with rustls and
//! offers the ALPN protocols `h2` (the stream) and `http/1.1` (the enrollment).
//!
//! The control plane terminates TLS before the gRPC server, so the peer certificates reach the
//! handler through the connection info of the stream (`request.peer_certs()`). The client chain is
//! verified against the installation root CA at the handshake; the handler then applies the
//! per-agent registry check (FR-PR-08).

use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use axum::extract::ConnectInfo;
use axum::middleware::{self, Next};
use axum::routing::post;
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::server::WebPkiClientVerifier;
use tokio_rustls::rustls::{self, RootCertStore, ServerConfig};
use tokio_rustls::server::TlsStream;
use tonic::transport::server::{TcpConnectInfo, TlsConnectInfo};

use crate::config::Config;
use crate::pki::IssuedServerCertificate;
use crate::server::AppState;

/// Errors of the agent listener (ADR-0026).
#[derive(Debug, thiserror::Error)]
pub enum ListenerError {
    #[error("cannot bind the agent listener")]
    Bind(#[source] std::io::Error),
    #[error("the agent listener stopped with an error")]
    Serve(#[from] tonic::transport::Error),
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
/// one listener.
pub fn agent_routes(state: AppState) -> tonic::service::Routes {
    let mut routes = tonic::service::Routes::new(crate::agent_stream::agent_service(state.clone()));
    let enrollment = Router::new()
        .route("/agent/v1/enroll", post(crate::enrollment::enroll))
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

/// Maps the TLS connection info of the gRPC server to the `ConnectInfo` of the web extractors.
///
/// The enrollment handler reads the client address through [`crate::client_ip::ClientIp`], which
/// expects `ConnectInfo<SocketAddr>`. The gRPC server inserts the TLS connection info instead.
async fn inject_peer(mut request: axum::extract::Request, next: Next) -> axum::response::Response {
    if request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_none()
        && let Some(info) = request.extensions().get::<TlsConnectInfo<TcpConnectInfo>>()
        && let Some(addr) = info.get_ref().remote_addr()
    {
        request.extensions_mut().insert(ConnectInfo(addr));
    }
    next.run(request).await
}

/// The subject alternative names of the server certificate (ADR-0026).
///
/// The host of the public endpoint comes first, so an agent verifies the name it dials. The listen
/// address covers a direct connection.
pub fn server_cert_sans(endpoint: Option<&str>, listen: SocketAddr) -> Vec<String> {
    let mut sans = Vec::new();
    if let Some(host) = endpoint.and_then(endpoint_host) {
        sans.push(host);
    }
    sans.push(listen.ip().to_string());
    if listen.ip().is_loopback() {
        sans.push("localhost".to_owned());
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

/// Serves the agent listener on a bound listener until the shutdown future resolves (ADR-0026).
///
/// The control plane terminates TLS on its own, so the gRPC server receives the already-decrypted
/// streams. This keeps the ALPN protocols of the enrollment and the stream on one port and passes
/// the client certificate to the handler.
pub async fn serve_listener(
    listener: TcpListener,
    state: AppState,
    tls: Arc<ServerConfig>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ListenerError> {
    let acceptor = TlsAcceptor::from(tls);
    let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<TlsStream<TcpStream>>>(64);
    tokio::spawn(async move {
        loop {
            let (stream, _peer) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(_) => {
                    // A transient accept error must not stop the listener.
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    continue;
                }
            };
            let acceptor = acceptor.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                // A failed handshake (a bad certificate, a probing client) drops the connection.
                if let Ok(tls) = acceptor.accept(stream).await {
                    let _ = tx.send(Ok(tls)).await;
                }
            });
        }
    });

    let mut builder = tonic::transport::Server::builder();
    builder = builder.accept_http1(true);
    let router = builder.add_routes(agent_routes(state));
    router
        .serve_with_incoming_shutdown(tokio_stream::wrappers::ReceiverStream::new(rx), shutdown)
        .await?;
    Ok(())
}

/// Binds the listen address and serves the agent listener until the shutdown future resolves.
pub async fn serve(
    state: AppState,
    addr: SocketAddr,
    tls: Arc<ServerConfig>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ListenerError> {
    let listener = TcpListener::bind(addr).await.map_err(ListenerError::Bind)?;
    serve_listener(listener, state, tls, shutdown).await
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
        let listen: SocketAddr = "0.0.0.0:8443".parse().expect("a socket address");
        let sans = server_cert_sans(Some("https://agents.example:8443"), listen);
        assert!(sans.contains(&"agents.example".to_owned()));
        assert!(sans.contains(&"0.0.0.0".to_owned()));
    }

    #[test]
    fn a_loopback_listener_adds_localhost() {
        let listen: SocketAddr = "127.0.0.1:8443".parse().expect("a socket address");
        let sans = server_cert_sans(None, listen);
        assert!(sans.contains(&"127.0.0.1".to_owned()));
        assert!(sans.contains(&"localhost".to_owned()));
    }
}
