//! Shared helpers for the agent integration tests.
//!
//! The tests enroll against a real control plane router over TLS. The control plane is a
//! dev-dependency, so the agent keeps a small normal dependency tree.

#![allow(dead_code)]
#![allow(clippy::expect_used)]

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use hyper_util::rt::TokioIo;
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, PKCS_ECDSA_P256_SHA256,
};
use restichub_cp::agent_listener::{
    ListenerLimits, der_to_pem, serve_listener, server_cert_sans, tls_config_from_issued,
};
use restichub_cp::config::{AuthConfig, TenancyConfig};
use restichub_cp::pki::{CaService, CaStore};
use restichub_cp::server::{AppState, AuthState, router};
use restichub_db::test_util::{TestDatabase, test_database};
use restichub_db::{Database, Role, TenantId, enrollment_tokens, signing, tenants, users};
use restichub_proto::agent::v1::{
    AgentMessage, ControlMessage, Ping,
    agent_service_server::{AgentService, AgentServiceServer},
    control_message,
};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use time::{Duration, OffsetDateTime};
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

/// The gRPC endpoint that the enrollment tests report to an agent.
pub const AGENT_ENDPOINT: &str = "https://agents.example:8443";

/// A test database, a tenant, a one-time token and a TLS control plane.
pub struct Fixture {
    pub db: TestDatabase,
    pub tenant: TenantId,
    pub token: String,
    pub server: TestServer,
}

/// Makes a fresh tenant, a one-time token and a TLS control plane.
pub async fn fixture(name: &str) -> Option<Fixture> {
    let db = test_database().await?;
    let tenant = create_tenant(db.database(), name).await;
    let creator = create_user(db.database(), tenant, "agent-admin", "phc", "admin").await;
    let (token, _) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &["host".to_owned()],
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await;
    let app = app_with_pki(db.database()).await;
    let server = TestServer::start(app);
    Some(Fixture {
        db,
        tenant,
        token,
        server,
    })
}

/// Writes `contents` to a unique file under the temporary directory.
pub fn write_temp_file(name: &str, contents: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(unique_name(name));
    std::fs::write(&path, contents).expect("cannot write the test file");
    path
}

