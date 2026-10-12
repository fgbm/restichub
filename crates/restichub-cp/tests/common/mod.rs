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
use restichub_cp::config::AuthConfig;
use restichub_cp::server::{AppState, AuthState, router};
use restichub_db::test_util::{TestDatabase, test_database};
use restichub_db::{Database, TenantId, api_tokens, memberships, sessions, tenants, users};
use serde_json::Value;
use time::OffsetDateTime;
use tower::ServiceExt;

/// The session cookie name over HTTPS.
pub const COOKIE_NAME: &str = "__Host-restichub_session";
/// The CSRF header and its value.
pub const CSRF: (&str, &str) = ("x-restichub-csrf", "1");

/// Returns the socket peer of a test request.
pub fn peer() -> ConnectInfo<SocketAddr> {
    ConnectInfo(
        "203.0.113.7:1234"
            .parse()
            .unwrap_or_else(|_| SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 0))),
    )
}

/// Builds a request with the socket peer and an optional cookie and body.
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

/// Builds a request with a raw body string and a session cookie.
///
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

/// Builds a request with a bearer token.
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

/// Reads the JSON body of a response.
pub async fn body_json(response: axum::response::Response) -> Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("cannot read the body")
        .to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// Sends a request and returns the status, the session cookie pair and the JSON body.
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

/// Returns the `name=value` part of a `Set-Cookie` header value.
fn cookie_pair(header: &str) -> Option<String> {
    header.split(';').next().map(str::to_owned)
}

/// Builds a router with the default authentication state.
pub fn app(db: &Database) -> Router {
    router(AppState::new(db.clone()))
}

/// Builds a router with a given authentication configuration and clock.
pub fn app_with_auth(db: &Database, config: AuthConfig, clock: Arc<dyn Clock>) -> Router {
    let auth = AuthState::new(config).with_clock(clock);
    router(AppState::with_auth(db.clone(), auth))
}

/// A key encryption key provider that holds one fixed key. A test uses this type.
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

/// Returns a fixed key encryption key provider for a test.
pub fn test_kek() -> Arc<dyn restichub_secrets::KekProvider> {
    let mut bytes = [7_u8; 32];
    Arc::new(TestKek(restichub_secrets::Kek::from_bytes(&mut bytes)))
}

/// Builds a router with the default authentication state and a key encryption key provider.
pub fn app_with_kek(db: &Database) -> Router {
    router(AppState::new(db.clone()).with_kek(test_kek()))
}

/// Creates a tenant through a platform transaction.
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

/// Sets the status of a tenant through a platform transaction.
pub async fn set_tenant_status(db: &Database, tenant: TenantId, status: &str) {
    let mut tx = db
        .begin_platform()
        .await
        .expect("cannot begin the transaction");
    tenants::set_status(&mut tx, tenant, status)
        .await
        .expect("cannot set the tenant status");
    tx.commit().await.expect("cannot commit the status");
}

/// Creates a user with a role and a password hash.
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
    let user = users::create(
        &mut tx,
        &users::NewUser {
            username,
            password_hash,
            display_name: username,
        },
    )
    .await
    .expect("cannot create the user");
    memberships::set_role(&mut tx, user, role)
        .await
        .expect("cannot set the role");
    tx.commit().await.expect("cannot commit the user");
    user
}

/// Creates a session for a user and returns the cookie header value.
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

/// Creates a user with a role and a session, and returns the cookie header value.
pub async fn seed_session(db: &Database, tenant: TenantId, username: &str, role: &str) -> String {
    let user = create_user(db, tenant, username, "phc", role).await;
    session_cookie(db, tenant, user).await
}

/// Creates an API token with a role and returns its value.
///
/// The creator is a user with the Admin role.
pub async fn seed_api_token(db: &Database, tenant: TenantId, role: &str) -> String {
    let creator_name = format!("creator-{role}");
    let creator = create_user(db, tenant, &creator_name, "phc", "admin").await;
    seed_token(db, tenant, role, creator).await
}

/// Creates an Admin API token whose creator is not an Admin and returns its value.
///
/// The token acts with the Admin role, but the creator does not count as an Admin. A test uses this
/// method to keep a stable actor while it disables the two Admins of a tenant.
pub async fn seed_admin_token_actor(db: &Database, tenant: TenantId) -> String {
    let creator = create_user(db, tenant, "token-owner", "phc", "viewer").await;
    seed_token(db, tenant, "admin", creator).await
}

/// Inserts an API token with a role and a creator.
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
            role,
            created_by,
            expires_at: None,
        },
    )
    .await
    .expect("cannot create the token");
    tx.commit().await.expect("cannot commit the token");
    value
}

/// Returns the number of rows of a table in a tenant transaction.
pub async fn count(db: &Database, tenant: TenantId, table: &'static str) -> i64 {
    restichub_db::test_util::count_rows(db, tenant, table)
        .await
        .expect("cannot count the rows")
}

/// Lists the users of a tenant.
pub async fn list_users(db: &Database, tenant: TenantId) -> Vec<users::User> {
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let result = users::list(&mut tx).await.expect("cannot list the users");
    tx.commit().await.expect("cannot commit the list");
    result
}

/// Returns the role of a user in a tenant.
pub async fn role_of(
    db: &Database,
    tenant: TenantId,
    user: restichub_db::UserId,
) -> Option<String> {
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

/// Lists the API tokens of a tenant.
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

/// Returns true if a user of a tenant is disabled.
pub async fn user_disabled(db: &Database, tenant: TenantId, user: restichub_db::UserId) -> bool {
    list_users(db, tenant)
        .await
        .iter()
        .find(|entry| entry.id == user)
        .is_some_and(|entry| entry.disabled)
}

/// Returns true if an API token of a tenant is revoked.
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

/// Sends a request and returns the raw response.
pub async fn send_response(app: &Router, request: Request<Body>) -> axum::response::Response {
    app.clone()
        .oneshot(request)
        .await
        .expect("the request failed")
}

/// Reads the `Retry-After` header of a response.
pub fn retry_after(response: &axum::response::Response) -> Option<String> {
    response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// A test database or a skip.
pub async fn test_db() -> Option<TestDatabase> {
    test_database().await
}
