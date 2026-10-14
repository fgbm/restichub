//! The agent listener and the gRPC stream over mTLS (ADR-0026, FR-ID-04, FR-ID-07, FR-PR-08).
//!
//! The tests start the real listener with the installation CA, enroll an agent over HTTPS, open the
//! stream with the enrolled certificate and drive the protocol. They cover the registry check
//! (revoked, wrong tenant, expired), the TLS chain (another CA), the single active stream and the
//! delivery of the signing key set.

#![allow(clippy::expect_used)]

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use prost::Message as _;
use restichub_cp::agent_listener::{
    der_to_pem, serve_listener, server_cert_sans, tls_config_from_issued,
};
use restichub_cp::pki::{CaService, CaStore};
use restichub_cp::server::AppState;
use restichub_db::test_util::TestDatabase;
use restichub_db::{Database, PLATFORM_TENANT_ID, SigningKeyRole, TenantId, signing};
use restichub_emulator::enroll::{EnrollFacts, EnrollTls, EnrolledIdentity, enroll_tls};
use restichub_emulator::identity::TestPki;
use restichub_proto::agent::v1::{
    AgentHello, AgentMessage, ControlMessage, KeyUpdate, KeyUpdateAck, ServerHello,
    SigningKeyUpdate, agent_message, agent_service_client::AgentServiceClient, control_message,
};
use restichub_proto::signing::KeyRing;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tonic::Streaming;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};
use uuid::Uuid;

use common::{create_tenant, create_user, pki_state, seed_enrollment_token_value};

/// One running server with its trust material.
struct Listener {
    state: AppState,
    ca: Arc<CaStore>,
    addr: SocketAddr,
    root_der: Vec<u8>,
    shutdown: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Listener {
    async fn start(state: AppState, ca: Arc<CaStore>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("cannot bind the listener");
        let addr = listener.local_addr().expect("the listener address");
        let sans = server_cert_sans(None, addr);
        let issued = CaService::new(common::test_kek())
            .issue_server_certificate(&ca, &sans)
            .expect("cannot issue the server certificate");
        let root_der = ca.root().cert_der.clone();
        let tls = tls_config_from_issued(&issued, &root_der).expect("cannot build the TLS config");
        let (shutdown, rx) = oneshot::channel();
        let serve_state = state.clone();
        let task = tokio::spawn(async move {
            let _ = serve_listener(listener, serve_state, tls, async move {
                let _ = rx.await;
            })
            .await;
        });
        Self {
            state,
            ca,
            addr,
            root_der,
            shutdown: Some(shutdown),
            task,
        }
    }

    fn https_url(&self) -> String {
        format!("https://127.0.0.1:{}", self.addr.port())
    }

    /// Sends the enrollment request of one agent over HTTPS.
    async fn enroll(&self, token: &str) -> EnrolledIdentity {
        let tls = EnrollTls {
            ca_der: self.root_der.clone(),
            server_name: "localhost".to_owned(),
        };
        let facts = EnrollFacts {
            hostname: "stream-host".to_owned(),
            os: "linux".to_owned(),
            arch: "amd64".to_owned(),
            agent_version: "0.1.0".to_owned(),
            restic_version: "0.19.0".to_owned(),
        };
        enroll_tls(&self.https_url(), token, &facts, &tls)
            .await
            .expect("the agent must enroll over HTTPS")
    }

    /// Builds a tonic client with the enrolled identity.
    async fn channel(
        &self,
        identity: &EnrolledIdentity,
    ) -> AgentServiceClient<tonic::transport::Channel> {
        let tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(der_to_pem(
                "CERTIFICATE",
                &self.root_der,
            )))
            .identity(Identity::from_pem(
                client_chain_pem(identity),
                identity.key_pem.clone(),
            ))
            .domain_name("localhost");
        let endpoint = Endpoint::from_shared(self.https_url())
            .expect("the endpoint URL")
            .tls_config(tls)
            .expect("the client TLS config");
        let channel = endpoint.connect().await.expect("the TLS channel");
        AgentServiceClient::new(channel)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

/// The client certificate chain in PEM: the leaf and its issuers.
fn client_chain_pem(identity: &EnrolledIdentity) -> String {
    let mut pem = der_to_pem("CERTIFICATE", &identity.certificate_der);
    for der in &identity.ca_chain_der {
        pem.push_str(&der_to_pem("CERTIFICATE", der));
    }
    pem
}

async fn setup(db: &TestDatabase) -> (Listener, TenantId, String) {
    let (state, ca) = pki_state(db.database()).await;
    let tenant = create_tenant(db.database(), "stream-tenant").await;
    let creator = create_user(db.database(), tenant, "stream-admin", "phc", "admin").await;
    let (token, _) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &[],
        time::OffsetDateTime::now_utc() + time::Duration::hours(1),
    )
    .await;
    let listener = Listener::start(state, ca).await;
    (listener, tenant, token)
}

fn hello() -> AgentMessage {
    AgentMessage {
        payload: Some(agent_message::Payload::AgentHello(AgentHello {
            protocol_version_major: restichub_proto::PROTOCOL_VERSION_MAJOR,
            protocol_version_minor: restichub_proto::PROTOCOL_VERSION_MINOR,
            agent_version: "0.1.0".to_owned(),
            ..AgentHello::default()
        })),
    }
}

/// Opens the stream and returns the outbound channel and the inbound stream.
async fn open_stream(
    client: &mut AgentServiceClient<tonic::transport::Channel>,
) -> Result<(mpsc::Sender<AgentMessage>, Streaming<ControlMessage>), tonic::Status> {
    let (tx, rx) = mpsc::channel::<AgentMessage>(16);
    let response = client.connect(ReceiverStream::new(rx)).await?;
    tx.send(hello()).await.expect("cannot send the hello");
    Ok((tx, response.into_inner()))
}

/// Reads messages until both the `ServerHello` and the `SigningKeyUpdate` arrived.
async fn read_handshake(
    inbound: &mut Streaming<ControlMessage>,
) -> (ServerHello, SigningKeyUpdate) {
    let mut hello = None;
    let mut update = None;
    for _ in 0..8 {
        let message = next_message(inbound).await;
        match message.payload {
            Some(control_message::Payload::ServerHello(value)) => hello = Some(value),
            Some(control_message::Payload::SigningKeyUpdate(value)) => update = Some(value),
            _ => {}
        }
        if hello.is_some() && update.is_some() {
            break;
        }
    }
    (
        hello.expect("the ServerHello must arrive"),
        update.expect("the key update must arrive"),
    )
}

async fn next_message(inbound: &mut Streaming<ControlMessage>) -> ControlMessage {
    tokio::time::timeout(Duration::from_secs(10), inbound.message())
        .await
        .expect("the message must arrive")
        .expect("the stream must not fail")
        .expect("the stream must not end")
}

/// True when the inbound side closes within the timeout.
async fn stream_ends(inbound: &mut Streaming<ControlMessage>) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(10), inbound.message()).await,
        Ok(Ok(None)) | Ok(Err(_))
    )
}

