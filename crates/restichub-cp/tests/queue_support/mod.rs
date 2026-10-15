//! The shared fixture of the queue tests: a control plane with a manual clock, the real agent
//! listener, enrolled agents on open streams and the API objects that a plan needs.

#![allow(clippy::expect_used, dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use restichub_cp::agent_listener::{
    ListenerLimits, der_to_pem, serve_listener, server_cert_sans, tls_config_from_issued,
};
use restichub_cp::clock::ManualClock;
use restichub_cp::config::{AuthConfig, TenancyConfig};
use restichub_cp::pki::{CaService, CaStore};
use restichub_cp::queue::QueueConfig;
use restichub_cp::server::{AppState, AuthState};
use restichub_db::test_util::TestDatabase;
use restichub_db::{AgentId, PLATFORM_TENANT_ID, SigningKeyRole, TenantId, signing};
use restichub_emulator::enroll::{EnrollFacts, EnrollTls, EnrolledIdentity, enroll_tls};
use restichub_proto::agent::v1::{
    AgentHello, AgentMessage, ControlMessage, ForgetGroup, ForgetResult, ForgetSnapshot, Job,
    JobAck, JobResult, JobStatus, SignedJob, StepResult, StepStatus, agent_message,
    agent_service_client::AgentServiceClient, control_message, step_result,
};
use restichub_proto::signing::{Expected, KeyRing, verify_job};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tonic::Streaming;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};
use uuid::Uuid;

use crate::common::{
    self, api_router, bearer_request, create_tenant_with_key, create_user,
    seed_enrollment_token_value, send,
};

