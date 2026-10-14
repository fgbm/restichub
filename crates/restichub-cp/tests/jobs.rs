//! The repository init and test-access jobs (FR-RP-02, FR-PR-03, FR-PR-05, AC-RP-01).
//!
//! The tests start the real agent listener, enroll an agent over HTTPS, open the stream and drive
//! the web API. They cover the signed job that reaches the agent, the result that updates the job
//! and the repository state, the `config_error` of a wrong password (AC-RP-01), the typed
//! `not_initialized` result, the offline agent and the canary scan of the secret.

#![allow(clippy::expect_used)]

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use restichub_cp::agent_listener::{
    ListenerLimits, der_to_pem, serve_listener, server_cert_sans, tls_config_from_issued,
};
use restichub_cp::pki::{CaService, CaStore};
use restichub_cp::server::AppState;
use restichub_db::test_util::TestDatabase;
use restichub_db::{Database, PLATFORM_TENANT_ID, SigningKeyRole, TenantId, signing};
use restichub_emulator::enroll::{EnrollFacts, EnrollTls, EnrolledIdentity, enroll_tls};
use restichub_proto::agent::v1::{
    AgentHello, AgentMessage, ControlMessage, ErrorDetail, JobAck, JobResult, JobStatus, SignedJob,
    agent_message, agent_service_client::AgentServiceClient, control_message,
};
use restichub_proto::signing::{Expected, KeyRing, verify_job};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tonic::Streaming;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};
use uuid::Uuid;

use common::{
    api_router, create_tenant_with_key, create_user, pki_state, request,
    seed_enrollment_token_value, seed_session, send,
};

/// One running server with its trust material and its address.
struct Listener {
    state: AppState,
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
            root_der,
            shutdown: Some(shutdown),
            task,
        }
    }

    fn https_url(&self) -> String {
        format!("https://127.0.0.1:{}", self.addr.port())
    }

    async fn enroll(&self, token: &str) -> EnrolledIdentity {
        let tls = EnrollTls {
            ca_der: self.root_der.clone(),
            server_name: "localhost".to_owned(),
        };
        let facts = EnrollFacts {
            hostname: "job-host".to_owned(),
            os: "linux".to_owned(),
            arch: "amd64".to_owned(),
            agent_version: "0.1.0".to_owned(),
            restic_version: "0.19.0".to_owned(),
        };
        enroll_tls(&self.https_url(), token, &facts, &tls)
            .await
            .expect("the agent must enroll over HTTPS")
    }

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

fn client_chain_pem(identity: &EnrolledIdentity) -> String {
    let mut pem = der_to_pem("CERTIFICATE", &identity.certificate_der);
    for der in &identity.ca_chain_der {
        pem.push_str(&der_to_pem("CERTIFICATE", der));
    }
    pem
}

/// The enrollment, the listener and an Admin session of one tenant.
struct Setup {
    listener: Listener,
    tenant: TenantId,
    admin: String,
    identity: EnrolledIdentity,
}

impl Setup {
    fn agent_id(&self) -> Uuid {
        Uuid::parse_str(&self.identity.agent_id).expect("the agent id")
    }
}

