//! The HTTP server of the control plane.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode, request::Parts};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, delete, get, patch, post};
use axum::{Json, Router};
use restichub_db::Database;
use tokio::sync::Semaphore;
use tower_http::request_id::{
    MakeRequestId, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{MakeSpan, TraceLayer};
use tracing::Span;
use utoipa::Modify;
use utoipa::OpenApi;
use utoipa::openapi::OpenApi as OpenApiDocument;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme};
use uuid::Uuid;

use crate::client_ip::TrustedProxies;
use crate::clock::{Clock, SystemClock};
use crate::config::AuthConfig;
use crate::error::ApiError;
use crate::rate_limit::RateLimiter;

const REQUEST_ID_HEADER: &str = "x-request-id";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The largest request body of an authentication or a setup route.
pub const BODY_LIMIT: usize = 16 * 1024;

/// The path of the tenant audit endpoint.
pub const AUDIT_TENANT_PATH: &str = "/api/v1/audit";
/// The path of the platform audit endpoint.
pub const AUDIT_PLATFORM_PATH: &str = "/api/v1/platform/audit";

/// The authentication state of the server.
///
/// The type holds the configuration, the two sign-in rate limiters and the clock. A `Debug` shows
/// the configuration only.
#[derive(Clone)]
pub struct AuthState {
    /// The local authentication settings.
    pub config: AuthConfig,
    /// The capped limiter for the synthetic sign-in identities.
    pub user_limiter: Arc<RateLimiter>,
    /// The capped limiter for each source IP network.
    pub ip_limiter: Arc<RateLimiter>,
    /// The uncapped limiter for the resolved `(tenant_id, user_id)` identities.
    pub resolved_limiter: Arc<RateLimiter>,
    /// The clock of the session and token checks.
    pub clock: Arc<dyn Clock>,
    /// The semaphore that bounds the number of concurrent password hashes.
    pub hash_semaphore: Arc<Semaphore>,
}

impl std::fmt::Debug for AuthState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthState")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl AuthState {
    /// Creates the authentication state from the configuration.
    pub fn new(config: AuthConfig) -> Self {
        let window = Duration::from_secs(60);
        let user_limiter = RateLimiter::new(
            config.sign_in_user_rate,
            window,
            crate::rate_limit::MAX_ENTRIES,
        );
        let ip_limiter = RateLimiter::new(
            config.sign_in_ip_rate,
            window,
            crate::rate_limit::MAX_ENTRIES,
        );
        let resolved_limiter = RateLimiter::new(config.sign_in_user_rate, window, 0);
        let hash_semaphore = Arc::new(Semaphore::new(config.max_concurrent_hashes as usize));
        Self {
            config,
            user_limiter: Arc::new(user_limiter),
            ip_limiter: Arc::new(ip_limiter),
            resolved_limiter: Arc::new(resolved_limiter),
            clock: Arc::new(SystemClock),
            hash_semaphore,
        }
    }

    /// Replaces the cap of the synthetic and the IP maps. A test uses this method.
    pub fn with_bucket_cap(mut self, cap: usize) -> Self {
        let window = Duration::from_secs(60);
        self.user_limiter = Arc::new(RateLimiter::new(self.config.sign_in_user_rate, window, cap));
        self.ip_limiter = Arc::new(RateLimiter::new(self.config.sign_in_ip_rate, window, cap));
        self
    }

    /// Replaces the clock. A test uses this method.
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }
}

/// The application state of the HTTP server.
#[derive(Debug, Clone)]
pub struct AppState {
    db: Database,
    trusted_proxies: TrustedProxies,
    auth: AuthState,
}

impl AppState {
    /// Creates the application state with the default authentication settings.
    pub fn new(db: Database) -> Self {
        Self {
            db,
            trusted_proxies: TrustedProxies::default(),
            auth: AuthState::new(AuthConfig::default()),
        }
    }