pub struct Listener {
    pub addr: SocketAddr,
    pub root_der: Vec<u8>,
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
        let task = tokio::spawn(async move {
            let _ = serve_listener(
                listener,
                state,
                tls,
                ListenerLimits::default(),
                async move {
                    let _ = rx.await;
                },
            )
            .await;
        });
        Self {
            addr,
            root_der,
            shutdown: Some(shutdown),
            task,
        }
    }

    pub fn https_url(&self) -> String {
        format!("https://127.0.0.1:{}", self.addr.port())
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

pub fn client_chain_pem(identity: &EnrolledIdentity) -> String {
    let mut pem = der_to_pem("CERTIFICATE", &identity.certificate_der);
    for der in &identity.ca_chain_der {
        pem.push_str(&der_to_pem("CERTIFICATE", der));
    }
    pem
}

/// One control plane instance on a test database, with a manual clock.
pub struct Env {
    pub db: TestDatabase,
    pub state: AppState,
    pub clock: Arc<ManualClock>,
    pub listener: Listener,
    pub app: Router,
    pub tenant: TenantId,
    pub admin: String,
    pub operator: String,
    pub viewer: String,
    creator: restichub_db::UserId,
}

/// The queue settings of the tests: the delays are short, so a test needs no sleep.
pub fn test_queue() -> QueueConfig {
    QueueConfig {
        interval: Duration::from_millis(100),
        lease: Duration::from_secs(60),
        agent_limit: 10,
        repository_limit: 10,
        global_limit: 100,
        lock_retry_limit: 5,
        lock_retry_base: Duration::from_secs(30),
        max_deliveries: 5,
    }
}

impl Env {
    pub async fn start(db: TestDatabase, queue: QueueConfig) -> Self {
        Self::start_with_browse_rate(db, queue, None).await
    }

    /// Like [`Env::start`], with a smaller snapshot browsing limit for each actor.
    pub async fn start_with_browse_rate(
        db: TestDatabase,
        queue: QueueConfig,
        browse_rate: Option<u32>,
    ) -> Self {
        Self::start_configured(db, queue, browse_rate, None).await
    }

    /// Like [`Env::start`], with a short certificate lifetime for the agents (FR-ID-03).
    pub async fn start_with_lifetime(
        db: TestDatabase,
        queue: QueueConfig,
        lifetime: Duration,
    ) -> Self {
        Self::start_configured(db, queue, None, Some(lifetime)).await
    }

    async fn start_configured(
        db: TestDatabase,
        queue: QueueConfig,
        browse_rate: Option<u32>,
        lifetime: Option<Duration>,
    ) -> Self {
        let clock = Arc::new(ManualClock::new(time::OffsetDateTime::now_utc()));
        let provider = common::test_kek();
        let ca_service = CaService::new(provider.clone());
        let store = Arc::new(
            ca_service
                .ensure_initialized(db.database())
                .await
                .expect("cannot create the CA"),
        );
        let signing_service = restichub_cp::job_signing::JobSigningService::new(provider.clone());
        signing_service
            .ensure_initialized(db.database())
            .await
            .expect("cannot create the job signing key set");
        let shared: Arc<dyn restichub_cp::clock::Clock> = clock.clone();
        let mut auth = AuthState::new(AuthConfig::default()).with_clock(shared);
        if let Some(rate) = browse_rate {
            auth = auth.with_browse_rate(rate);
        }
        let state = AppState::with_auth(db.database().clone(), auth)
            .with_kek(provider)
            .with_tenancy(TenancyConfig::multi())
            .with_queue(queue)
            .with_pki(
                store.clone(),
                signing_service,
                Some(common::AGENT_ENDPOINT.to_owned()),
            );
        let state = match lifetime {
            Some(lifetime) => state.with_agent_cert_lifetime(lifetime),
            None => state,
        };
        let listener = Listener::start(state.clone(), store).await;
        let tenant = create_tenant_with_key(db.database(), "queue-tenant").await;
        let creator = create_user(db.database(), tenant, "queue-admin", "phc", "admin").await;
        // API tokens do not idle out, so a test may move the manual clock by hours.
        let admin = common::seed_api_token(db.database(), tenant, "admin").await;
        let operator = common::seed_api_token(db.database(), tenant, "operator").await;
        let viewer = common::seed_api_token(db.database(), tenant, "viewer").await;
        let app = api_router(state.clone());
        Self {
            db,
            state,
            clock,
            listener,
            app,
            tenant,
            admin,
            operator,
            viewer,
            creator,
        }
    }

    /// An owner-level transaction scoped to the tenant, for the rows that the API does not expose.
    pub async fn scoped(&self) -> sqlx::Transaction<'static, sqlx::Postgres> {
        let mut tx = self.db.owner_pool().begin().await.expect("cannot begin");
        sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
            .bind(self.tenant.to_string())
            .execute(&mut *tx)
            .await
            .expect("cannot set the tenant scope");
        tx
    }

    pub fn now(&self) -> time::OffsetDateTime {
        use restichub_cp::clock::Clock;
        self.clock.now()
    }

    /// Runs one dispatcher pass at the time of the manual clock.
    pub async fn pass(&self) {
        restichub_cp::queue::run_once(&self.state, self.now())
            .await
            .expect("the dispatcher pass");
    }

    /// The user who created the enrollment tokens of the environment.
    pub fn creator_id(&self) -> restichub_db::UserId {
        self.creator
    }

    /// Spends a one-time enrollment token over HTTPS and returns the issued identity.
    pub async fn enroll_agent(&self, hostname: &str) -> EnrolledIdentity {
        let (token, _) = seed_enrollment_token_value(
            self.db.database(),
            self.tenant,
            self.creator,
            &[],
            time::OffsetDateTime::now_utc() + time::Duration::hours(1),
        )
        .await;
        self.enroll_with_token_named(&token, hostname).await
    }

    /// Spends the given token over HTTPS.
    pub async fn enroll_with_token(&self, token: &str) -> EnrolledIdentity {
        self.enroll_with_token_named(token, "token-host").await
    }

    async fn enroll_with_token_named(&self, token: &str, hostname: &str) -> EnrolledIdentity {
        let tls = EnrollTls {
            ca_der: self.listener.root_der.clone(),
            server_name: "localhost".to_owned(),
        };
        let facts = EnrollFacts {
            hostname: hostname.to_owned(),
            os: "linux".to_owned(),
            arch: "amd64".to_owned(),
            agent_version: "0.1.0".to_owned(),
            restic_version: "0.19.0".to_owned(),
        };
        enroll_tls(&self.listener.https_url(), token, &facts, &tls)
            .await
            .expect("the agent must enroll over HTTPS")
    }

    /// Enrolls an agent, opens its stream and drains the hello messages.
    pub async fn connect_agent(&self, hostname: &str) -> FakeAgent {
        let capabilities = restichub_proto::AgentCapability::ALL
            .iter()
            .map(|capability| capability.as_str().to_owned())
            .collect();
        self.connect_agent_with(hostname, capabilities).await
    }

    /// Like [`Env::connect_agent`], for an agent that announces `capabilities` only.
    pub async fn connect_agent_with(&self, hostname: &str, capabilities: Vec<String>) -> FakeAgent {
        let identity = self.enroll_agent(hostname).await;
        self.open_agent_stream(identity, capabilities, default_restore_roots(), None)
            .await
    }

    /// Like [`Env::connect_agent`], for an agent that reports the trust mode for server scripts
    /// and the interpreters it found (FR-HK-08).
    pub async fn connect_agent_trusting(
        &self,
        hostname: &str,
        enabled: bool,
        interpreters: Vec<restichub_proto::agent::v1::ScriptInterpreter>,
    ) -> FakeAgent {
        self.connect_agent_reporting(hostname, scripts_report(enabled, interpreters))
            .await
    }

    /// Like [`Env::connect_agent`], for an agent that reports a hook allow-list (FR-HK-04).
    pub async fn connect_agent_reporting(&self, hostname: &str, report: HookReport) -> FakeAgent {
        let capabilities = restichub_proto::AgentCapability::ALL
            .iter()
            .map(|capability| capability.as_str().to_owned())
            .collect();
        let identity = self.enroll_agent(hostname).await;
        self.open_agent_stream(
            identity,
            capabilities,
            default_restore_roots(),
            Some(report),
        )
        .await
    }

    /// Opens a new stream of an enrolled identity with another hook report, as a restart after a
    /// change of the allow-list does.
    pub async fn reconnect_agent_reporting(
        &self,
        agent: &FakeAgent,
        capabilities: Vec<String>,
        report: HookReport,
    ) -> FakeAgent {
        self.open_agent_stream(
            agent.identity.clone(),
            capabilities,
            default_restore_roots(),
            Some(report),
        )
        .await
    }

    /// Like [`Env::connect_agent`], for an agent that reports `roots` as its restore roots.
    pub async fn connect_agent_with_roots(&self, hostname: &str, roots: Vec<String>) -> FakeAgent {
        let capabilities = restichub_proto::AgentCapability::ALL
            .iter()
            .map(|capability| capability.as_str().to_owned())
            .collect();
        let identity = self.enroll_agent(hostname).await;
        self.open_agent_stream(identity, capabilities, roots, None)
            .await
    }

    /// Opens a new stream for an enrolled identity, as a restart or a downgrade does. This closes
    /// the stream that the identity held before.
    pub async fn reconnect_agent(&self, agent: &FakeAgent, capabilities: Vec<String>) -> FakeAgent {
        self.open_agent_stream(
            agent.identity.clone(),
            capabilities,
            default_restore_roots(),
            None,
        )
        .await
    }

    async fn open_agent_stream(
        &self,
        identity: EnrolledIdentity,
        capabilities: Vec<String>,
        roots: Vec<String>,
        report: Option<HookReport>,
    ) -> FakeAgent {
        open_stream_on(
            &self.listener,
            &self.state,
            identity,
            capabilities,
            roots,
            report,
        )
        .await
    }

    /// Opens a stream for an identity that the test enrolled or rotated by itself.
    pub async fn connect_enrolled(&self, identity: EnrolledIdentity) -> FakeAgent {
        let capabilities = restichub_proto::AgentCapability::ALL
            .iter()
            .map(|capability| capability.as_str().to_owned())
            .collect();
        self.open_agent_stream(identity, capabilities, default_restore_roots(), None)
            .await
    }

    /// Opens a stream with the given identity without the hello, as a test that plays a wrong or
    /// a changed agent needs. The error is the status of a refused open.
    pub async fn open_raw(
        &self,
        identity: &EnrolledIdentity,
    ) -> Result<(mpsc::Sender<AgentMessage>, Streaming<ControlMessage>), tonic::Status> {
        raw_stream(&self.listener, identity).await
    }

    /// Starts a second control plane instance on the same database, with its own stream
    /// registry and its own listener.
    pub async fn start_instance(&self) -> Instance {
        let provider = common::test_kek();
        let store = Arc::new(
            CaService::new(provider.clone())
                .load(self.db.database())
                .await
                .expect("cannot load the CA")
                .expect("the CA exists"),
        );
        let signing_service = restichub_cp::job_signing::JobSigningService::new(provider.clone());
        signing_service
            .ensure_initialized(self.db.database())
            .await
            .expect("cannot load the job signing key set");
        let shared: Arc<dyn restichub_cp::clock::Clock> = self.clock.clone();
        let state = AppState::with_auth(
            self.db.database().clone(),
            AuthState::new(AuthConfig::default()).with_clock(shared),
        )
        .with_kek(provider)
        .with_tenancy(TenancyConfig::multi())
        .with_queue(test_queue())
        .with_pki(
            store.clone(),
            signing_service,
            Some(common::AGENT_ENDPOINT.to_owned()),
        );
        let listener = Listener::start(state.clone(), store).await;
        Instance { state, listener }
    }

    /// Opens a stream to a second instance for an enrolled identity.
    pub async fn connect_to(&self, instance: &Instance, identity: EnrolledIdentity) -> FakeAgent {
        let capabilities = restichub_proto::AgentCapability::ALL
            .iter()
            .map(|capability| capability.as_str().to_owned())
            .collect();
        open_stream_on(
            &instance.listener,
            &instance.state,
            identity,
            capabilities,
            default_restore_roots(),
            None,
        )
        .await
    }

    /// The capabilities that the registry holds for the agent.
    pub async fn stored_capabilities(&self, agent: AgentId) -> Vec<String> {
        let mut tx = self.scoped().await;
        sqlx::query_scalar("SELECT capabilities FROM agents WHERE id = $1")
            .bind(agent)
            .fetch_one(&mut *tx)
            .await
            .expect("cannot read the capabilities")
    }

    /// Creates a local repository whose maintenance agent is `agent` and returns its identifier.
    pub async fn create_repository(&self, agent: AgentId, password: &str) -> Uuid {
        self.create_repository_at(agent, password, "/srv/backup/restic")
            .await
    }

    pub async fn create_repository_at(&self, agent: AgentId, password: &str, path: &str) -> Uuid {
        let body = json!({
            "name": format!("repo-{}", Uuid::now_v7()),
            "backend": {"kind": "local", "path": path},
            "password": password,
            "maintenance_agent_id": agent.to_string(),
        });
        let (status, _, repository) = send(
            &self.app,
            bearer_request("POST", "/api/v1/repositories", &self.admin, Some(body)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{repository}");
        id_of(&repository)
    }

    /// The fields of `extra` replace or add to the request body.
    pub async fn create_profile(&self, repository: Uuid, extra: Value) -> Uuid {
        let mut body = json!({
            "name": format!("profile-{}", Uuid::now_v7()),
            "repository_id": repository.to_string(),
            "paths": {"all": ["/srv"]},
        });
        if let (Some(target), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                target.insert(key.clone(), value.clone());
            }
        }
        let (status, _, response) = send(
            &self.app,
            bearer_request("POST", "/api/v1/profiles", &self.admin, Some(body)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{response}");
        id_of(&response["profile"])
    }

    /// Creates a plan that no scheduler pass fires during the test (a far cron).
    pub async fn create_plan(&self, profile: Uuid, agent: AgentId) -> Uuid {
        let body = json!({
            "name": format!("plan-{}", Uuid::now_v7()),
            "profile_id": profile.to_string(),
            "agent_id": agent.to_string(),
            "cron": "0 2 1 1 *",
            "time_zone": "UTC",
            "missed_run_policy": "skip",
        });
        let (status, _, response) = send(
            &self.app,
            bearer_request("POST", "/api/v1/plans", &self.admin, Some(body)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{response}");
        id_of(&response["plan"])
    }

    /// Runs a new plan on its own profile. A plan holds one queued manual run at most, so a test that
    /// needs many queued backups of one repository uses one plan for each.
    pub async fn run_fresh_plan(&self, repository: Uuid, agent: AgentId) -> Uuid {
        let profile = self.create_profile(repository, json!({})).await;
        let plan = self.create_plan(profile, agent).await;
        self.run_plan(plan).await
    }

    /// Posts the manual run as an Operator and returns the new job.
    pub async fn run_plan(&self, plan: Uuid) -> Uuid {
        let (status, _, job) = send(
            &self.app,
            bearer_request(
                "POST",
                &format!("/api/v1/plans/{plan}/run"),
                &self.operator,
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{job}");
        id_of(&job)
    }

    /// Writes a queued job of any queue kind, as the maintenance of a later epic does.
    pub async fn enqueue(
        &self,
        kind: restichub_db::JobKind,
        agent: AgentId,
        repository: Uuid,
        plan: Option<Uuid>,
    ) -> Uuid {
        let id = restichub_db::JobId::new();
        let mut tx = self
            .db
            .database()
            .begin_tenant(self.tenant)
            .await
            .expect("cannot begin the transaction");
        restichub_db::jobs::create(
            &mut tx,
            &restichub_db::jobs::NewJob {
                id,
                kind,
                agent_id: agent,
                repository_id: Some(restichub_db::RepositoryId::from_uuid(repository)),
                plan_id: plan.map(restichub_db::PlanId::from_uuid),
                fire_time: None,
                expires_at: Some(self.now() + time::Duration::hours(1)),
                location_fingerprint: None,
                created_by: None,
            },
        )
        .await
        .expect("cannot create the job");
        tx.commit().await.expect("cannot commit the job");
        id.as_uuid()
    }

    pub async fn plan_jobs(&self, plan: Uuid) -> Vec<Value> {
        let (status, body) = self
            .call(
                &self.admin,
                "GET",
                &format!("/api/v1/jobs?plan_id={plan}"),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["items"].as_array().cloned().unwrap_or_default()
    }

    pub async fn job(&self, job: Uuid) -> Value {
        let (status, _, body) = send(
            &self.app,
            bearer_request("GET", &format!("/api/v1/jobs/{job}"), &self.admin, None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    pub async fn job_status(&self, job: Uuid) -> String {
        self.job(job).await["status"]
            .as_str()
            .expect("the job status")
            .to_owned()
    }

    /// Sends an API call with a bearer token and returns the status and the body.
    pub async fn call(
        &self,
        token: &str,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let (status, _, body) = send(&self.app, bearer_request(method, uri, token, body)).await;
        (status, body)
    }

    /// Reads the signed job key of the installation, to verify a delivered job.
    pub async fn verify(&self, signed: &SignedJob, agent: &FakeAgent) -> Job {
        let mut tx = self
            .db
            .database()
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
        let mut ring = KeyRing::new();
        ring.insert(
            row.id.to_string(),
            ed25519_dalek::VerifyingKey::from_bytes(&bytes).expect("a valid key"),
        )
        .expect("the ring must accept the key");
        verify_job(
            signed,
            &ring,
            &Expected {
                agent_id: agent.identity.agent_id.clone(),
                tenant_id: self.tenant.to_string(),
                now: prost_types::Timestamp {
                    seconds: self.now().unix_timestamp(),
                    nanos: 0,
                },
            },
        )
        .expect("the agent must verify the job")
    }
}

/// A second control plane process on the same database; see [`Env::start_instance`].
pub struct Instance {
    pub state: AppState,
    pub listener: Listener,
}

async fn raw_stream(
    listener: &Listener,
    identity: &EnrolledIdentity,
) -> Result<(mpsc::Sender<AgentMessage>, Streaming<ControlMessage>), tonic::Status> {
    let client_tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(der_to_pem(
            "CERTIFICATE",
            &listener.root_der,
        )))
        .identity(Identity::from_pem(
            client_chain_pem(identity),
            identity.key_pem.clone(),
        ))
        .domain_name("localhost");
    let channel = Endpoint::from_shared(listener.https_url())
        .expect("the endpoint URL")
        .tls_config(client_tls)
        .expect("the client TLS config")
        .connect()
        .await
        .map_err(|error| tonic::Status::unavailable(error.to_string()))?;
    let mut client = AgentServiceClient::new(channel);
    let (tx, rx) = mpsc::channel::<AgentMessage>(64);
    let response = client.connect(ReceiverStream::new(rx)).await?;
    Ok((tx, response.into_inner()))
}

/// Opens a stream to `listener`, sends the hello and drains the greeting.
async fn open_stream_on(
    listener: &Listener,
    state: &AppState,
    identity: EnrolledIdentity,
    capabilities: Vec<String>,
    roots: Vec<String>,
    report: Option<HookReport>,
) -> FakeAgent {
    let (tx, mut inbound) = raw_stream(listener, &identity)
        .await
        .expect("the stream must open");
    tx.send(hello(capabilities, roots, report))
        .await
        .expect("cannot send the hello");
    let (mut hello_seen, mut update_seen) = (false, false);
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
    assert!(hello_seen && update_seen, "the stream hello must arrive");
    let agent_id = AgentId::from_uuid(Uuid::parse_str(&identity.agent_id).expect("agent id"));
    // The dispatcher sees the stream only after the hello fixed its capabilities.
    eventually_online(state, agent_id).await;
    FakeAgent {
        id: agent_id,
        identity,
        tx,
        inbound,
    }
}

pub fn id_of(value: &Value) -> Uuid {
    Uuid::parse_str(value["id"].as_str().expect("the object id")).expect("the id is a UUID")
}

/// The temporary directory, where the tests restore (FR-RS-03).
pub fn default_restore_roots() -> Vec<String> {
    vec![std::env::temp_dir().to_string_lossy().into_owned()]
}

/// The hook allow-list that a fake agent reports in its hello (FR-HK-04).
#[derive(Clone)]
pub struct HookReport {
    pub state: restichub_proto::agent::v1::HookAllowListState,
    pub hooks: Vec<restichub_proto::agent::v1::HookMetadata>,
    /// Trust mode and interpreters (FR-HK-08).
    pub scripts: Option<(
        restichub_proto::agent::v1::ServerScriptsState,
        Vec<restichub_proto::agent::v1::ScriptInterpreter>,
    )>,
}

/// The report of a host without a hook allow-list and with the trust mode on or off (FR-HK-08).
pub fn scripts_report(
    enabled: bool,
    interpreters: Vec<restichub_proto::agent::v1::ScriptInterpreter>,
) -> HookReport {
    use restichub_proto::agent::v1::{HookAllowListState, ServerScriptsState};
    HookReport {
        state: HookAllowListState::Absent,
        hooks: Vec::new(),
        scripts: Some((
            if enabled {
                ServerScriptsState::Enabled
            } else {
                ServerScriptsState::Disabled
            },
            interpreters,
        )),
    }
}

fn hello(
    capabilities: Vec<String>,
    restore_roots: Vec<String>,
    report: Option<HookReport>,
) -> AgentMessage {
    let (hook_allow_list_state, hooks, scripts) = match report {
        Some(report) => (report.state as i32, report.hooks, report.scripts),
        None => (0, Vec::new(), None),
    };
    let (server_scripts_state, script_interpreters) = scripts.map_or((0, Vec::new()), |scripts| {
        (
            scripts.0 as i32,
            scripts.1.into_iter().map(|kind| kind as i32).collect(),
        )
    });
    AgentMessage {
        payload: Some(agent_message::Payload::AgentHello(AgentHello {
            protocol_version_major: restichub_proto::PROTOCOL_VERSION_MAJOR,
            protocol_version_minor: restichub_proto::PROTOCOL_VERSION_MINOR,
            agent_version: "0.1.0".to_owned(),
            capabilities,
            restore_roots,
            hooks,
            hook_allow_list_state,
            server_scripts_state,
            script_interpreters,
            ..AgentHello::default()
        })),
    }
}

pub async fn next_message(inbound: &mut Streaming<ControlMessage>) -> ControlMessage {
    tokio::time::timeout(Duration::from_secs(10), inbound.message())
        .await
        .expect("the message must arrive")
        .expect("the stream must not fail")
        .expect("the stream must not end")
}

/// An enrolled agent on an open stream. The test plays the agent side by hand.
pub struct FakeAgent {
    pub id: AgentId,
    pub identity: EnrolledIdentity,
    pub tx: mpsc::Sender<AgentMessage>,
    pub inbound: Streaming<ControlMessage>,
}

impl FakeAgent {
    /// The next signed job within `wait`, or `None`. A heartbeat is skipped.
    pub async fn try_job(&mut self, wait: Duration) -> Option<SignedJob> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let message = tokio::time::timeout(left, self.inbound.message())
                .await
                .ok()?
                .ok()??;
            if let Some(control_message::Payload::SignedJob(signed)) = message.payload {
                return Some(signed);
            }
        }
    }

    /// Fails the test when no signed job arrives within ten seconds.
    pub async fn job(&mut self) -> SignedJob {
        self.try_job(Duration::from_secs(10))
            .await
            .expect("the signed job must arrive")
    }

    /// The identifier of the next cancel message within `wait`, or `None`. A heartbeat is skipped.
    pub async fn try_cancel(&mut self, wait: Duration) -> Option<String> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let message = tokio::time::timeout(left, self.inbound.message())
                .await
                .ok()?
                .ok()??;
            if let Some(control_message::Payload::CancelJob(cancel)) = message.payload {
                return Some(cancel.job_id);
            }
        }
    }

    /// Collects every signed job that arrives before the deadline, in order.
    pub async fn jobs_within(&mut self, wait: Duration) -> Vec<String> {
        let mut ids = Vec::new();
        let deadline = tokio::time::Instant::now() + wait;
        while let Some(signed) = self
            .try_job(deadline.saturating_duration_since(tokio::time::Instant::now()))
            .await
        {
            ids.push(decode_job_id(&signed));
        }
        ids
    }

    pub async fn ack(&self, job_id: &str) {
        self.tx
            .send(AgentMessage {
                payload: Some(agent_message::Payload::JobAck(JobAck {
                    job_id: job_id.to_owned(),
                })),
            })
            .await
            .expect("cannot send the acknowledgement");
    }

    /// Acknowledges a job and waits until the control plane shows it as running.
    pub async fn start(&self, env: &Env, job_id: &str) {
        self.ack(job_id).await;
        let job = Uuid::parse_str(job_id).expect("job id");
        eventually(|| async { env.job_status(job).await == "running" }).await;
    }

    /// Reports a success and waits until the control plane shows the job as succeeded.
    pub async fn finish(&self, env: &Env, job_id: &str) {
        self.result(job_id, JobStatus::Succeeded, StepStatus::Success)
            .await;
        let job = Uuid::parse_str(job_id).expect("job id");
        eventually(|| async { env.job_status(job).await == "succeeded" }).await;
    }

    /// Sends the result of a successful retention step. With `selected` it holds one removed
    /// snapshot, without it no group.
    pub async fn forget_step(&self, job_id: &str, selected: bool) {
        let groups = if selected {
            vec![ForgetGroup {
                removed: vec![ForgetSnapshot::default()],
                ..ForgetGroup::default()
            }]
        } else {
            Vec::new()
        };
        self.tx
            .send(AgentMessage {
                payload: Some(agent_message::Payload::StepResult(StepResult {
                    job_id: job_id.to_owned(),
                    step_id: "forget-1".to_owned(),
                    status: StepStatus::Success as i32,
                    output: Some(step_result::Output::ForgetResult(ForgetResult {
                        groups,
                        truncated: false,
                    })),
                    ..StepResult::default()
                })),
            })
            .await
            .expect("cannot send the step result");
    }

    /// Reports a hard-killed step that can leave a stale lock (FR-AG-10).
    pub async fn stale_lock_step(&self, job_id: &str) {
        self.tx
            .send(AgentMessage {
                payload: Some(agent_message::Payload::StepResult(StepResult {
                    job_id: job_id.to_owned(),
                    step_id: "backup".to_owned(),
                    status: StepStatus::Failed as i32,
                    exit_code: -1,
                    possible_stale_lock: true,
                    ..StepResult::default()
                })),
            })
            .await
            .expect("cannot send the step result");
    }

    /// Reports a held repository lock as the real agent does: the step has the retry status and exit
    /// code 11, and the stopped job has the status `failed` with the outcome `failed`.
    pub async fn lock_held(&self, job_id: &str) {
        self.tx
            .send(AgentMessage {
                payload: Some(agent_message::Payload::StepResult(StepResult {
                    job_id: job_id.to_owned(),
                    step_id: "backup".to_owned(),
                    status: StepStatus::Retry as i32,
                    exit_code: 11,
                    ..StepResult::default()
                })),
            })
            .await
            .expect("cannot send the step result");
        self.tx
            .send(AgentMessage {
                payload: Some(agent_message::Payload::JobResult(JobResult {
                    job_id: job_id.to_owned(),
                    status: StepStatus::Failed as i32,
                    steps_total: 1,
                    steps_succeeded: 0,
                    steps_failed: 1,
                    outcome: JobStatus::Failed as i32,
                    error: Some(restichub_proto::agent::v1::ErrorDetail {
                        code: "step_failed".to_owned(),
                        params: Vec::new(),
                    }),
                    ..JobResult::default()
                })),
            })
            .await
            .expect("cannot send the job result");
    }

    pub async fn result(&self, job_id: &str, outcome: JobStatus, status: StepStatus) {
        self.tx
            .send(AgentMessage {
                payload: Some(agent_message::Payload::JobResult(JobResult {
                    job_id: job_id.to_owned(),
                    status: status as i32,
                    steps_total: 1,
                    steps_succeeded: u32::from(outcome == JobStatus::Succeeded),
                    steps_failed: u32::from(outcome != JobStatus::Succeeded),
                    outcome: outcome as i32,
                    error: None,
                    ..JobResult::default()
                })),
            })
            .await
            .expect("cannot send the job result");
    }
}

pub fn decode_job_id(signed: &SignedJob) -> String {
    use prost::Message as _;
    Job::decode(signed.job.as_slice())
        .expect("the signed bytes hold a job")
        .job_id
}

/// Waits until `check` holds, polling the API. The test fails after ten seconds.
pub async fn eventually<F, Fut>(mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the condition did not hold within ten seconds");
}

/// An emulated agent that runs the jobs with the given executor.
pub struct Emulator {
    pub id: AgentId,
    pub config: Arc<restichub_emulator::agent::AgentConfig>,
    shutdown: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Emulator {
    pub async fn stop(self) {
        let _ = self.shutdown.send(true);
        let _ = tokio::time::timeout(Duration::from_secs(2), self.task).await;
    }
}

impl Env {
    /// The signing keys that the enrollment gave the agent, as the agent holds them.
    pub fn key_ring(identity: &EnrolledIdentity) -> KeyRing {
        use base64::Engine as _;

        let mut ring = KeyRing::new();
        for key in &identity.signing_keys {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&key.public_key)
                .expect("the public key is base64");
            let bytes: [u8; 32] = bytes.as_slice().try_into().expect("a 32-byte key");
            ring.insert(
                key.key_id.clone(),
                ed25519_dalek::VerifyingKey::from_bytes(&bytes).expect("a valid key"),
            )
            .expect("the ring must accept the enrollment key");
        }
        ring
    }

    /// Enrolls an agent and runs the emulator with the executor against the listener.
    pub async fn start_emulator(
        &self,
        hostname: &str,
        executor: Arc<dyn restichub_emulator::executor::Executor>,
    ) -> Emulator {
        let identity = self.enroll_agent(hostname).await;
        self.start_emulator_for(identity, executor).await
    }

    /// Runs the emulator with an identity that the test enrolled before.
    pub async fn start_emulator_for(
        &self,
        identity: EnrolledIdentity,
        executor: Arc<dyn restichub_emulator::executor::Executor>,
    ) -> Emulator {
        use base64::Engine as _;
        use restichub_emulator::agent::{AgentConfig, AgentLoad, Backoff, run_agent};
        use restichub_emulator::identity::AgentIdentity;
        use restichub_emulator::metrics::Metrics;

        let mut ring = KeyRing::new();
        for key in &identity.signing_keys {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&key.public_key)
                .expect("the public key is base64");
            let bytes: [u8; 32] = bytes.as_slice().try_into().expect("a 32-byte key");
            ring.insert(
                key.key_id.clone(),
                ed25519_dalek::VerifyingKey::from_bytes(&bytes).expect("a valid key"),
            )
            .expect("the ring must accept the enrollment key");
        }
        let id = AgentId::from_uuid(Uuid::parse_str(&identity.agent_id).expect("agent id"));
        let config = Arc::new(AgentConfig {
            tenant_id: identity.tenant_id.clone(),
            server_url: self.listener.https_url(),
            server_name: "localhost".to_owned(),
            ca_pem: der_to_pem("CERTIFICATE", &self.listener.root_der),
            identity: AgentIdentity {
                agent_id: identity.agent_id.clone(),
                cert_pem: client_chain_pem(&identity),
                key_pem: identity.key_pem.clone(),
            },
            key_ring: ring,
            executor,
            backoff: Backoff {
                base: Duration::from_millis(50),
                max: Duration::from_millis(200),
            },
            load: {
                let mut load = AgentLoad::new(Arc::new(Metrics::default()));
                load.restore_roots = default_restore_roots();
                load
            },
        });
        let (shutdown, rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(run_agent(Arc::clone(&config), rx));
        eventually_online(&self.state, id).await;
        Emulator {
            id,
            config,
            shutdown,
            task,
        }
    }
}

pub async fn eventually_online(state: &AppState, agent: AgentId) {
    eventually(|| async { state.agent_registry().is_online(agent) }).await;
}