/// Reads the active current job signing key from the platform tenant.
async fn current_key(db: &Database) -> (String, [u8; 32]) {
    let mut tx = db
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .expect("cannot begin the transaction");
    let rows = signing::list(&mut tx).await.expect("cannot list the keys");
    tx.commit().await.expect("cannot commit the read");
    let row = rows
        .into_iter()
        .find(|row| {
            row.role == SigningKeyRole::Current
                && row.retired_at.is_none()
                && row.retire_after.is_none()
        })
        .expect("the current key is present");
    let bytes: [u8; 32] = row.public_key.as_slice().try_into().expect("a 32-byte key");
    (row.id.to_string(), bytes)
}

/// AC-ID-07 / FR-ID-07: enroll over HTTPS, open the stream, verify the key set with the emulator key
/// ring and write `last_seen_at`.
#[tokio::test]
async fn ac_id_07_enroll_stream_key_update_and_last_seen() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let (listener, tenant, token) = setup(&db).await;
    let identity = listener.enroll(&token).await;
    assert_eq!(identity.tenant_id, tenant.to_string());

    let mut client = listener.channel(&identity).await;
    let (_tx, mut inbound) = open_stream(&mut client)
        .await
        .expect("the stream must open");
    let (_hello, update) = read_handshake(&mut inbound).await;

    // The emulator verifies the signed key update with the current key from the installation.
    let (current_id, current_public) = current_key(db.database()).await;
    let mut ring = KeyRing::new();
    ring.insert(
        current_id,
        ed25519_dalek::VerifyingKey::from_bytes(&current_public).expect("a valid key"),
    )
    .expect("the ring must accept the current key");
    ring.apply_key_update(&update)
        .expect("the emulator must verify the key update");
    let inner = KeyUpdate::decode(update.key_update.as_slice()).expect("the inner update decodes");
    assert_eq!(inner.signer_key_id, update.signer_key_id);

    // The stream wrote `last_seen_at` for the enrolled agent.
    let agent_id = Uuid::parse_str(&identity.agent_id).expect("the agent id");
    let seen = last_seen(db.database(), tenant, agent_id).await;
    assert!(seen.is_some(), "the stream must write last_seen_at");

    // The acknowledgement reaches the control plane without an error.
    _tx.send(AgentMessage {
        payload: Some(agent_message::Payload::KeyUpdateAck(KeyUpdateAck {
            key_id: update.signer_key_id.clone(),
        })),
    })
    .await
    .expect("cannot send the acknowledgement");
    tokio::time::sleep(Duration::from_millis(200)).await;

    db.cleanup().await;
}