/// A directory under the temporary directory, removed before use.
pub fn temp_config_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(unique_name(name));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A process-unique path component, so parallel tests never share a file.
fn unique_name(name: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "restichub-agent-{name}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

#[derive(Debug)]
struct TestKek(restichub_secrets::Kek);

impl restichub_secrets::KekProvider for TestKek {
    fn current(&self) -> &restichub_secrets::Kek {
        &self.0
    }

    fn by_id(&self, id: &restichub_secrets::KekId) -> Option<&restichub_secrets::Kek> {
        (self.0.id() == id).then_some(&self.0)
    }
}

fn test_kek() -> Arc<dyn restichub_secrets::KekProvider> {
    let mut bytes = [7_u8; 32];
    Arc::new(TestKek(restichub_secrets::Kek::from_bytes(&mut bytes)))
}

async fn app_with_pki(db: &Database) -> Router {
    let provider = test_kek();
    let ca = restichub_cp::pki::CaService::new(provider.clone());
    let store = ca
        .ensure_initialized(db)
        .await
        .expect("cannot create the CA");
    let job_signing = restichub_cp::job_signing::JobSigningService::new(provider.clone());
    job_signing
        .ensure_initialized(db)
        .await
        .expect("cannot create the job signing key set");
    let auth = AuthState::new(AuthConfig::default());
    router(
        AppState::with_auth(db.clone(), auth)
            .with_kek(provider)
            .with_tenancy(TenancyConfig::multi())
            .with_pki(
                Arc::new(store),
                job_signing,
                Some(AGENT_ENDPOINT.to_owned()),
            ),
    )
}

async fn create_tenant(db: &Database, name: &str) -> TenantId {
    // The tenant needs a wrapped data encryption key, so a repository job can store a secret.
    let kek = test_kek();
    let id = TenantId::new();
    let dek = restichub_secrets::generate_dek().expect("cannot generate the DEK");
    let wrapped = restichub_secrets::wrap_dek(kek.current(), id.as_uuid(), 1, &dek)
        .expect("cannot wrap the DEK");
    let mut tx = db
        .begin_platform()
        .await
        .expect("cannot begin the transaction");
    let input = tenants::BootstrapTenant {
        id,
        name,
        key_version: 1,
        kek_id: wrapped.kek_id.as_str(),
        wrapped_dek: &wrapped.ciphertext,
        nonce: wrapped.nonce.as_slice(),
        actor: restichub_db::audit::Actor::System,
        source_ip: None,
        admin: None,
    };
    tenants::create_with_bootstrap(&mut tx, &input)
        .await
        .expect("cannot create the tenant with a key");
    tx.commit().await.expect("cannot commit the tenant");
    id
}

pub async fn create_user(
    db: &Database,
    tenant: TenantId,
    username: &str,
    password_hash: &str,
    role: &str,
) -> restichub_db::UserId {
    let role = Role::parse(role).expect("the role is known");
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let user = users::create_member(
        &mut tx,
        &users::NewUser {
            username,
            password_hash,
            display_name: username,
        },
        role,
    )
    .await
    .expect("cannot create the user");
    tx.commit().await.expect("cannot commit the user");
    user
}

async fn seed_enrollment_token_value(
    db: &Database,
    tenant: TenantId,
    created_by: restichub_db::UserId,
    labels: &[String],
    expires_at: OffsetDateTime,
) -> (String, restichub_db::EnrollmentTokenId) {
    let (value, prefix) =
        restichub_cp::token::random_enrollment_token().expect("cannot build the token");
    let hash = restichub_cp::token::hash_token(&value);
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let id = enrollment_tokens::create(
        &mut tx,
        &enrollment_tokens::NewEnrollmentToken {
            token_hash: &hash,
            display_prefix: &prefix,
            labels,
            expires_at,
            created_by,
        },
    )
    .await
    .expect("cannot create the enrollment token");
    tx.commit().await.expect("cannot commit the token");
    (value, id)
}

/// An HTTPS server that serves the control plane router with a test certificate.
pub struct TestServer {
    pub url: String,
    ca_file: PathBuf,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl TestServer {
    pub fn start(app: Router) -> Self {
        let (ca_pem, config) = test_tls();
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("cannot bind the test server");
        listener
            .set_nonblocking(true)
            .expect("cannot set the listener nonblocking");
        let addr = listener.local_addr().expect("the server address");
        let ca_file = write_temp_file("ca.pem", ca_pem.as_bytes());
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(serve(listener, app, config, shutdown_rx));
        Self {
            url: format!("https://localhost:{}", addr.port()),
            ca_file,
            shutdown: Some(shutdown_tx),
            task: Some(task),
        }
    }

    pub fn ca_file(&self) -> PathBuf {
        self.ca_file.clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
        let _ = std::fs::remove_file(&self.ca_file);
    }
}

async fn serve(
    listener: std::net::TcpListener,
    app: Router,
    config: Arc<rustls::ServerConfig>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let listener =
        tokio::net::TcpListener::from_std(listener).expect("cannot adopt the test listener");
    let acceptor = tokio_rustls::TlsAcceptor::from(config);
    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                let Ok((tcp, _)) = accepted else { break };
                let acceptor = acceptor.clone();
                let app = app.clone();
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(tcp).await else { return };
                    let service = hyper::service::service_fn(
                        move |request: hyper::Request<hyper::body::Incoming>| {
                            let app = app.clone();
                            async move {
                                let (mut parts, body) = request.into_parts();
                                parts.extensions.insert(ConnectInfo(SocketAddr::from((
                                    Ipv4Addr::LOCALHOST,
                                    0,
                                ))));
                                let request =
                                    hyper::Request::from_parts(parts, Body::new(body));
                                Ok::<_, std::convert::Infallible>(
                                    tower::ServiceExt::oneshot(app, request)
                                        .await
                                        .expect("the router failed"),
                                )
                            }
                        },
                    );
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(tls), service)
                        .await;
                });
            }
        }
    }
}

