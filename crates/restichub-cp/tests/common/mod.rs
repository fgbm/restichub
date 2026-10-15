//! Shared helpers for the control plane integration tests.
//!
//! A test builds its authentication through a real session or a real API token in the database. The
//! extractor no longer accepts a context from the request extensions.

#![allow(dead_code)]
#![allow(clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use restichub_cp::clock::Clock;
use restichub_cp::config::{AuthConfig, TenancyConfig};
use restichub_cp::server::{AppState, AuthState, router};
use restichub_db::test_util::{TestDatabase, test_database};
use restichub_db::{
    Database, PLATFORM_TENANT_ID, Role, TenantId, TenantStatus, api_tokens, memberships, sessions,
    tenants, users,
};
use serde_json::{Value, json};
use time::OffsetDateTime;
use tower::ServiceExt;

pub const COOKIE_NAME: &str = "__Host-restichub_session";
pub const CSRF: (&str, &str) = ("x-restichub-csrf", "1");

pub fn peer() -> ConnectInfo<SocketAddr> {
    ConnectInfo(
        "203.0.113.7:1234"
            .parse()
            .unwrap_or_else(|_| SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 0))),
    )
}

pub fn request(
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = cookie {
        builder = builder.header(header::COOKIE, value);
    }
    // An unsafe method needs the CSRF header when the request is cookie-authenticated.
    if matches!(method, "POST" | "PUT" | "PATCH" | "DELETE") {
        builder = builder.header(CSRF.0, CSRF.1);
    }
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let mut request = builder.body(body).expect("cannot build the request");
    request.extensions_mut().insert(peer());
    request
}

/// The function sends the body as it is, so a test can send a malformed JSON body or a body that is
/// larger than the limit.
pub fn raw_request(method: &str, uri: &str, cookie: &str, body: String) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, cookie)
        .header(CSRF.0, CSRF.1)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("cannot build the request");
    request.extensions_mut().insert(peer());
    request
}

pub fn bearer_request(method: &str, uri: &str, token: &str, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let mut request = builder.body(body).expect("cannot build the request");
    request.extensions_mut().insert(peer());
    request
}