/// FR-ID-04: a revoked agent is refused at the next stream open.
#[tokio::test]
async fn ac_id_04_a_revoked_agent_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let (listener, tenant, token) = setup(&db).await;
    let identity = listener.enroll(&token).await;
    let actor = create_user(db.database(), tenant, "stream-revoker", "phc", "admin").await;
    revoke(
        db.database(),
        tenant,
        Uuid::parse_str(&identity.agent_id).expect("the agent id"),
        actor,
    )
    .await;

    let mut client = listener.channel(&identity).await;
    let error = open_stream(&mut client)
        .await
        .expect_err("the revoked agent must be refused");
    assert_eq!(error.code(), tonic::Code::PermissionDenied, "{error}");

    db.cleanup().await;
}

/// FR-PR-08: a certificate whose tenant names another tenant is refused.
#[tokio::test]
async fn a_wrong_tenant_san_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let (listener, _tenant, token) = setup(&db).await;
    let identity = listener.enroll(&token).await;
    let agent = Uuid::parse_str(&identity.agent_id).expect("the agent id");

    // Issue a certificate for the same agent under another tenant, so the registry lookup misses.
    let other = create_tenant(db.database(), "stream-other").await;
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .expect("cannot generate the key");
    let csr = rcgen::CertificateParams::default()
        .serialize_request(&key)
        .expect("cannot build the CSR")
        .der()
        .to_vec();
    let issued = listener
        .ca
        .issue_agent_certificate(other.as_uuid(), agent, &csr)
        .expect("cannot issue the certificate");
    let foreign = EnrolledIdentity {
        agent_id: agent.to_string(),
        tenant_id: other.to_string(),
        certificate_der: issued.cert_der,
        ca_chain_der: issued.chain_der,
        key_pem: key.serialize_pem(),
        signing_key_update: Vec::new(),
        grpc_endpoint: None,
    };

    let mut client = listener.channel(&foreign).await;
    let error = open_stream(&mut client)
        .await
        .expect_err("the wrong tenant must be refused");
    assert_eq!(error.code(), tonic::Code::PermissionDenied, "{error}");

    db.cleanup().await;
}

/// ADR-0026: an expired registry entry refuses the stream.
#[tokio::test]
async fn an_expired_registry_entry_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let (listener, tenant, token) = setup(&db).await;
    let identity = listener.enroll(&token).await;
    let agent = Uuid::parse_str(&identity.agent_id).expect("the agent id");
    expire(&db, tenant, agent).await;

    let mut client = listener.channel(&identity).await;
    let error = open_stream(&mut client)
        .await
        .expect_err("the expired agent must be refused");
    assert_eq!(error.code(), tonic::Code::PermissionDenied, "{error}");

    db.cleanup().await;
}

/// ADR-0026: a certificate from another CA does not open a stream.
///
/// The TLS layer offers client authentication with the installation root as the acceptable issuer. A
/// certificate from another CA fails the handshake with a fatal `UnknownCA` alert; a client that
/// presents no certificate reaches the handler, which refuses the anonymous stream.
#[tokio::test]
async fn a_certificate_from_another_ca_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let (listener, _tenant, _token) = setup(&db).await;
    let foreign = TestPki::generate(1).expect("cannot build the test PKI");
    let agent = &foreign.agents()[0];
    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(der_to_pem(
            "CERTIFICATE",
            &listener.root_der,
        )))
        .identity(Identity::from_pem(
            agent.cert_pem.clone(),
            agent.key_pem.clone(),
        ))
        .domain_name("localhost");
    let endpoint = Endpoint::from_shared(listener.https_url())
        .expect("the endpoint URL")
        .tls_config(tls)
        .expect("the client TLS config");

    match endpoint.connect().await {
        // A TLS refusal at the connect is also acceptable.
        Err(_) => {}
        Ok(channel) => {
            let mut client = AgentServiceClient::new(channel);
            let result = open_stream(&mut client).await;
            assert!(result.is_err(), "the foreign CA must not open a stream");
        }
    }

    db.cleanup().await;
}