fn test_tls() -> (String, Arc<rustls::ServerConfig>) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let ca_key =
        KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the CA key");
    let mut ca_params = CertificateParams::new(Vec::new()).expect("cannot build the CA parameters");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "restichub-agent-test-ca");
    let ca = CertifiedIssuer::self_signed(ca_params, ca_key).expect("cannot sign the CA");

    let server_key =
        KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("cannot generate the server key");
    let mut server_params = CertificateParams::new(vec!["localhost".to_owned()])
        .expect("cannot build the server parameters");
    server_params
        .distinguished_name
        .push(DnType::CommonName, "localhost");
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = server_params
        .signed_by(&server_key, &ca)
        .expect("cannot sign the server certificate");

    let cert = CertificateDer::from(server_cert.der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der()));
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .expect("cannot build the server TLS configuration");
    (ca.pem(), Arc::new(config))
}

/// A test database, a tenant, a one-time token and a real agent listener.
pub struct StreamFixture {
    pub db: TestDatabase,
    pub tenant: TenantId,
    pub token: String,
    pub server: StreamServer,
}

/// Makes a fresh tenant, a one-time token and a running agent listener.
pub async fn stream_fixture(name: &str) -> Option<StreamFixture> {
    let db = test_database().await?;
    let tenant = create_tenant(db.database(), name).await;
    let creator = create_user(db.database(), tenant, "agent-admin", "phc", "admin").await;
    let (token, _) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &["host".to_owned()],
        OffsetDateTime::now_utc() + Duration::hours(1),
    )
    .await;
    let server = StreamServer::start(db.database()).await;
    Some(StreamFixture {
        db,
        tenant,
        token,
        server,
    })
}

/// The real agent listener over TLS with the installation CA (ADR-0026).
pub struct StreamServer {
    pub state: AppState,
    pub addr: SocketAddr,
    pub endpoint: String,
    ca: Arc<CaStore>,
    root_der: Vec<u8>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl StreamServer {
    /// Binds a loopback port and serves the listener with the installation CA.
    pub async fn start(db: &Database) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("cannot bind the listener");
        let addr = listener.local_addr().expect("the listener address");
        // The agent dials the endpoint host and verifies the name, so use the certificate DNS name.
        let endpoint = format!("https://localhost:{}", addr.port());
        let (state, ca) = pki_state(db, &endpoint).await;
        let sans = server_cert_sans(Some(&endpoint), addr);
        let issued = CaService::new(test_kek())
            .issue_server_certificate(&ca, &sans)
            .expect("cannot issue the server certificate");
        let root_der = ca.root().cert_der.clone();
        let tls = tls_config_from_issued(&issued, &root_der).expect("cannot build the TLS config");
        let (shutdown, rx) = tokio::sync::oneshot::channel();
        let serve_state = state.clone();
        let task = tokio::spawn(async move {
            let _ = serve_listener(
                listener,
                serve_state,
                tls,
                ListenerLimits::default(),
                async move {
                    let _ = rx.await;
                },
            )
            .await;
        });
        Self {
            state,
            addr,
            endpoint,
            ca,
            root_der,
            shutdown: Some(shutdown),
            task,
        }
    }

    /// The installation certificate authority, for a test that issues another server certificate.
    #[must_use]
    pub fn ca(&self) -> &Arc<CaStore> {
        &self.ca
    }

    /// Pinned installation CA, in DER.
    #[must_use]
    pub fn root_der(&self) -> &[u8] {
        &self.root_der
    }

    /// The pinned installation root CA in PEM, as `register --ca-file` reads it.
    #[must_use]
    pub fn root_pem(&self) -> String {
        restichub_cp::agent_listener::der_to_pem("CERTIFICATE", &self.root_der)
    }

    /// Revokes an agent as the API does: the status change and the stream close (FR-ID-04).
    pub async fn revoke(
        &self,
        db: &Database,
        tenant: TenantId,
        agent: Uuid,
        actor: restichub_db::UserId,
    ) {
        let mut tx = db
            .begin_tenant(tenant)
            .await
            .expect("cannot begin the transaction");
        restichub_db::agents::revoke(
            &mut tx,
            restichub_db::AgentId::from_uuid(agent),
            actor,
            OffsetDateTime::now_utc(),
        )
        .await
        .expect("cannot revoke the agent");
        tx.commit().await.expect("cannot commit the revoke");
        self.state
            .agent_registry()
            .close_by_agent(restichub_db::AgentId::from_uuid(agent));
    }
}

