//! The restart of the agent listener (NFR-OPS-02).

#![allow(clippy::expect_used)]

mod common;

use std::time::Duration;

use restichub_cp::agent_listener::{
    ListenerLimits, serve_listener, server_cert_sans, tls_config_from_issued,
};
use restichub_cp::pki::CaService;

use common::pki_state;

#[tokio::test]
async fn nfr_ops_02_a_cancelled_listener_releases_its_address() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let (state, ca) = pki_state(db.database()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("cannot bind the listener");
    let addr = listener.local_addr().expect("the listener address");
    let issued = CaService::new(common::test_kek())
        .issue_server_certificate(&ca, &server_cert_sans(None, addr))
        .expect("cannot issue the server certificate");
    let tls =
        tls_config_from_issued(&issued, &ca.root().cert_der).expect("cannot build the TLS config");
    let serving = tokio::spawn(serve_listener(
        listener,
        state,
        tls,
        ListenerLimits::default(),
        std::future::pending(),
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;

    // A panic in the server future unwinds the same way: the future drops without a normal return.
    serving.abort();

    let mut bound = None;
    for _ in 0..50 {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                bound = Some(listener);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    assert!(bound.is_some(), "the address stays bound after the abort");
    db.cleanup().await;
}