    /// Creates the application state with a given authentication state.
    pub fn with_auth(db: Database, auth: AuthState) -> Self {
        Self {
            db,
            trusted_proxies: TrustedProxies::default(),
            auth,
        }
    }

    /// Sets the trusted proxy list.
    pub fn with_trusted_proxies(mut self, trusted_proxies: TrustedProxies) -> Self {
        self.trusted_proxies = trusted_proxies;
        self
    }

    /// Returns the database.
    pub fn db(&self) -> &Database {
        &self.db
    }

    /// Returns the trusted proxy list.
    pub fn trusted_proxies(&self) -> &TrustedProxies {
        &self.trusted_proxies
    }

    /// Returns the authentication state.
    pub fn auth(&self) -> &AuthState {
        &self.auth
    }
}

/// Builds the HTTP router.
///
/// The router registers one route for each entry of [`crate::context::ROUTE_OPERATIONS`], so the
/// authorization table and the router cannot drift apart. A new operation without a handler stops
/// the start with a clear message.
pub fn router(state: AppState) -> Router {
    let mut router = Router::new();
    for route in crate::context::ROUTE_OPERATIONS {
        router = router.route(route.path, method_router(route.method, route.path));
    }
    router
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(TraceLayer::new_for_http().make_span_with(RequestSpan))
        .layer(PropagateRequestIdLayer::new(request_id_header()))
        .layer(SetRequestIdLayer::new(request_id_header(), MakeRequestIdV7))
}

/// Returns the method router of one route.
///
/// The body-carrying authentication and setup routes get the small body limit [`BODY_LIMIT`].
fn method_router(method: &str, path: &str) -> MethodRouter<AppState> {
    match (method, path) {
        ("get", "/healthz") => get(healthz),
        ("get", "/readyz") => get(readyz),
        ("get", "/api/v1/openapi.json") => get(openapi),
        ("post", "/api/v1/auth/sign-in") => {
            post(crate::auth::sign_in).layer(DefaultBodyLimit::max(BODY_LIMIT))
        }
        ("post", "/api/v1/auth/sign-out") => post(crate::auth::sign_out),
        ("get", "/api/v1/auth/me") => get(crate::auth::me),
        ("post", "/api/v1/auth/password") => {
            post(crate::auth::change_password).layer(DefaultBodyLimit::max(BODY_LIMIT))
        }
        ("post", "/api/v1/setup") => {
            post(crate::setup::setup).layer(DefaultBodyLimit::max(BODY_LIMIT))
        }
        ("get", "/api/v1/setup/status") => get(crate::setup::status),
        ("get", AUDIT_TENANT_PATH) => get(crate::audit::list_tenant_audit),
        ("get", AUDIT_PLATFORM_PATH) => get(crate::audit::list_platform_audit),
        ("get", "/api/v1/users") => get(crate::users::list_users),
        ("post", "/api/v1/users") => {
            post(crate::users::create_user).layer(DefaultBodyLimit::max(BODY_LIMIT))
        }
        ("get", "/api/v1/users/{id}") => get(crate::users::get_user),
        ("patch", "/api/v1/users/{id}") => {
            patch(crate::users::update_user).layer(DefaultBodyLimit::max(BODY_LIMIT))
        }
        ("delete", "/api/v1/users/{id}") => delete(crate::users::delete_user),
        ("post", "/api/v1/users/{id}/password") => {
            post(crate::users::reset_password).layer(DefaultBodyLimit::max(BODY_LIMIT))
        }
        ("get", "/api/v1/api-tokens") => get(crate::api_tokens::list_tokens),
        ("post", "/api/v1/api-tokens") => {
            post(crate::api_tokens::create_token).layer(DefaultBodyLimit::max(BODY_LIMIT))
        }
        ("delete", "/api/v1/api-tokens/{id}") => delete(crate::api_tokens::revoke_token),
        _ => panic!("the route {method} {path} has no handler"),
    }
}