impl Drop for StreamServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

/// Builds the application state with a real certificate authority and job signing key set.
pub async fn pki_state(db: &Database, endpoint: &str) -> (AppState, Arc<CaStore>) {
    let provider = test_kek();
    let ca = CaService::new(provider.clone());
    let store = Arc::new(
        ca.ensure_initialized(db)
            .await
            .expect("cannot create the CA"),
    );
    let job_signing = restichub_cp::job_signing::JobSigningService::new(provider.clone());
    job_signing
        .ensure_initialized(db)
        .await
        .expect("cannot create the job signing key set");
    let auth = AuthState::new(AuthConfig::default());
    let state = AppState::with_auth(db.clone(), auth)
        .with_kek(provider)
        .with_tenancy(TenancyConfig::multi())
        .with_pki(store.clone(), job_signing, Some(endpoint.to_owned()));
    (state, store)
}

/// Reads `last_seen_at` of an agent through a tenant transaction.
pub async fn last_seen(db: &Database, tenant: TenantId, agent: Uuid) -> Option<OffsetDateTime> {
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

/// Sets `last_seen_at` of an agent through the owner connection.
pub async fn set_last_seen(
    db: &TestDatabase,
    tenant: TenantId,
    agent: Uuid,
    value: OffsetDateTime,
) {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query("UPDATE agents SET last_seen_at = $2 WHERE id = $1")
        .bind(agent)
        .bind(value)
        .execute(&mut *tx)
        .await
        .expect("cannot set last_seen_at");
    tx.commit().await.expect("cannot commit last_seen_at");
}

/// The number of retired job signing keys in the platform tenant (ADR-0026).
pub async fn retired_keys(db: &Database) -> usize {
    let mut tx = db
        .begin_tenant(restichub_db::PLATFORM_TENANT_ID)
        .await
        .expect("cannot begin the transaction");
    let rows = signing::list(&mut tx).await.expect("cannot list the keys");
    tx.commit().await.expect("cannot commit the read");
    rows.iter().filter(|row| row.retired_at.is_some()).count()
}

/// The identifier of the current active job signing key (ADR-0026).
pub async fn current_key_id(db: &Database) -> String {
    let mut tx = db
        .begin_tenant(restichub_db::PLATFORM_TENANT_ID)
        .await
        .expect("cannot begin the transaction");
    let rows = signing::list(&mut tx).await.expect("cannot list the keys");
    tx.commit().await.expect("cannot commit the read");
    rows.into_iter()
        .find(|row| {
            row.role == restichub_db::SigningKeyRole::Current
                && row.retired_at.is_none()
                && row.retire_after.is_none()
        })
        .expect("the current key is present")
        .id
        .to_string()
}

/// A gRPC listener that accepts the agent stream and never sends a message (FR-ID-07).
///
/// The listener presents a certificate from the installation CA and verifies the agent against the
/// installation root, so the agent reaches it over mTLS. It counts the accepted streams.
pub struct SilentServer {
    addr: SocketAddr,
    connections: Arc<std::sync::atomic::AtomicUsize>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

/// How the silent server ends a stream (FR-ID-07, FR-ID-08).
#[derive(Clone, Copy, PartialEq, Eq)]
enum SilentMode {
    /// Keeps the stream open and sends no message.
    Silent,
    /// Ends the stream at once, without a message.
    Closing,
    /// Sends one ping and then fails, as a control plane restart does.
    PingThenError,
}

impl SilentServer {
    /// Serves a silent agent stream on a loopback port (FR-ID-07).
    pub async fn start(ca: &CaStore, root_der: &[u8]) -> Self {
        Self::start_mode(ca, root_der, SilentMode::Silent).await
    }

    /// Serves a stream that ends at once, for the reconnect-backoff test (FR-ID-08).
    pub async fn start_closing(ca: &CaStore, root_der: &[u8]) -> Self {
        Self::start_mode(ca, root_der, SilentMode::Closing).await
    }

    /// Serves a stream that answers one ping and then fails, for the healthy-stream test (FR-ID-08).
    pub async fn start_ping_then_error(ca: &CaStore, root_der: &[u8]) -> Self {
        Self::start_mode(ca, root_der, SilentMode::PingThenError).await
    }

    async fn start_mode(ca: &CaStore, root_der: &[u8], mode: SilentMode) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("cannot bind the silent server");
        let addr = listener.local_addr().expect("the silent server address");
        let endpoint = format!("https://localhost:{}", addr.port());
        let sans = server_cert_sans(Some(&endpoint), addr);
        let issued = CaService::new(test_kek())
            .issue_server_certificate(ca, &sans)
            .expect("cannot issue the server certificate");
        let mut chain = der_to_pem("CERTIFICATE", &issued.cert_der);
        for der in &issued.chain_der {
            chain.push_str(&der_to_pem("CERTIFICATE", der));
        }
        let tls = ServerTlsConfig::new()
            .identity(Identity::from_pem(
                chain,
                der_to_pem("PRIVATE KEY", issued.key_der.as_slice()),
            ))
            .client_ca_root(Certificate::from_pem(der_to_pem("CERTIFICATE", root_der)));
        let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let service = SilentService {
            connections: Arc::clone(&connections),
            mode,
        };
        let (shutdown, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let incoming = TcpListenerStream::new(listener);
            let _ = Server::builder()
                .tls_config(tls)
                .expect("cannot build the silent TLS config")
                .add_service(AgentServiceServer::new(service))
                .serve_with_incoming_shutdown(incoming, async move {
                    let _ = rx.await;
                })
                .await;
        });
        Self {
            addr,
            connections,
            shutdown: Some(shutdown),
            task,
        }
    }

    /// Address that the agent dials.
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!("https://localhost:{}", self.addr.port())
    }

    /// Count of the accepted agent streams.
    #[must_use]
    pub fn connections(&self) -> usize {
        self.connections.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Drop for SilentServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

/// The gRPC service of the silent server. It sends no control message (FR-ID-07), closes at once
/// (FR-ID-08), or answers one ping and then fails (FR-ID-08).
#[derive(Clone)]
struct SilentService {
    connections: Arc<std::sync::atomic::AtomicUsize>,
    mode: SilentMode,
}

#[tonic::async_trait]
impl AgentService for SilentService {
    type ConnectStream = Pin<Box<dyn Stream<Item = Result<ControlMessage, Status>> + Send>>;

    async fn connect(
        &self,
        request: Request<Streaming<AgentMessage>>,
    ) -> Result<Response<Self::ConnectStream>, Status> {
        self.connections
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut inbound = request.into_inner();
        tokio::spawn(async move { while let Ok(Some(_)) = inbound.message().await {} });
        let (tx, rx) = tokio::sync::mpsc::channel::<ControlMessage>(1);
        match self.mode {
            SilentMode::Silent => {
                // The sender is leaked, so the outbound stream stays open and sends nothing.
                std::mem::forget(tx);
                Ok(Response::new(Box::pin(ReceiverStream::new(rx).map(Ok))))
            }
            SilentMode::Closing => Ok(Response::new(Box::pin(ReceiverStream::new(rx).map(Ok)))),
            SilentMode::PingThenError => {
                let ping = ControlMessage {
                    payload: Some(control_message::Payload::Ping(Ping { nonce: 1 })),
                };
                let stream = tokio_stream::iter(vec![
                    Ok(ping),
                    Err(Status::unavailable("the control plane restarted")),
                ]);
                Ok(Response::new(Box::pin(stream)))
            }
        }
    }
}

pub mod api;
pub mod hooks;
pub mod jobs;
pub mod steps;

#[allow(unused_imports)]
pub use api::*;
#[allow(unused_imports)]
pub use jobs::*;
#[allow(unused_imports)]
pub use steps::*;