/// Builds a body that passes the JSON extraction for an operation (AC-AU-02, AC-TN-01).
///
/// A malformed body would stop a request before the handler runs the authorization and the
/// isolation checks, so the two tests send a valid body. A Platform Admin acts in the platform
/// tenant, where the only valid role is `platform_admin`.
#[allow(clippy::too_many_arguments)]
pub fn write_body(
    method: &str,
    path: &str,
    index: usize,
    platform_admin: bool,
    tenant_id: &str,
    repository_id: &str,
    profile_id: &str,
    agent_id: &str,
) -> Option<Value> {
    let role = if platform_admin {
        "platform_admin"
    } else {
        "viewer"
    };
    match (method, path) {
        ("GET", _) | ("DELETE", _) => None,
        ("POST", "/api/v1/users") => Some(json!({
            "username": format!("matrix-{index}"),
            "password": "a long enough password",
            "display_name": "Matrix",
            "role": role,
        })),
        ("POST", "/api/v1/users/{id}/password") => {
            Some(json!({"new_password": "a long enough password"}))
        }
        ("POST", "/api/v1/api-tokens") => Some(json!({"name": "matrix", "role": role})),
        ("POST", "/api/v1/enrollment-tokens") => {
            Some(json!({"labels": ["matrix"], "ttl_seconds": 900}))
        }
        ("PATCH", "/api/v1/agents/{id}") => Some(json!({"hostname": "matrix-host"})),
        ("POST", "/api/v1/secrets") => Some(json!({
            "name": format!("matrix-secret-{index}"),
            "purpose": "repository_password",
            "value": "matrix-value",
        })),
        ("POST", "/api/v1/repositories") => Some(json!({
            "name": format!("matrix-repo-{index}"),
            "backend": {"kind": "s3", "endpoint": "https://s3.example.net", "bucket": "matrix-bucket"},
            "password": "matrix-value",
            "s3_access_key": "matrix-access",
            "s3_secret_key": "matrix-secret",
        })),
        ("PATCH", "/api/v1/repositories/{id}") => Some(json!({"name": "matrix-repo"})),
        ("POST", "/api/v1/repositories/{id}/unlock-all") => Some(json!({"confirm": "matrix-repo"})),
        ("PUT", "/api/v1/repositories/{id}/maintenance") => Some(json!({
            "time_zone": "UTC",
            "retention": {"cron": "0 3 * * 0", "enabled": true},
            "check": {"cron": "0 4 1 * *", "enabled": true},
        })),
        ("POST", "/api/v1/repositories/{id}/maintenance/run") => Some(json!({"task": "check"})),
        ("POST", "/api/v1/repositories/{id}/snapshots") => Some(json!({})),
        ("POST", "/api/v1/repositories/{id}/ls") => Some(json!({"snapshot_id": "0123abcd"})),
        ("POST", "/api/v1/repositories/{id}/restore") => Some(json!({
            "snapshot_id": "0123abcd",
            "target": "/srv/restore",
            "overwrite": "never"
        })),
        ("POST", "/api/v1/profiles") => Some(json!({
            "name": format!("matrix-profile-{index}"),
            "repository_id": repository_id,
            "paths": {"all": ["/srv"]},
        })),
        ("PATCH", "/api/v1/profiles/{id}") => Some(json!({
            "repository_id": repository_id,
            "paths": {"all": ["/srv"]},
        })),
        ("POST", "/api/v1/profiles/{id}/preview") => Some(json!({"agent_id": repository_id})),
        ("POST", "/api/v1/plans") => Some(json!({
            "name": format!("matrix-plan-{index}"),
            "profile_id": profile_id,
            "agent_id": agent_id,
            "cron": "0 2 * * *",
            "time_zone": "UTC",
            "missed_run_policy": "run_all",
        })),
        ("PATCH", "/api/v1/plans/{id}") => Some(json!({
            "name": format!("matrix-plan-{index}"),
            "profile_id": profile_id,
            "agent_id": agent_id,
            "cron": "0 2 * * *",
            "time_zone": "UTC",
            "missed_run_policy": "run_all",
        })),
        ("POST", "/api/v1/plans/preview") => Some(json!({
            "cron": "0 2 * * *",
            "time_zone": "UTC",
            "count": 3,
        })),
        ("POST", "/api/v1/notification-channels") => Some(json!({
            "name": format!("matrix-channel-{index}"),
            "kind": "email",
            "recipients": ["matrix@example.com"],
        })),
        ("PATCH", "/api/v1/notification-channels/{id}") => Some(json!({"enabled": true})),
        ("PUT", "/api/v1/alert-rules") => Some(json!({"items": []})),
        ("PUT", _) => Some(json!({"value": "matrix-value"})),
        // A key that is never current: the matrix checks the right and compromises nothing.
        ("POST", "/api/v1/platform/signing-keys/compromise") => Some(json!({
            "expected_current_key_id": "00000000-0000-0000-0000-000000000000",
        })),
        ("POST", "/api/v1/platform/tenants") => Some(json!({
            "name": format!("matrix-{index}"),
        })),
        ("POST", "/api/v1/platform/users") => Some(json!({
            "username": format!("matrix-{index}"),
            "display_name": "Matrix",
            "password": "a long enough password",
        })),
        ("POST", "/api/v1/platform/users/{id}/memberships") => {
            Some(json!({"tenant_id": tenant_id, "role": "viewer"}))
        }
        ("PATCH", "/api/v1/platform/users/{id}/memberships/{tenant_id}") => {
            Some(json!({"role": "viewer"}))
        }
        ("PATCH", "/api/v1/platform/users/{id}") => Some(json!({"disabled": true})),
        ("POST", "/api/v1/auth/password") => Some(json!({
            "current_password": "wrong",
            "new_password": "a long enough password",
        })),
        ("POST", "/api/v1/auth/switch") => Some(json!({"tenant_id": tenant_id})),
        ("POST", "/api/v1/auth/sign-out") => None,
        ("PATCH", _) => Some(json!({"display_name": "Matrix"})),
        _ => Some(json!({})),
    }
}

pub async fn body_json(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("cannot read the body")
        .to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

pub async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Option<String>, Value) {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("the request failed");
    let status = response.status();
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(cookie_pair);
    let body = body_json(response).await;
    (status, cookie, body)
}

fn cookie_pair(header: &str) -> Option<String> {
    header.split(';').next().map(str::to_owned)
}

/// The existing tests cover the multi-tenant behaviour, so the helper sets multi mode.
pub fn app(db: &Database) -> Router {
    router(AppState::new(db.clone()).with_tenancy(TenancyConfig::multi()))
}