/// FR-ID-08: one agent keeps one stream; a new open closes the old.
#[tokio::test]
async fn a_second_stream_closes_the_first() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let (listener, _tenant, token) = setup(&db).await;
    let identity = listener.enroll(&token).await;

    let mut first = listener.channel(&identity).await;
    let (_tx_a, mut inbound_a) = open_stream(&mut first)
        .await
        .expect("the first stream must open");
    read_handshake(&mut inbound_a).await;

    let mut second = listener.channel(&identity).await;
    let (_tx_b, mut inbound_b) = open_stream(&mut second)
        .await
        .expect("the second stream must open");
    read_handshake(&mut inbound_b).await;

    assert!(
        stream_ends(&mut inbound_a).await,
        "the first stream must close when the second opens"
    );

    db.cleanup().await;
}

/// FR-PR-11 / ADR-0026: a rotation delivers a fresh key set that the emulator verifies.
#[tokio::test]
async fn a_rotation_delivers_a_fresh_key_set() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let (listener, _tenant, token) = setup(&db).await;
    let identity = listener.enroll(&token).await;

    let mut client = listener.channel(&identity).await;
    let (_tx, mut inbound) = open_stream(&mut client)
        .await
        .expect("the stream must open");
    let (_hello, update) = read_handshake(&mut inbound).await;

    let (current_id, current_public) = current_key(db.database()).await;
    let mut ring = KeyRing::new();
    ring.insert(
        current_id,
        ed25519_dalek::VerifyingKey::from_bytes(&current_public).expect("a valid key"),
    )
    .expect("the ring must accept the current key");
    ring.apply_key_update(&update)
        .expect("the emulator must verify the initial key update");

    // Rotate the key set. The stream delivers the new set, signed by the promoted key.
    let signing = listener
        .state
        .job_signing()
        .expect("the job signing service is loaded")
        .clone();
    signing
        .rotate(db.database(), time::OffsetDateTime::now_utc())
        .await
        .expect("the rotation must succeed");

    let rotated = read_key_update(&mut inbound).await;
    ring.apply_key_update(&rotated)
        .expect("the emulator must verify the rotated key set");

    db.cleanup().await;
}

/// Reads the next `SigningKeyUpdate` message.
async fn read_key_update(inbound: &mut Streaming<ControlMessage>) -> SigningKeyUpdate {
    for _ in 0..16 {
        let message = next_message(inbound).await;
        if let Some(control_message::Payload::SigningKeyUpdate(update)) = message.payload {
            return update;
        }
    }
    panic!("the key update must arrive");
}

/// Reads `last_seen_at` of an agent through the owner connection.
async fn last_seen(db: &Database, tenant: TenantId, agent: Uuid) -> Option<time::OffsetDateTime> {
    // The public API of the repository returns the row through a tenant transaction.
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let row = restichub_db::agents::get(&mut tx, restichub_db::AgentId::from_uuid(agent))
        .await
        .expect("cannot read the agent");
    tx.commit().await.expect("cannot commit the read");
    row.and_then(|agent| agent.last_seen_at)
}

/// Revokes an agent through the repository.
async fn revoke(db: &Database, tenant: TenantId, agent: Uuid, actor: restichub_db::UserId) {
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    restichub_db::agents::revoke(
        &mut tx,
        restichub_db::AgentId::from_uuid(agent),
        actor,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .expect("cannot revoke the agent");
    tx.commit().await.expect("cannot commit the revoke");
}

/// Sets the certificate expiry of an agent to the past through the owner connection.
async fn expire(db: &TestDatabase, tenant: TenantId, agent: Uuid) {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query(
        "UPDATE agents SET certificate_expires_at = now() - interval '1 day' WHERE id = $1",
    )
    .bind(agent)
    .execute(&mut *tx)
    .await
    .expect("cannot expire the agent");
    tx.commit().await.expect("cannot commit the expiry");
}
