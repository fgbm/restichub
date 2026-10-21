//! The refusal counters of the agent listener (FR-MN-05).

#![allow(clippy::expect_used)]

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use restichub_cp::agent_listener::{
    ListenerLimits, serve_listener, server_cert_sans, tls_config_from_issued,
};
use restichub_cp::listener_stats::RefusalReason;
use restichub_cp::pki::CaService;
use restichub_cp::server::AppState;
use restichub_db::test_util::TestDatabase;
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;
use tokio_rustls::client::TlsStream;

use common::pki_state;

struct Listener {
    addr: SocketAddr,
    root_der: Vec<u8>,
    _shutdown: oneshot::Sender<()>,
}

async fn start(limits: ListenerLimits) -> Option<(AppState, Listener, TestDatabase)> {
    let db = common::test_db().await?;
    let (state, ca) = pki_state(db.database()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("cannot bind the listener");
    let addr = listener.local_addr().expect("the listener address");
    let issued = CaService::new(common::test_kek())
        .issue_server_certificate(&ca, &server_cert_sans(None, addr))
        .expect("cannot issue the server certificate");
    let root_der = ca.root().cert_der.clone();
    let tls = tls_config_from_issued(&issued, &root_der).expect("cannot build the TLS config");
    let (shutdown, rx) = oneshot::channel();
    let serve_state = state.clone();
    tokio::spawn(async move {
        let _ = serve_listener(listener, serve_state, tls, limits, async move {
            let _ = rx.await;
        })
        .await;
    });
    Some((
        state,
        Listener {
            addr,
            root_der,
            _shutdown: shutdown,
        },
        db,
    ))
}

/// Connects with the installation root CA and no client certificate.
async fn anonymous_tls(listener: &Listener) -> TlsStream<tokio::net::TcpStream> {
    use tokio_rustls::TlsConnector;
    use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};
    use tokio_rustls::rustls::{ClientConfig, RootCertStore};

    let mut roots = RootCertStore::empty();
    roots.add_parsable_certificates([CertificateDer::from(listener.root_der.clone())]);
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&tokio_rustls::rustls::version::TLS13])
        .expect("the protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    let stream = tokio::net::TcpStream::connect(listener.addr)
        .await
        .expect("cannot connect");
    TlsConnector::from(Arc::new(config))
        .connect(
            ServerName::try_from("localhost").expect("the server name"),
            stream,
        )
        .await
        .expect("the TLS handshake")
}

async fn assert_closed(stream: &mut TlsStream<tokio::net::TcpStream>) {
    let mut buffer = [0_u8; 1];
    let result = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
        .await
        .expect("the refused connection must close");
    assert!(matches!(result, Ok(0) | Err(_)), "the connection is open");
}

#[tokio::test]
async fn fr_mn_05_a_connection_above_the_cap_counts_as_a_cap_refusal() {
    let Some((state, listener, db)) = start(ListenerLimits {
        max_connections: 1,
        anonymous_per_ip: 10,
        anonymous_idle_timeout: Duration::from_secs(30),
        anonymous_lifetime: Duration::from_secs(30),
        ..ListenerLimits::default()
    })
    .await
    else {
        return;
    };

    let _held = anonymous_tls(&listener).await;
    let mut refused = anonymous_tls(&listener).await;
    assert_closed(&mut refused).await;

    let stats = state.listener_stats();
    assert_eq!(stats.refused(RefusalReason::Cap), 1);
    assert_eq!(stats.refused(RefusalReason::AnonymousShare), 0);
    assert_eq!(stats.accept_errors(), 0);
    db.cleanup().await;
}

#[tokio::test]
async fn fr_mn_05_a_connection_above_the_anonymous_share_counts_with_its_reason() {
    // The anonymous share is one tenth of the cap, so this cap leaves one anonymous slot.
    let Some((state, listener, db)) = start(ListenerLimits {
        max_connections: 10,
        anonymous_per_ip: 10,
        anonymous_idle_timeout: Duration::from_secs(30),
        anonymous_lifetime: Duration::from_secs(30),
        ..ListenerLimits::default()
    })
    .await
    else {
        return;
    };

    let _held = anonymous_tls(&listener).await;
    let mut refused = anonymous_tls(&listener).await;
    assert_closed(&mut refused).await;

    let stats = state.listener_stats();
    assert_eq!(stats.refused(RefusalReason::AnonymousShare), 1);
    assert_eq!(stats.refused(RefusalReason::Cap), 0);
    db.cleanup().await;
}

#[tokio::test]
async fn fr_mn_05_a_connection_above_the_anonymous_limit_of_an_address_counts_with_its_reason() {
    let Some((state, listener, db)) = start(ListenerLimits {
        max_connections: 100,
        anonymous_per_ip: 1,
        anonymous_idle_timeout: Duration::from_secs(30),
        anonymous_lifetime: Duration::from_secs(30),
        ..ListenerLimits::default()
    })
    .await
    else {
        return;
    };

    let _held = anonymous_tls(&listener).await;
    let mut refused = anonymous_tls(&listener).await;
    assert_closed(&mut refused).await;

    let stats = state.listener_stats();
    assert_eq!(stats.refused(RefusalReason::AnonymousPerIp), 1);
    assert_eq!(stats.refused(RefusalReason::AnonymousShare), 0);
    assert_eq!(stats.refused(RefusalReason::Cap), 0);
    db.cleanup().await;
}

/// Opens a TCP connection that never starts the TLS handshake, so it holds a handshake slot.
async fn silent_tcp(listener: &Listener) -> tokio::net::TcpStream {
    tokio::net::TcpStream::connect(listener.addr)
        .await
        .expect("cannot connect")
}

async fn assert_tcp_closed(stream: &mut tokio::net::TcpStream) {
    let mut buffer = [0_u8; 1];
    let result = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
        .await
        .expect("the refused connection must close");
    assert!(matches!(result, Ok(0) | Err(_)), "the connection is open");
}

#[tokio::test]
async fn fr_mn_05_a_handshake_above_the_limit_of_an_address_counts_with_its_reason() {
    let Some((state, listener, db)) = start(ListenerLimits {
        handshakes_per_ip: 1,
        handshake_timeout: Duration::from_secs(30),
        ..ListenerLimits::default()
    })
    .await
    else {
        return;
    };

    let _held = silent_tcp(&listener).await;
    let mut refused = silent_tcp(&listener).await;
    assert_tcp_closed(&mut refused).await;

    let stats = state.listener_stats();
    assert_eq!(stats.refused(RefusalReason::HandshakesPerIp), 1);
    assert_eq!(stats.refused(RefusalReason::Handshakes), 0);
    db.cleanup().await;
}

#[tokio::test]
async fn fr_mn_05_a_handshake_above_the_global_limit_counts_with_its_reason() {
    // The global limit is 256 handshakes; the per-address limit must not stop the loopback first.
    const HANDSHAKES: usize = 256;
    let Some((state, listener, db)) = start(ListenerLimits {
        handshakes_per_ip: HANDSHAKES * 2,
        handshake_timeout: Duration::from_secs(30),
        ..ListenerLimits::default()
    })
    .await
    else {
        return;
    };

    let mut held = Vec::with_capacity(HANDSHAKES);
    for _ in 0..HANDSHAKES {
        held.push(silent_tcp(&listener).await);
    }
    let mut refused = silent_tcp(&listener).await;
    assert_tcp_closed(&mut refused).await;

    let stats = state.listener_stats();
    assert_eq!(stats.refused(RefusalReason::Handshakes), 1);
    assert_eq!(stats.refused(RefusalReason::HandshakesPerIp), 0);
    db.cleanup().await;
}
