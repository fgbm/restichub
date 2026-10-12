//! Shared helpers for the control plane integration tests.
//!
//! A test builds its authentication through a real session or a real API token in the database. The
//! extractor no longer accepts a context from the request extensions.

#![allow(dead_code)]

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
    let mut request = match builder.body(body) {
        Ok(request) => request,
        Err(error) => panic!("cannot build the request: {error}"),
    };
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
    let mut request = match builder.body(body) {
        Ok(request) => request,
        Err(error) => panic!("cannot build the request: {error}"),
    };
    request.extensions_mut().insert(peer());
    request
}

/// Reads the JSON body of a response.
pub async fn body_json(response: axum::response::Response) -> Value {
    let bytes = match response.into_body().collect().await {
        Ok(body) => body.to_bytes(),
        Err(error) => panic!("cannot read the body: {error}"),
    };
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// Sends a request and returns the status, the session cookie pair and the JSON body.
pub async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Option<String>, Value) {
    let response = match app.clone().oneshot(request).await {
        Ok(response) => response,
        Err(error) => panic!("the request failed: {error}"),
    };
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
    let mut tx = match db.begin_platform().await {
        Ok(tx) => tx,
        Err(error) => panic!("cannot begin the transaction: {error}"),
    };
    let tenant = match tenants::create(&mut tx, name).await {
        Ok(tenant) => tenant,
        Err(error) => panic!("cannot create the tenant: {error}"),
    };
    if let Err(error) = tx.commit().await {
        panic!("cannot commit the tenant: {error}");
    }
    tenant
}

/// Sets the status of a tenant through a platform transaction.
pub async fn set_tenant_status(db: &Database, tenant: TenantId, status: &str) {
    let mut tx = match db.begin_platform().await {
        Ok(tx) => tx,
        Err(error) => panic!("cannot begin the transaction: {error}"),
    };
    if let Err(error) = tenants::set_status(&mut tx, tenant, status).await {
        panic!("cannot set the tenant status: {error}");
    }
    if let Err(error) = tx.commit().await {
        panic!("cannot commit the status: {error}");
    }
}

/// Creates a user with a role and a password hash.
pub async fn create_user(
    db: &Database,
    tenant: TenantId,
    username: &str,
    password_hash: &str,
    role: &str,
) -> restichub_db::UserId {
    let mut tx = match db.begin_tenant(tenant).await {
        Ok(tx) => tx,
        Err(error) => panic!("cannot begin the transaction: {error}"),
    };
    let user = match users::create(
        &mut tx,
        &users::NewUser {
            username,
            password_hash,
            display_name: username,
        },
    )
    .await
    {
        Ok(user) => user,
        Err(error) => panic!("cannot create the user: {error}"),
    };
    if let Err(error) = memberships::set_role(&mut tx, user, role).await {
        panic!("cannot set the role: {error}");
    }
    if let Err(error) = tx.commit().await {
        panic!("cannot commit the user: {error}");
    }
    user
}

/// Creates a session for a user and returns the cookie header value.
pub async fn session_cookie(db: &Database, tenant: TenantId, user: restichub_db::UserId) -> String {
    let value = match restichub_cp::token::random_token() {
        Ok(value) => value,
        Err(error) => panic!("cannot build the token: {error}"),
    };
    let hash = restichub_cp::token::hash_token(&value);
    let now = OffsetDateTime::now_utc();
    let mut tx = match db.begin_tenant(tenant).await {
        Ok(tx) => tx,
        Err(error) => panic!("cannot begin the transaction: {error}"),
    };
    if let Err(error) = sessions::create(
        &mut tx,
        user,
        &hash,
        now + time::Duration::hours(12),
        now,
        None,
        None,
    )
    .await
    {
        panic!("cannot create the session: {error}");
    }
    if let Err(error) = tx.commit().await {
        panic!("cannot commit the session: {error}");
    }
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
    let (value, prefix) = match restichub_cp::token::random_api_token() {
        Ok(pair) => pair,
        Err(error) => panic!("cannot build the token: {error}"),
    };
    let hash = restichub_cp::token::hash_token(&value);
    let mut tx = match db.begin_tenant(tenant).await {
        Ok(tx) => tx,
        Err(error) => panic!("cannot begin the transaction: {error}"),
    };
    if let Err(error) = api_tokens::create(
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
    {
        panic!("cannot create the token: {error}");
    }
    if let Err(error) = tx.commit().await {
        panic!("cannot commit the token: {error}");
    }
    value
}

/// Returns the number of rows of a table in a tenant transaction.
pub async fn count(db: &Database, tenant: TenantId, table: &'static str) -> i64 {
    match restichub_db::test_util::count_rows(db, tenant, table).await {
        Ok(count) => count,
        Err(error) => panic!("cannot count the rows: {error}"),
    }
}

/// Lists the users of a tenant.
pub async fn list_users(db: &Database, tenant: TenantId) -> Vec<users::User> {
    let mut tx = match db.begin_tenant(tenant).await {
        Ok(tx) => tx,
        Err(error) => panic!("cannot begin the transaction: {error}"),
    };
    let result = match users::list(&mut tx).await {
        Ok(list) => list,
        Err(error) => panic!("cannot list the users: {error}"),
    };
    if let Err(error) = tx.commit().await {
        panic!("cannot commit the list: {error}");
    }
    result
}

/// Returns the role of a user in a tenant.
pub async fn role_of(
    db: &Database,
    tenant: TenantId,
    user: restichub_db::UserId,
) -> Option<String> {
    let mut tx = match db.begin_tenant(tenant).await {
        Ok(tx) => tx,
        Err(error) => panic!("cannot begin the transaction: {error}"),
    };
    let result = match memberships::role_of(&mut tx, user).await {
        Ok(role) => role,
        Err(error) => panic!("cannot read the role: {error}"),
    };
    if let Err(error) = tx.commit().await {
        panic!("cannot commit the role: {error}");
    }
    result
}

/// Lists the API tokens of a tenant.
pub async fn list_tokens(db: &Database, tenant: TenantId) -> Vec<api_tokens::ApiToken> {
    let mut tx = match db.begin_tenant(tenant).await {
        Ok(tx) => tx,
        Err(error) => panic!("cannot begin the transaction: {error}"),
    };
    let result = match api_tokens::list(&mut tx).await {
        Ok(list) => list,
        Err(error) => panic!("cannot list the tokens: {error}"),
    };
    if let Err(error) = tx.commit().await {
        panic!("cannot commit the list: {error}");
    }
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
    match app.clone().oneshot(request).await {
        Ok(response) => response,
        Err(error) => panic!("the request failed: {error}"),
    }
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
