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
pub fn write_body(
    method: &str,
    path: &str,
    index: usize,
    platform_admin: bool,
    tenant_id: &str,
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
        ("PUT", _) => Some(json!({"value": "matrix-value"})),
        ("POST", "/api/v1/platform/tenants") => Some(json!({
            "name": format!("matrix-{index}"),
            "first_admin": {
                "username": format!("admin-{index}"),
                "display_name": "Matrix",
                "password": "a long enough password",
            },
        })),
        ("POST", "/api/v1/platform/users/{id}/memberships") => {
            Some(json!({"tenant_id": tenant_id, "role": "viewer"}))
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
            hostname,
            os: "linux",
            arch: "amd64",
            agent_version: "0.1.0",
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
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let role = Role::parse(role).expect("the role is known");
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

/// Creates a Platform Admin API token and returns its value.
///
/// The platform tenant holds the role `platform_admin` only (FR-TN-02), so the creator holds that
/// role.
pub async fn seed_platform_api_token(db: &Database) -> String {
    let creator = create_user(
        db,
        PLATFORM_TENANT_ID,
        "platform-token-owner",
        "phc",
        "platform_admin",
    )
    .await;
    seed_token(db, PLATFORM_TENANT_ID, "platform_admin", creator).await
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