pub fn app_with_auth(db: &Database, config: AuthConfig, clock: Arc<dyn Clock>) -> Router {
    let auth = AuthState::new(config).with_clock(clock);
    router(AppState::with_auth(db.clone(), auth).with_tenancy(TenancyConfig::multi()))
}

/// The same as [`app`], in single mode with the default tenant name.
pub fn app_single(db: &Database) -> Router {
    router(AppState::new(db.clone()).with_tenancy(TenancyConfig::default()))
}

/// The same as [`app`], with a small per-tenant account create bucket, so a test reaches the limit
/// quickly.
pub fn app_with_tenant_create_rate(db: &Database, capacity: u32) -> Router {
    let auth = AuthState::new(AuthConfig::default()).with_create_user_tenant_rate(capacity);
    router(AppState::with_auth(db.clone(), auth).with_tenancy(TenancyConfig::multi()))
}

#[derive(Debug)]
pub struct TestKek(restichub_secrets::Kek);

impl restichub_secrets::KekProvider for TestKek {
    fn current(&self) -> &restichub_secrets::Kek {
        &self.0
    }

    fn by_id(&self, id: &restichub_secrets::KekId) -> Option<&restichub_secrets::Kek> {
        (self.0.id() == id).then_some(&self.0)
    }
}

pub fn test_kek() -> Arc<dyn restichub_secrets::KekProvider> {
    let mut bytes = [7_u8; 32];
    Arc::new(TestKek(restichub_secrets::Kek::from_bytes(&mut bytes)))
}

pub fn app_with_kek(db: &Database) -> Router {
    router(
        AppState::new(db.clone())
            .with_kek(test_kek())
            .with_tenancy(TenancyConfig::multi()),
    )
}

/// The gRPC endpoint that the enrollment tests report to an agent.
pub const AGENT_ENDPOINT: &str = "https://agents.example:8443";

/// Builds a router with a real certificate authority and job signing key set, so the enrollment
/// endpoint can issue a certificate and return the signed key set.
pub async fn app_with_pki(db: &Database) -> Router {
    router(pki_state(db).await.0)
}

/// Builds the application state with a real certificate authority and job signing key set.
///
/// The agent listener tests use the state with [`agent_router`]; the enrollment tests use it with
/// both the web router and the agent router.
pub async fn pki_state(db: &Database) -> (AppState, Arc<restichub_cp::pki::CaStore>) {
    let provider = test_kek();
    let ca = restichub_cp::pki::CaService::new(provider.clone());
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
        .with_pki(store.clone(), job_signing, Some(AGENT_ENDPOINT.to_owned()));
    (state, store)
}

/// Builds the listener routes for a test that drives them without TLS.
pub fn agent_router(state: AppState) -> Router {
    restichub_cp::agent_listener::agent_router(state)
}

/// Public web router of a state.
pub fn api_router(state: AppState) -> Router {
    router(state)
}

/// Builds a router with a certificate authority but no job signing key set.
///
/// The enrollment endpoint builds the signed key set before it consumes the token, so this router
/// refuses an enrollment without spending the token.
pub async fn app_with_ca_without_signing_keys(db: &Database) -> Router {
    router(ca_without_signing_state(db).await)
}

/// Builds the state with a certificate authority but no job signing key set.
pub async fn ca_without_signing_state(db: &Database) -> AppState {
    let provider = test_kek();
    let ca = restichub_cp::pki::CaService::new(provider.clone());
    let store = ca
        .ensure_initialized(db)
        .await
        .expect("cannot create the CA");
    let job_signing = restichub_cp::job_signing::JobSigningService::new(provider.clone());
    let auth = AuthState::new(AuthConfig::default());
    AppState::with_auth(db.clone(), auth)
        .with_kek(provider)
        .with_tenancy(TenancyConfig::multi())
        .with_pki(
            Arc::new(store),
            job_signing,
            Some(AGENT_ENDPOINT.to_owned()),
        )
}

