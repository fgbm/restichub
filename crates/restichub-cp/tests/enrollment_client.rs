//! The enrollment exchange over a real socket, driven by the emulator client (ADR-0026, FR-ID-02).
//!
//! The control plane serves the router on a loopback port and the emulator client of
//! `restichub-emulator` enrolls one agent end to end. The test proves the request and the response
//! of `POST /agent/v1/enroll` between the two crates.

#![allow(clippy::expect_used)]

mod common;

use std::net::SocketAddr;

use prost::Message as _;
use restichub_emulator::enroll::{EnrollFacts, enroll};
use restichub_proto::agent::v1::SigningKeyUpdate;
use serde_json::Value;
use time::{Duration, OffsetDateTime};
use x509_parser::prelude::{FromDer, GeneralName, X509Certificate};

use common::{
    AGENT_ENDPOINT, app_with_pki, create_tenant, create_user, request, seed_enrollment_token_value,
    seed_session, send,
};

#[tokio::test]
async fn ac_id_01_the_emulator_enrolls_end_to_end_against_the_control_plane() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "emulator-enroll").await;
    let creator = create_user(db.database(), tenant, "enroll-admin", "phc", "admin").await;
    let (token, _) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &["host".to_owned()],
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await;

    // Serve the control plane router on a loopback port.
    let app = app_with_pki(db.database()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("cannot bind the test server");
    let addr = listener.local_addr().expect("the server address");
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            server_app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("the test server stopped");
    });
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    let facts = EnrollFacts {
        hostname: "emulator-host".to_owned(),
        os: "linux".to_owned(),
        arch: "amd64".to_owned(),
        agent_version: "0.1.0".to_owned(),
        restic_version: "0.19.0".to_owned(),
    };
    let identity = enroll(&base_url, &token, &facts)
        .await
        .expect("the emulator must enroll");

    assert_eq!(identity.tenant_id, tenant.to_string());
    assert!(identity.key_pem.contains("PRIVATE KEY"));
    assert_eq!(identity.ca_chain_der.len(), 2);
    assert_eq!(identity.grpc_endpoint.as_deref(), Some(AGENT_ENDPOINT));
    let update = SigningKeyUpdate::decode(identity.signing_key_update.as_slice())
        .expect("the key update must decode");
    assert!(!update.signer_key_id.is_empty());

    // The certificate carries the identity of the returned agent.
    let cert = X509Certificate::from_der(&identity.certificate_der)
        .expect("cannot parse the certificate")
        .1;
    let expected_uri = format!(
        "restichub://tenant/{}/agent/{}",
        identity.tenant_id, identity.agent_id
    );
    let san = cert
        .subject_alternative_name()
        .expect("cannot read the SAN")
        .expect("the SAN is present");
    assert!(
        san.value
            .general_names
            .iter()
            .any(|name| matches!(name, GeneralName::URI(uri) if *uri == expected_uri.as_str())),
        "the SAN URI is missing: {:?}",
        san.value.general_names
    );

    // The agent appears in the tenant through the API.
    let reader = seed_session(db.database(), tenant, "reader", "viewer").await;
    let (status, _, page) = send(&app, request("GET", "/api/v1/agents", Some(&reader), None)).await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .expect("the agents list");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].get("id").and_then(Value::as_str),
        Some(identity.agent_id.as_str())
    );

    server.abort();
    db.cleanup().await;
}