/// Liveness probe. The process is alive. The function does not touch the database.
#[utoipa::path(
    get,
    path = "/healthz",
    responses((status = 200, description = "The process is alive."))
)]
async fn healthz() -> &'static str {
    "ok"
}

/// Readiness probe. The function checks the database.
#[utoipa::path(
    get,
    path = "/readyz",
    responses(
        (status = 200, description = "The database is ready."),
        (status = 503, description = "The database is not ready.")
    )
)]
async fn readyz(State(state): State<AppState>, request_id: RequestIdHeader) -> Response {
    if state.db.is_ready().await {
        StatusCode::OK.into_response()
    } else {
        ApiError::service_unavailable(&request_id.0).into_response()
    }
}

/// The OpenAPI document.
#[utoipa::path(
    get,
    path = "/api/v1/openapi.json",
    responses((status = 200, description = "The OpenAPI document."))
)]
async fn openapi() -> impl IntoResponse {
    Json(ApiDoc::openapi())
}

/// The OpenAPI document of the control plane API.
#[derive(OpenApi)]
#[openapi(
    info(title = "ResticHub API", version = env!("CARGO_PKG_VERSION")),
    paths(
        healthz,
        readyz,
        openapi,
        crate::audit::list_tenant_audit,
        crate::audit::list_platform_audit,
        crate::auth::sign_in,
        crate::auth::sign_out,
        crate::auth::me,
        crate::auth::change_password,
        crate::setup::setup,
        crate::setup::status,
        crate::users::list_users,
        crate::users::create_user,
        crate::users::get_user,
        crate::users::update_user,
        crate::users::delete_user,
        crate::users::reset_password,
        crate::api_tokens::list_tokens,
        crate::api_tokens::create_token,
        crate::api_tokens::revoke_token
    ),
    modifiers(&SecurityAddon)
)]
struct ApiDoc;

/// Adds the two security schemes to the OpenAPI document.
///
/// The scheme `cookie` is the session cookie. The scheme `bearer` is an API token.
struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut OpenApiDocument) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "cookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new(
                crate::token::SESSION_COOKIE,
            ))),
        );
        components.add_security_scheme(
            "bearer",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    }
}

/// The fallback handler for an unknown route.
async fn not_found(request_id: RequestIdHeader) -> ApiError {
    ApiError::not_found(&request_id.0)
}

/// The fallback handler for a known route with a method that the route does not accept.
async fn method_not_allowed(request_id: RequestIdHeader) -> ApiError {
    ApiError::method_not_allowed(&request_id.0)
}

/// Returns the request ID of a request.
///
/// The extractors use the function to build an error before the request reaches a handler.
pub(crate) fn request_id_of(parts: &Parts) -> String {
    parts
        .headers
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// The request ID of the current request.
///
/// The request ID comes from the `x-request-id` header. The middleware [`MakeRequestIdV7`] sets
/// the header if it is absent.
#[derive(Debug, Clone)]
pub struct RequestIdHeader(pub String);

impl<S> axum::extract::FromRequestParts<S> for RequestIdHeader
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let request_id = parts
            .headers
            .get(REQUEST_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        Ok(Self(request_id.to_owned()))
    }
}

/// Creates a request ID with UUIDv7.
#[derive(Debug, Clone, Copy)]
struct MakeRequestIdV7;

impl MakeRequestId for MakeRequestIdV7 {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        let value = HeaderValue::from_str(&Uuid::now_v7().to_string()).ok()?;
        Some(RequestId::new(value))
    }
}

/// Makes a tracing span with the request ID.
#[derive(Debug, Clone, Copy)]
struct RequestSpan;

impl<B> MakeSpan<B> for RequestSpan {
    fn make_span(&mut self, request: &Request<B>) -> Span {
        let request_id = request
            .headers()
            .get(REQUEST_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        tracing::info_span!(
            "request",
            method = %request.method(),
            uri = %request.uri(),
            request_id = %request_id
        )
    }
}

fn request_id_header() -> HeaderName {
    HeaderName::from_static(REQUEST_ID_HEADER)
}