async fn setup(db: &TestDatabase) -> Setup {
    let (state, ca) = pki_state(db.database()).await;
    let tenant = create_tenant_with_key(db.database(), "job-tenant").await;
    let creator = create_user(db.database(), tenant, "job-admin", "phc", "admin").await;
    let (token, _) = seed_enrollment_token_value(
        db.database(),
        tenant,
        creator,
        &[],
        time::OffsetDateTime::now_utc() + time::Duration::hours(1),
    )
    .await;
    let listener = Listener::start(state, ca).await;
    let identity = listener.enroll(&token).await;
    let admin = seed_session(db.database(), tenant, "job-session", "admin").await;
    Setup {
        listener,
        tenant,
        admin,
        identity,
    }
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

/// Opens the connection and drains the `ServerHello` and the `SigningKeyUpdate`.
async fn open_stream(
    client: &mut AgentServiceClient<tonic::transport::Channel>,
) -> (mpsc::Sender<AgentMessage>, Streaming<ControlMessage>) {
    let (tx, rx) = mpsc::channel::<AgentMessage>(16);
    let response = client
        .connect(ReceiverStream::new(rx))
        .await
        .expect("the stream must open");
    tx.send(hello()).await.expect("cannot send the hello");
    let mut inbound = response.into_inner();
    let mut hello_seen = false;
    let mut update_seen = false;
    for _ in 0..8 {
        let message = next_message(&mut inbound).await;
        match message.payload {
            Some(control_message::Payload::ServerHello(_)) => hello_seen = true,
            Some(control_message::Payload::SigningKeyUpdate(_)) => update_seen = true,
            _ => {}
        }
        if hello_seen && update_seen {
            break;
        }
    }
    assert!(hello_seen, "the ServerHello must arrive");
    assert!(update_seen, "the key update must arrive");
    (tx, inbound)
}

async fn next_message(inbound: &mut Streaming<ControlMessage>) -> ControlMessage {
    tokio::time::timeout(Duration::from_secs(10), inbound.message())
        .await
        .expect("the message must arrive")
        .expect("the stream must not fail")
        .expect("the stream must not end")
}

/// Reads the next `SignedJob` from the stream, skipping a heartbeat.
async fn read_signed_job(inbound: &mut Streaming<ControlMessage>) -> SignedJob {
    for _ in 0..16 {
        let message = next_message(inbound).await;
        if let Some(control_message::Payload::SignedJob(signed)) = message.payload {
            return signed;
        }
    }
    panic!("the signed job must arrive");
}

/// The active Ed25519 key that signs the jobs of the installation.
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

/// Verifies a signed job with the current key and returns the decoded job.
async fn verify_with_current_key(
    db: &Database,
    signed: &SignedJob,
    setup: &Setup,
) -> restichub_proto::agent::v1::Job {
    let (key_id, key_bytes) = current_key(db).await;
    assert_eq!(signed.signer_key_id, key_id);
    let mut ring = KeyRing::new();
    ring.insert(
        key_id,
        ed25519_dalek::VerifyingKey::from_bytes(&key_bytes).expect("a valid key"),
    )
    .expect("the ring must accept the current key");
    verify_job(
        signed,
        &ring,
        &Expected {
            agent_id: setup.identity.agent_id.clone(),
            tenant_id: setup.tenant.to_string(),
            now: prost_types::Timestamp {
                seconds: time::OffsetDateTime::now_utc().unix_timestamp(),
                nanos: 0,
            },
        },
    )
    .expect("the agent must verify the job")
}

/// Creates a local repository with the maintenance agent and returns its identifier.
async fn create_repository(app: &Router, cookie: &str, agent_id: Uuid, password: &str) -> Uuid {
    let body = json!({
        "name": "repo-init",
        "backend": {"kind": "local", "path": "/srv/backup/restic"},
        "password": password,
        "maintenance_agent_id": agent_id.to_string(),
    });
    let (status, _, repository) = send(
        app,
        request("POST", "/api/v1/repositories", Some(cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{repository}");
    Uuid::parse_str(
        repository
            .get("id")
            .and_then(Value::as_str)
            .expect("the repository id"),
    )
    .expect("the repository id is a UUID")
}

async fn get_job(app: &Router, cookie: &str, job_id: &str) -> Value {
    let (status, _, body) = send(
        app,
        request("GET", &format!("/api/v1/jobs/{job_id}"), Some(cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn get_repository(app: &Router, cookie: &str, repository_id: Uuid) -> Value {
    let (status, _, body) = send(
        app,
        request(
            "GET",
            &format!("/api/v1/repositories/{repository_id}"),
            Some(cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// Puts one job outcome on the wire, with an optional error code.
async fn send_result(
    tx: &mpsc::Sender<AgentMessage>,
    job_id: &str,
    outcome: JobStatus,
    error_code: Option<&str>,
) {
    let error = error_code.map(|code| ErrorDetail {
        code: code.to_owned(),
        params: Vec::new(),
    });
    tx.send(AgentMessage {
        payload: Some(agent_message::Payload::JobResult(JobResult {
            job_id: job_id.to_owned(),
            status: 0,
            steps_total: 1,
            steps_succeeded: u32::from(outcome == JobStatus::Succeeded),
            steps_failed: u32::from(outcome != JobStatus::Succeeded),
            outcome: outcome as i32,
            error,
        })),
    })
    .await
    .expect("cannot send the job result");
}

/// AC-RP-01: the signed init job reaches the agent, the result updates the job and the repository.
#[tokio::test]
async fn ac_rp_01_the_signed_init_job_reaches_the_agent_and_initializes() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let setup = setup(&db).await;
    let app = api_router(setup.listener.state.clone());
    let canary = format!("CANARY-{}", Uuid::now_v7());
    let repository_id = create_repository(&app, &setup.admin, setup.agent_id(), &canary).await;

    let mut client = setup.listener.channel(&setup.identity).await;
    let (tx, mut inbound) = open_stream(&mut client).await;

    let (status, _, job) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/repositories/{repository_id}/init"),
            Some(&setup.admin),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    assert_eq!(job.get("kind").and_then(Value::as_str), Some("init"));
    assert_eq!(job.get("status").and_then(Value::as_str), Some("sent"));
    let job_id = job
        .get("id")
        .and_then(Value::as_str)
        .expect("the job id")
        .to_owned();

    // The signed job reaches the agent and verifies with the current key (FR-PR-03).
    let signed = read_signed_job(&mut inbound).await;
    let job_proto = verify_with_current_key(db.database(), &signed, &setup).await;
    assert_eq!(job_proto.steps.len(), 2, "test_access then init");
    let access = &job_proto.repositories[0];
    assert_eq!(access.password, canary, "the job carries the password");

    // The acknowledgement moves the job to running (FR-PR-06).
    tx.send(AgentMessage {
        payload: Some(agent_message::Payload::JobAck(JobAck {
            job_id: job_id.clone(),
        })),
    })
    .await
    .expect("cannot send the acknowledgement");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let running = get_job(&app, &setup.admin, &job_id).await;
    assert_eq!(
        running.get("status").and_then(Value::as_str),
        Some("running")
    );

    // A successful result sets the job and the repository state (FR-RP-02).
    send_result(&tx, &job_id, JobStatus::Succeeded, None).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let finished = get_job(&app, &setup.admin, &job_id).await;
    assert_eq!(
        finished.get("status").and_then(Value::as_str),
        Some("succeeded")
    );
    assert_eq!(
        finished.get("result_code").and_then(Value::as_str),
        Some("succeeded")
    );
    let repository = get_repository(&app, &setup.admin, repository_id).await;
    assert_eq!(
        repository.get("initialized").and_then(Value::as_bool),
        Some(true)
    );

    db.cleanup().await;
}

/// AC-RP-01: a wrong password gives `config_error` and no initialization.
#[tokio::test]
async fn ac_rp_01_a_wrong_password_gives_config_error() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let setup = setup(&db).await;
    let app = api_router(setup.listener.state.clone());
    let repository_id =
        create_repository(&app, &setup.admin, setup.agent_id(), "correct-password").await;

    let mut client = setup.listener.channel(&setup.identity).await;
    let (tx, mut inbound) = open_stream(&mut client).await;

    let (status, _, job) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/repositories/{repository_id}/init"),
            Some(&setup.admin),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    let job_id = job
        .get("id")
        .and_then(Value::as_str)
        .expect("the job id")
        .to_owned();
    let _ = read_signed_job(&mut inbound).await;

    send_result(&tx, &job_id, JobStatus::ConfigError, Some("wrong_password")).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let finished = get_job(&app, &setup.admin, &job_id).await;
    assert_eq!(
        finished.get("status").and_then(Value::as_str),
        Some("failed")
    );
    assert_eq!(
        finished.get("result_code").and_then(Value::as_str),
        Some("config_error")
    );
    assert_eq!(
        finished.get("result_message").and_then(Value::as_str),
        Some("wrong_password")
    );
    let repository = get_repository(&app, &setup.admin, repository_id).await;
    assert_eq!(
        repository.get("initialized").and_then(Value::as_bool),
        Some(false)
    );

    db.cleanup().await;
}

/// FR-RP-02: a test-access that finds no repository gives the typed `not_initialized` result.
#[tokio::test]
async fn a_test_access_on_a_missing_repository_gives_not_initialized() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let setup = setup(&db).await;
    let app = api_router(setup.listener.state.clone());
    let repository_id =
        create_repository(&app, &setup.admin, setup.agent_id(), "correct-password").await;

    let mut client = setup.listener.channel(&setup.identity).await;
    let (tx, mut inbound) = open_stream(&mut client).await;

    let (status, _, job) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/repositories/{repository_id}/test-access"),
            Some(&setup.admin),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    assert_eq!(job.get("kind").and_then(Value::as_str), Some("test_access"));
    let job_id = job
        .get("id")
        .and_then(Value::as_str)
        .expect("the job id")
        .to_owned();

    let signed = read_signed_job(&mut inbound).await;
    let job_proto = verify_with_current_key(db.database(), &signed, &setup).await;
    assert_eq!(job_proto.steps.len(), 1, "test_access only");

    send_result(
        &tx,
        &job_id,
        JobStatus::NotInitialized,
        Some("repository_missing"),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let finished = get_job(&app, &setup.admin, &job_id).await;
    assert_eq!(
        finished.get("result_code").and_then(Value::as_str),
        Some("not_initialized")
    );
    let repository = get_repository(&app, &setup.admin, repository_id).await;
    assert_eq!(
        repository.get("initialized").and_then(Value::as_bool),
        Some(false)
    );

    db.cleanup().await;
}

/// The MVP holds no queue: an offline agent refuses the job at once with HTTP 409.
#[tokio::test]
async fn an_offline_agent_is_refused_with_a_clear_error() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let setup = setup(&db).await;
    let app = api_router(setup.listener.state.clone());
    let repository_id =
        create_repository(&app, &setup.admin, setup.agent_id(), "correct-password").await;

    // No stream is open, so the maintenance agent is offline.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/repositories/{repository_id}/test-access"),
            Some(&setup.admin),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("agent_offline")
    );
    assert_eq!(common::count(db.database(), setup.tenant, "jobs").await, 0);

    db.cleanup().await;
}

/// AC-RP-01 / FR-PR-05: the canary secret never reaches the job row, the audit or a response.
#[tokio::test]
async fn ac_rp_01_the_secret_never_reaches_the_job_row_or_the_audit() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let setup = setup(&db).await;
    let app = api_router(setup.listener.state.clone());
    let canary = format!("CANARY-{}", Uuid::now_v7());
    let repository_id = create_repository(&app, &setup.admin, setup.agent_id(), &canary).await;

    let mut client = setup.listener.channel(&setup.identity).await;
    let (tx, mut inbound) = open_stream(&mut client).await;

    let (status, _, job) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/repositories/{repository_id}/init"),
            Some(&setup.admin),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    let job_id = job
        .get("id")
        .and_then(Value::as_str)
        .expect("the job id")
        .to_owned();
    let _ = read_signed_job(&mut inbound).await;
    send_result(&tx, &job_id, JobStatus::Succeeded, None).await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // The job row and the audit records hold no canary.
    let dump = tenant_dump(&db, setup.tenant).await;
    assert!(!dump.contains(&canary), "the job rows hold the canary");
    let audit = audit_dump(&db, setup.tenant).await;
    assert!(
        !audit.contains(&canary),
        "the audit records hold the canary"
    );
    // The delivery is recorded, without the value (AC-RP-03).
    assert!(
        audit.contains("secret_delivered"),
        "the audit records hold no delivery record"
    );
    // The responses must never hold the canary value.
    let job_view = get_job(&app, &setup.admin, &job_id).await.to_string();
    assert!(!job_view.contains(&canary), "the job view holds the canary");
    let repository_view = get_repository(&app, &setup.admin, repository_id)
        .await
        .to_string();
    assert!(
        !repository_view.contains(&canary),
        "the repository view holds the canary"
    );

    db.cleanup().await;
}

/// The role matrix: a Viewer cannot start an init or a test-access job.
#[tokio::test]
async fn a_viewer_cannot_start_a_job() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let setup = setup(&db).await;
    let app = api_router(setup.listener.state.clone());
    let repository_id =
        create_repository(&app, &setup.admin, setup.agent_id(), "correct-password").await;
    let viewer = seed_session(db.database(), setup.tenant, "job-viewer", "viewer").await;

    for path in ["init", "test-access"] {
        let (status, _, _) = send(
            &app,
            request(
                "POST",
                &format!("/api/v1/repositories/{repository_id}/{path}"),
                Some(&viewer),
                Some(json!({})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }

    db.cleanup().await;
}

/// The full row dump of a tenant, read through the owner connection.
async fn tenant_dump(db: &TestDatabase, tenant: TenantId) -> String {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    let (jobs,): (String,) =
        sqlx::query_as("SELECT coalesce(string_agg(row_to_json(t)::text, E'\\n'), '') FROM jobs t")
            .fetch_one(&mut *tx)
            .await
            .expect("cannot dump the jobs");
    tx.commit().await.expect("cannot commit");
    jobs
}

/// The audit records of a tenant, read through the owner connection.
async fn audit_dump(db: &TestDatabase, tenant: TenantId) -> String {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    let (audit,): (String,) = sqlx::query_as(
        "SELECT coalesce(string_agg(row_to_json(t)::text, E'\\n'), '') FROM audit_log t",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("cannot dump the audit");
    tx.commit().await.expect("cannot commit");
    audit
}
