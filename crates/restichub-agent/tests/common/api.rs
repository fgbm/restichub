//! Web API helpers for the agent job tests.
//!
//! A test starts an init or a test-access job over the public web API, so it needs a session and a
//! JSON request, as the control plane tests do.

#![allow(dead_code)]

use restichub_cp::server::AppState;
use restichub_db::{Database, TenantId};
use time::{Duration, OffsetDateTime};

const COOKIE_NAME: &str = "__Host-restichub_session";

/// The public web router of a state, for a test that starts a job over the API.
pub fn api_router(state: AppState) -> axum::Router {
    restichub_cp::server::router(state)
}

/// Builds a JSON request with a session cookie and the CSRF header.
pub fn request(
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<serde_json::Value>,
) -> axum::http::Request<axum::body::Body> {
    use axum::http::{Request, header};
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = cookie {
        builder = builder.header(header::COOKIE, value);
    }
    // An unsafe method needs the CSRF header when the request is cookie-authenticated.
    if matches!(method, "POST" | "PUT" | "PATCH" | "DELETE") {
        builder = builder.header("x-restichub-csrf", "1");
    }
    let body = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            axum::body::Body::from(value.to_string())
        }
        None => axum::body::Body::empty(),
    };
    let mut request = builder.body(body).expect("cannot build the request");
    // The request context reads the socket peer address; the router test inserts it by hand.
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
            [127, 0, 0, 1],
            0,
        ))));
    request
}

/// Sends a request to the router and returns the status, the session cookie and the JSON body.
pub async fn send(
    app: &axum::Router,
    request: axum::http::Request<axum::body::Body>,
) -> (axum::http::StatusCode, Option<String>, serde_json::Value) {
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("the request failed");
    let status = response.status();
    let cookie = response
        .headers()
        .get(axum::http::header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|header| header.split(';').next().map(str::to_owned));
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("cannot read the body")
        .to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, cookie, body)
}

/// Creates an Admin session for a tenant and returns its cookie value (FR-AU-01).
pub async fn seed_session(db: &Database, tenant: TenantId, username: &str, role: &str) -> String {
    let user = super::create_user(db, tenant, username, "phc", role).await;
    let value = restichub_cp::token::random_token().expect("cannot build the token");
    let hash = restichub_cp::token::hash_token(&value);
    let now = OffsetDateTime::now_utc();
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    restichub_db::sessions::create(
        &mut tx,
        user,
        &hash,
        now + Duration::hours(12),
        now,
        None,
        None,
    )
    .await
    .expect("cannot create the session");
    tx.commit().await.expect("cannot commit the session");
    format!("{COOKIE_NAME}={value}")
}