/// Creates an enrollment token with a known value, so a test can send it to the endpoint.
pub async fn seed_enrollment_token_value(
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
    let id = restichub_db::enrollment_tokens::create(
        &mut tx,
        &restichub_db::enrollment_tokens::NewEnrollmentToken {
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

/// The single mode with a key encryption key provider. A test that runs the single-mode setup
/// needs the provider to wrap the data encryption key of the tenant `default`.
pub fn app_single_with_kek(db: &Database) -> Router {
    router(
        AppState::new(db.clone())
            .with_kek(test_kek())
            .with_tenancy(TenancyConfig::default()),
    )
}

pub async fn create_tenant(db: &Database, name: &str) -> TenantId {
    let mut tx = db
        .begin_platform()
        .await
        .expect("cannot begin the transaction");
    let tenant = tenants::create(&mut tx, name)
        .await
        .expect("cannot create the tenant");
    tx.commit().await.expect("cannot commit the tenant");
    tenant
}

/// Creates a tenant with a wrapped data encryption key.
///
/// The helper uses the fixed test key encryption key (KEK). A test that creates a secret needs the
/// tenant to hold an active key row.
pub async fn create_tenant_with_key(db: &Database, name: &str) -> TenantId {
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
    tx.commit()
        .await
        .expect("cannot commit the tenant with a key");
    id
}

/// Inserts a secret row of a tenant and returns its identifier.
///
/// The ciphertext is a placeholder. A test uses this helper only for a route fixture, not for a
/// decryption.
pub async fn seed_secret(db: &Database, tenant: TenantId, name: &str) -> restichub_db::SecretId {
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let id = restichub_db::secrets::create(
        &mut tx,
        &restichub_db::secrets::NewSecret {
            id: restichub_db::SecretId::new(),
            name,
            purpose: "repository_password",
            key_version: 1,
            nonce: &[0_u8; 12],
            ciphertext: &[0_u8; 16],
            created_by: None,
        },
    )
    .await
    .expect("cannot create the secret");
    tx.commit().await.expect("cannot commit the secret");
    id
}

/// Inserts a local repository row of a tenant and returns its identifier.
///
/// A test uses it only for a route fixture. The bound secret must already exist in the tenant.
pub async fn seed_repository(
    db: &Database,
    tenant: TenantId,
    name: &str,
    password_secret: restichub_db::SecretId,
) -> restichub_db::RepositoryId {
    use restichub_db::repositories::{NewRepository, RepositorySecrets, RepositorySettings};
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let id = restichub_db::RepositoryId::new();
    restichub_db::repositories::create(
        &mut tx,
        &NewRepository {
            id,
            name,
            backend: restichub_db::BackendKind::Local,
            settings: RepositorySettings {
                local_path: Some("/mnt/backup/restic"),
                ..RepositorySettings::default()
            },
            secrets: RepositorySecrets {
                password: password_secret,
                s3_access_key: None,
                s3_secret_key: None,
                rest_password: None,
            },
            maintenance_agent_id: None,
            created_by: None,
        },
    )
    .await
    .expect("cannot create the repository");
    tx.commit().await.expect("cannot commit the repository");
    id
}

/// Inserts a profile row of a tenant and returns its identifier.
///
/// A test uses it only for a route fixture. The repository must already exist in the tenant.
pub async fn seed_profile(
    db: &Database,
    tenant: TenantId,
    name: &str,
    repository: restichub_db::RepositoryId,
) -> restichub_db::ProfileId {
    use restichub_db::profiles::{NewProfile, create};
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let id = restichub_db::ProfileId::new();
    create(
        &mut tx,
        &NewProfile {
            id,
            name,
            repository_id: repository,
            paths_all: &["/srv".to_owned()],
            paths_windows: &[],
            paths_linux: &[],
            paths_macos: &[],
            excludes: &[],
            exclude_files: &[],
            tags: &[],
            use_fs_snapshot: false,
            one_file_system: false,
            exclude_caches: false,
            limit_upload_kib: 0,
            limit_download_kib: 0,
            retention: None,
            check_options: None,
            hooks: serde_json::json!([]),
            created_by: None,
        },
    )
    .await
    .expect("cannot create the profile");
    tx.commit().await.expect("cannot commit the profile");
    id
}

/// Inserts an active agent row of a tenant and returns its identifier.
///
/// The enrollment exchange (#48) creates the row in production; a test seeds it directly.
pub async fn seed_agent(db: &Database, tenant: TenantId, hostname: &str) -> restichub_db::AgentId {
    let mut serial = vec![0_u8; 16];
    getrandom::fill(&mut serial).expect("cannot read random bytes");
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let id = restichub_db::agents::create(
        &mut tx,
        &restichub_db::agents::NewAgent {
            id: restichub_db::AgentId::new(),
            hostname,
            os: "linux",
            arch: "amd64",
            agent_version: "0.1.0",
            restic_version: "0.19.0",
            labels: &[],
            certificate_serial: &serial,
            certificate_expires_at: OffsetDateTime::now_utc() + time::Duration::days(30),
            last_seen_at: None,
        },
    )
    .await
    .expect("cannot create the agent");
    tx.commit().await.expect("cannot commit the agent");
    id
}

/// Inserts a job row of a tenant and returns its identifier.
///
/// The control plane creates the row in production; a test seeds it directly for a route fixture.
pub async fn seed_job(
    db: &Database,
    tenant: TenantId,
    agent: restichub_db::AgentId,
    repository: restichub_db::RepositoryId,
) -> restichub_db::JobId {
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let id = restichub_db::JobId::new();
    restichub_db::jobs::create(
        &mut tx,
        &restichub_db::jobs::NewJob {
            id,
            kind: restichub_db::JobKind::TestAccess,
            agent_id: agent,
            repository_id: Some(repository),
            plan_id: None,
            fire_time: None,
            expires_at: None,
            location_fingerprint: None,
            created_by: None,
        },
    )
    .await
    .expect("cannot create the job");
    tx.commit().await.expect("cannot commit the job");
    id
}

/// Inserts a queued scheduled job of one plan and returns its identifier (FR-SC-02).
///
/// The scheduler creates the row in production; a test seeds it directly, so a route test can name
/// the job of a plan without a scheduler pass.
pub async fn seed_scheduled_job(
    db: &Database,
    tenant: TenantId,
    plan: restichub_db::PlanId,
    agent: restichub_db::AgentId,
    repository: restichub_db::RepositoryId,
    fire_time: OffsetDateTime,
) -> restichub_db::JobId {
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let id = restichub_db::JobId::new();
    restichub_db::jobs::create_scheduled(
        &mut tx,
        &restichub_db::jobs::NewJob {
            id,
            kind: restichub_db::JobKind::Backup,
            agent_id: agent,
            repository_id: Some(repository),
            plan_id: Some(plan),
            fire_time: Some(fire_time),
            expires_at: None,
            location_fingerprint: None,
            created_by: None,
        },
    )
    .await
    .expect("cannot create the scheduled job")
    .expect("the scheduled job is created");
    tx.commit().await.expect("cannot commit the job");
    id
}

/// Inserts a plan row of a tenant and returns its identifier.
///
/// A test uses it only for a route fixture. The profile and the agent must already exist in the
/// tenant.
pub async fn seed_plan(
    db: &Database,
    tenant: TenantId,
    name: &str,
    profile: restichub_db::ProfileId,
    agent: restichub_db::AgentId,
) -> restichub_db::PlanId {
    seed_scheduled_plan(
        db,
        tenant,
        name,
        profile,
        agent,
        "0 2 * * *",
        "UTC",
        restichub_db::MissedRunPolicy::RunAll,
        None,
    )
    .await
}

/// Inserts a plan row with an explicit schedule and returns its identifier.
///
/// A scheduler test uses it to place the next fire time at a known instant.
#[allow(clippy::too_many_arguments)]
pub async fn seed_scheduled_plan(
    db: &Database,
    tenant: TenantId,
    name: &str,
    profile: restichub_db::ProfileId,
    agent: restichub_db::AgentId,
    cron: &str,
    time_zone: &str,
    missed_run_policy: restichub_db::MissedRunPolicy,
    next_run_at: Option<OffsetDateTime>,
) -> restichub_db::PlanId {
    use restichub_db::plans::{NewPlan, create};
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let id = restichub_db::PlanId::new();
    create(
        &mut tx,
        &NewPlan {
            id,
            name,
            profile_id: profile,
            agent_id: agent,
            cron,
            time_zone,
            missed_run_policy,
            enabled: true,
            rpo_seconds: None,
            next_run_at,
            created_by: None,
        },
    )
    .await
    .expect("cannot create the plan");
    tx.commit().await.expect("cannot commit the plan");
    id
}

/// Every row that names the given schedule, oldest fire time first.
pub async fn plan_jobs(
    db: &Database,
    tenant: TenantId,
    plan: restichub_db::PlanId,
) -> Vec<restichub_db::jobs::Job> {
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let jobs = restichub_db::jobs::list_for_plan(&mut tx, plan)
        .await
        .expect("cannot list the jobs");
    tx.commit().await.expect("cannot commit the list");
    jobs
}

/// The fire times of the jobs of one plan, in UTC, oldest first.
pub async fn plan_fire_times(
    db: &Database,
    tenant: TenantId,
    plan: restichub_db::PlanId,
) -> Vec<OffsetDateTime> {
    plan_jobs(db, tenant, plan)
        .await
        .into_iter()
        .filter_map(|job| job.fire_time)
        .collect()
}

/// Inserts an unused enrollment token row of a tenant and returns its identifier.
pub async fn seed_enrollment_token(
    db: &Database,
    tenant: TenantId,
    created_by: restichub_db::UserId,
) -> restichub_db::EnrollmentTokenId {
    let mut hash = vec![0_u8; 32];
    getrandom::fill(&mut hash).expect("cannot read random bytes");
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let id = restichub_db::enrollment_tokens::create(
        &mut tx,
        &restichub_db::enrollment_tokens::NewEnrollmentToken {
            token_hash: &hash,
            display_prefix: "ab12cd34",
            labels: &["host".to_owned()],
            expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(15),
            created_by,
        },
    )
    .await
    .expect("cannot create the enrollment token");
    tx.commit().await.expect("cannot commit the token");
    id
}

pub async fn set_tenant_status(db: &Database, tenant: TenantId, status: &str) {
    let status = TenantStatus::parse(status).expect("the status is known");
    let mut tx = db
        .begin_platform()
        .await
        .expect("cannot begin the transaction");
    tenants::set_status(&mut tx, tenant, status)
        .await
        .expect("cannot set the tenant status");
    tx.commit().await.expect("cannot commit the status");
}

/// Inserts an account row with no membership. The control plane never creates this state, and the
/// runtime role writes no account row, so the owner connection writes it.
pub async fn seed_memberless_account(
    db: &TestDatabase,
    username: &str,
    password_hash: &str,
) -> restichub_db::UserId {
    let id = restichub_db::UserId::new();
    let mut tx = db
        .owner_pool()
        .begin()
        .await
        .expect("cannot begin the transaction");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(PLATFORM_TENANT_ID.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query(
        "INSERT INTO users (id, tenant_id, username, password_hash, display_name) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(PLATFORM_TENANT_ID)
    .bind(username)
    .bind(password_hash)
    .bind(username)
    .execute(&mut *tx)
    .await
    .expect("cannot insert the account");
    tx.commit().await.expect("cannot commit the account");
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
    // The Platform Admin is an account right, not a membership (ADR-0029). The account holds the
    // right and no membership; the session tenant is the internal platform tenant.
    if role == Role::PlatformAdmin {
        return create_platform_admin(db, username, password_hash).await;
    }
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

/// Creates an account with the Platform Admin right and no membership (ADR-0029).
pub async fn create_platform_admin(
    db: &Database,
    username: &str,
    password_hash: &str,
) -> restichub_db::UserId {
    let id = restichub_db::UserId::new();
    let mut tx = db
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .expect("cannot begin the transaction");
    let created = users::create_platform_admin(
        &mut tx,
        id,
        &users::NewUser {
            username,
            password_hash,
            display_name: username,
        },
    )
    .await
    .expect("cannot create the platform admin");
    assert!(created, "the platform admin account must be created");
    tx.commit().await.expect("cannot commit the platform admin");
    id
}

pub async fn session_cookie(db: &Database, tenant: TenantId, user: restichub_db::UserId) -> String {
    let value = restichub_cp::token::random_token().expect("cannot build the token");
    let hash = restichub_cp::token::hash_token(&value);
    let now = OffsetDateTime::now_utc();
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    sessions::create(
        &mut tx,
        user,
        &hash,
        now + time::Duration::hours(12),
        now,
        None,
        None,
    )
    .await
    .expect("cannot create the session");
    tx.commit().await.expect("cannot commit the session");
    format!("{COOKIE_NAME}={value}")
}

pub async fn seed_session(db: &Database, tenant: TenantId, username: &str, role: &str) -> String {
    let user = create_user(db, tenant, username, "phc", role).await;
    session_cookie(db, tenant, user).await
}

pub async fn seed_api_token(db: &Database, tenant: TenantId, role: &str) -> String {
    let creator_name = format!("creator-{role}-{tenant}");
    let creator = create_user(db, tenant, &creator_name, "phc", "admin").await;
    seed_token(db, tenant, role, creator).await
}

/// Creates an Admin API token whose creator is not an Admin and returns its value.
///
/// The token acts with the Admin role, but the creator does not count as an Admin. A test uses this
/// method to keep a stable actor while it disables the two Admins of a tenant.
pub async fn seed_admin_token_actor(db: &Database, tenant: TenantId) -> String {
    let creator_name = format!("token-owner-{tenant}");
    let creator = create_user(db, tenant, &creator_name, "phc", "viewer").await;
    seed_token(db, tenant, "admin", creator).await
}

async fn seed_token(
    db: &Database,
    tenant: TenantId,
    role: &str,
    created_by: restichub_db::UserId,
) -> String {
    let (value, prefix) = restichub_cp::token::random_api_token().expect("cannot build the token");
    let hash = restichub_cp::token::hash_token(&value);
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    api_tokens::create(
        &mut tx,
        &api_tokens::NewApiToken {
            name: "test-token",
            token_hash: &hash,
            display_prefix: &prefix,
            role: Role::parse(role).expect("the role is known"),
            created_by,
            expires_at: None,
        },
    )
    .await
    .expect("cannot create the token");
    tx.commit().await.expect("cannot commit the token");
    value
}

pub async fn count(db: &Database, tenant: TenantId, table: &'static str) -> i64 {
    restichub_db::test_util::count_rows(db, tenant, table)
        .await
        .expect("cannot count the rows")
}

/// The identifier of an account by its username (ADR-0029).
pub async fn find_account_id(db: &Database, username: &str) -> restichub_db::UserId {
    users::find_account(db, username)
        .await
        .expect("cannot read the account")
        .expect("the account exists")
        .id
}

/// Reads the number of the active installation administrators (ADR-0029).
pub async fn platform_admin_count(db: &Database) -> i64 {
    let mut tx = db
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .expect("cannot begin the transaction");
    let count = users::platform_admin_count(&mut tx)
        .await
        .expect("cannot count the platform admins");
    tx.commit().await.expect("cannot commit the count");
    count
}

pub async fn list_users(db: &Database, tenant: TenantId) -> Vec<users::Member> {
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let result = users::list_members(&mut tx)
        .await
        .expect("cannot list the members");
    tx.commit().await.expect("cannot commit the list");
    result
}

pub async fn role_of(db: &Database, tenant: TenantId, user: restichub_db::UserId) -> Option<Role> {
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let result = memberships::role_of(&mut tx, user)
        .await
        .expect("cannot read the role");
    tx.commit().await.expect("cannot commit the role");
    result
}

pub async fn list_tokens(db: &Database, tenant: TenantId) -> Vec<api_tokens::ApiToken> {
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let result = api_tokens::list(&mut tx)
        .await
        .expect("cannot list the tokens");
    tx.commit().await.expect("cannot commit the list");
    result
}

pub async fn user_disabled(db: &Database, tenant: TenantId, user: restichub_db::UserId) -> bool {
    list_users(db, tenant)
        .await
        .iter()
        .find(|entry| entry.id == user)
        .is_some_and(|entry| entry.disabled || entry.account_disabled)
}

pub async fn token_revoked(
    db: &Database,
    tenant: TenantId,
    token: restichub_db::ApiTokenId,
) -> bool {
    list_tokens(db, tenant)
        .await
        .iter()
        .find(|entry| entry.id == token)
        .is_some_and(|entry| entry.revoked_at.is_some())
}

pub async fn send_response(app: &Router, request: Request<Body>) -> axum::response::Response {
    app.clone()
        .oneshot(request)
        .await
        .expect("the request failed")
}

pub fn retry_after(response: &axum::response::Response) -> Option<String> {
    response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

pub async fn test_db() -> Option<TestDatabase> {
    test_database().await
}

/// Builds an instant of the tests without the `time` macros feature.
pub fn utc(year: i32, month: time::Month, day: u8, hour: u8, minute: u8) -> OffsetDateTime {
    let date = time::Date::from_calendar_date(year, month, day).expect("a valid date");
    let time = time::Time::from_hms(hour, minute, 0).expect("a valid time");
    date.with_time(time).assume_utc()
}

/// The whole seconds from `from` to `to`, for [`restichub_cp::clock::ManualClock::advance`].
pub fn seconds_between(from: OffsetDateTime, to: OffsetDateTime) -> i64 {
    (to - from).whole_seconds()
}
