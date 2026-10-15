use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use http_body_util::BodyExt;
use restichub_db::Database;
use restichub_secrets::KekProvider;
use tokio::sync::{Notify, Semaphore};
use tower_http::request_id::{
    MakeRequestId, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{MakeSpan, TraceLayer};
use tracing::Span;
use utoipa::Modify;
use utoipa::openapi::OpenApi as OpenApiDocument;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme};
use utoipa_axum::router::OpenApiRouter;
use uuid::Uuid;

use crate::agent_stream::StreamRegistry;
use crate::client_ip::TrustedProxies;
use crate::clock::{Clock, SystemClock};
use crate::config::{AuthConfig, TenancyConfig};
use crate::error::{ApiError, InternalCause, PROBLEM_CONTENT_TYPE};
use crate::job_signing::JobSigningService;
use crate::metrics::SnapshotCache;
use crate::pki::CaStore;
use crate::queue::QueueConfig;
use crate::rate_limit::RateLimiter;
use crate::routes::ROUTES;

const REQUEST_ID_HEADER: &str = "x-request-id";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The longest time between two rotation requests of one agent (FR-ID-03).
pub const ROTATION_COOLDOWN: time::Duration = time::Duration::minutes(10);
pub const BODY_LIMIT: usize = 16 * 1024;
/// Largest request body of a secret route.
///
/// A secret value holds at most 64 KiB bytes. A request holds at most six of them and the metadata,
/// so the limit is six values plus 4 KiB.
pub const SECRET_BODY_LIMIT: usize = 6 * 64 * 1024 + 4 * 1024;
/// Largest request body of a route that carries scripts (FR-HK-09).
///
/// A profile holds at most 1 MiB of script text. The JSON escapes of quotes and line breaks add to
/// it, so the limit is 3 MiB.
pub const SCRIPT_BODY_LIMIT: usize = 3 * 1024 * 1024;

/// The account create limit for each actor and tenant in one minute (threat T15).
pub const CREATE_USER_RATE: u32 = 20;

/// The account create limit for each tenant in one minute. An actor cannot bypass the limit by
/// minting many API tokens, so a second bucket counts the tenant (threat T15).
pub const CREATE_USER_TENANT_RATE: u32 = 60;

/// The snapshot list and tree page requests of one actor in one minute (ADR-0035). Each request
/// can start a job that reads a repository, so an actor cannot flood an agent with them.
pub const BROWSE_RATE: u32 = 60;
/// The test-send limit for each actor and tenant in one minute. Each test makes an outbound
/// connection to a target that the tenant chose (threat T29).
pub const TEST_SEND_RATE: u32 = 6;

/// The test-send limit for each tenant in one minute.
pub const TEST_SEND_TENANT_RATE: u32 = 20;

/// The most test sends that run at the same time in one control plane instance.
pub const TEST_SEND_CONCURRENCY: usize = 8;

/// The enrollment limit for each source IP address in one minute (FR-ID-05, AC-ID-05).
///
/// The 11th request from one address in one minute is refused with HTTP 429.
pub const ENROLL_IP_RATE: u32 = 10;

/// The enrollment limit for each enrollment token prefix in one minute (FR-ID-05).
pub const ENROLL_TOKEN_RATE: u32 = 10;

/// The type holds the configuration, the sign-in rate limiters, the account create limiters and the
/// clock. A `Debug` shows the configuration only.
#[derive(Clone)]
pub struct AuthState {
    pub config: AuthConfig,
    pub user_limiter: Arc<RateLimiter>,
    pub ip_limiter: Arc<RateLimiter>,
    pub resolved_limiter: Arc<RateLimiter>,
    pub create_user_limiter: Arc<RateLimiter>,
    pub create_user_tenant_limiter: Arc<RateLimiter>,
    pub test_send_limiter: Arc<RateLimiter>,
    pub test_send_tenant_limiter: Arc<RateLimiter>,
    pub test_send_semaphore: Arc<Semaphore>,
    pub enroll_ip_limiter: Arc<RateLimiter>,
    pub enroll_token_limiter: Arc<RateLimiter>,
    pub browse_limiter: Arc<RateLimiter>,
    pub clock: Arc<dyn Clock>,
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
        let create_user_limiter =
            RateLimiter::new(CREATE_USER_RATE, window, crate::rate_limit::MAX_ENTRIES);
        let create_user_tenant_limiter = RateLimiter::new(
            CREATE_USER_TENANT_RATE,
            window,
            crate::rate_limit::MAX_ENTRIES,
        );
        let test_send_limiter =
            RateLimiter::new(TEST_SEND_RATE, window, crate::rate_limit::MAX_ENTRIES);
        let test_send_tenant_limiter = RateLimiter::new(
            TEST_SEND_TENANT_RATE,
            window,
            crate::rate_limit::MAX_ENTRIES,
        );
        let enroll_ip_limiter =
            RateLimiter::new(ENROLL_IP_RATE, window, crate::rate_limit::MAX_ENTRIES);
        let enroll_token_limiter =
            RateLimiter::new(ENROLL_TOKEN_RATE, window, crate::rate_limit::MAX_ENTRIES);
        let browse_limiter = RateLimiter::new(BROWSE_RATE, window, crate::rate_limit::MAX_ENTRIES);
        let hash_semaphore = Arc::new(Semaphore::new(config.max_concurrent_hashes as usize));
        Self {
            config,
            user_limiter: Arc::new(user_limiter),
            ip_limiter: Arc::new(ip_limiter),
            resolved_limiter: Arc::new(resolved_limiter),
            create_user_limiter: Arc::new(create_user_limiter),
            create_user_tenant_limiter: Arc::new(create_user_tenant_limiter),
            test_send_limiter: Arc::new(test_send_limiter),
            test_send_tenant_limiter: Arc::new(test_send_tenant_limiter),
            test_send_semaphore: Arc::new(Semaphore::new(TEST_SEND_CONCURRENCY)),
            enroll_ip_limiter: Arc::new(enroll_ip_limiter),
            enroll_token_limiter: Arc::new(enroll_token_limiter),
            browse_limiter: Arc::new(browse_limiter),
            clock: Arc::new(SystemClock),
            hash_semaphore,
        }
    }

    pub fn with_bucket_cap(mut self, cap: usize) -> Self {
        let window = Duration::from_secs(60);
        self.user_limiter = Arc::new(RateLimiter::new(self.config.sign_in_user_rate, window, cap));
        self.ip_limiter = Arc::new(RateLimiter::new(self.config.sign_in_ip_rate, window, cap));
        self.create_user_limiter = Arc::new(RateLimiter::new(CREATE_USER_RATE, window, cap));
        self
    }

    /// Replaces the per-tenant account create bucket, so a test can reach its limit quickly.
    pub fn with_create_user_tenant_rate(mut self, capacity: u32) -> Self {
        let window = Duration::from_secs(60);
        self.create_user_tenant_limiter = Arc::new(RateLimiter::new(
            capacity,
            window,
            crate::rate_limit::MAX_ENTRIES,
        ));
        self
    }

    /// Replaces the browse bucket, so a test can reach its limit quickly.
    pub fn with_browse_rate(mut self, capacity: u32) -> Self {
        let window = Duration::from_secs(60);
        self.browse_limiter = Arc::new(RateLimiter::new(
            capacity,
            window,
            crate::rate_limit::MAX_ENTRIES,
        ));
        self
    }

    /// Replaces the test-send buckets and the concurrency bound, so a test can reach a limit quickly.
    pub fn with_test_send_limits(mut self, actor: u32, tenant: u32, concurrency: usize) -> Self {
        let window = Duration::from_secs(60);
        self.test_send_limiter = Arc::new(RateLimiter::new(
            actor,
            window,
            crate::rate_limit::MAX_ENTRIES,
        ));
        self.test_send_tenant_limiter = Arc::new(RateLimiter::new(
            tenant,
            window,
            crate::rate_limit::MAX_ENTRIES,
        ));
        self.test_send_semaphore = Arc::new(Semaphore::new(concurrency));
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }
}

#[derive(Debug, Clone)]
pub struct AppState {
    db: Database,
    trusted_proxies: TrustedProxies,
    auth: AuthState,
    kek: Option<Arc<dyn KekProvider>>,
    tenancy: TenancyConfig,
    ca: Option<Arc<CaStore>>,
    job_signing: Option<JobSigningService>,
    agent_endpoint: Option<String>,
    agent_cert_lifetime: time::Duration,
    agent_registry: Arc<StreamRegistry>,
    queue: QueueConfig,
    dispatch_wake: Arc<Notify>,
    log_retention_days: u32,
    metrics_cache: Arc<SnapshotCache>,
    dashboard_cache: Arc<crate::dashboard::DashboardCache>,
    sender: Option<crate::alerts::sender::Sender>,
    progress_throttle:
        Arc<crate::progress_throttle::ProgressThrottle<crate::job_data::ProgressUpdate>>,
}

impl AppState {
    /// Creates the application state with the default authentication settings.
    ///
    /// The state holds no key encryption key provider. A test that creates a tenant calls
    /// [`AppState::with_kek`]. The production server always sets the provider. The tenancy mode is
    /// single, the default of the configuration; the production server sets it from the
    /// configuration with [`AppState::with_tenancy`].
    pub fn new(db: Database) -> Self {
        Self {
            db,
            trusted_proxies: TrustedProxies::default(),
            auth: AuthState::new(AuthConfig::default()),
            kek: None,
            tenancy: TenancyConfig::default(),
            ca: None,
            job_signing: None,
            agent_endpoint: None,
            agent_cert_lifetime: crate::pki::AGENT_CERT_LIFETIME,
            agent_registry: Arc::new(StreamRegistry::new()),
            queue: QueueConfig::default(),
            dispatch_wake: Arc::new(Notify::new()),
            log_retention_days: crate::log_store::DEFAULT_RETENTION_DAYS,
            metrics_cache: Arc::new(SnapshotCache::default()),
            dashboard_cache: Arc::new(crate::dashboard::DashboardCache::default()),
            sender: None,
            progress_throttle: Arc::default(),
        }
    }

    pub fn with_auth(db: Database, auth: AuthState) -> Self {
        Self {
            db,
            trusted_proxies: TrustedProxies::default(),
            auth,
            kek: None,
            tenancy: TenancyConfig::default(),
            ca: None,
            job_signing: None,
            agent_endpoint: None,
            agent_cert_lifetime: crate::pki::AGENT_CERT_LIFETIME,
            agent_registry: Arc::new(StreamRegistry::new()),
            queue: QueueConfig::default(),
            dispatch_wake: Arc::new(Notify::new()),
            log_retention_days: crate::log_store::DEFAULT_RETENTION_DAYS,
            metrics_cache: Arc::new(SnapshotCache::default()),
            dashboard_cache: Arc::new(crate::dashboard::DashboardCache::default()),
            sender: None,
            progress_throttle: Arc::default(),
        }
    }

    pub fn with_trusted_proxies(mut self, trusted_proxies: TrustedProxies) -> Self {
        self.trusted_proxies = trusted_proxies;
        self
    }

    /// The server loads the provider at start and passes it here. The provider wraps the data
    /// encryption key of a new tenant (ADR-0023, section Secrets).
    pub fn with_kek(mut self, kek: Arc<dyn KekProvider>) -> Self {
        self.kek = Some(kek);
        self
    }

    /// The server passes the tenancy configuration of the process here (ADR-0024).
    pub fn with_tenancy(mut self, tenancy: TenancyConfig) -> Self {
        self.tenancy = tenancy;
        self
    }

    /// The server passes the loaded certificate authority, the job signing key set and the gRPC
    /// endpoint here, so the enrollment endpoint can issue the agent certificate and return the
    /// signed key set (ADR-0026, FR-ID-02).
    pub fn with_pki(
        mut self,
        ca: Arc<CaStore>,
        job_signing: JobSigningService,
        agent_endpoint: Option<String>,
    ) -> Self {
        self.ca = Some(ca);
        self.job_signing = Some(job_signing);
        self.agent_endpoint = agent_endpoint;
        self
    }

    /// The server passes the lifetime of a rotated agent certificate here (FR-ID-03).
    pub fn with_agent_cert_lifetime(mut self, lifetime: std::time::Duration) -> Self {
        self.agent_cert_lifetime =
            time::Duration::try_from(lifetime).unwrap_or(crate::pki::AGENT_CERT_LIFETIME);
        self
    }

    /// The lifetime of an agent certificate that the control plane issues (FR-ID-03).
    pub fn agent_cert_lifetime(&self) -> time::Duration {
        self.agent_cert_lifetime
    }

    /// The time between two rotation requests of one agent, answered or refused. A quarter of a
    /// short certificate lifetime shortens it (FR-ID-03).
    pub fn rotation_cooldown(&self) -> time::Duration {
        std::cmp::min(self.agent_cert_lifetime / 4, ROTATION_COOLDOWN)
    }

    /// The server passes the settings of the job queue here (FR-SC-08).
    pub fn with_queue(mut self, queue: QueueConfig) -> Self {
        self.queue = queue;
        self
    }

    /// The server passes the days for which the control plane keeps the log text. A finished job
    /// takes log and step writes for the same time (FR-LG-02).
    pub fn with_log_retention_days(mut self, days: u32) -> Self {
        self.log_retention_days = days;
        self
    }

    /// The server passes the alert sender here. It serves the test button of a channel and the
    /// link base of the messages (ADR-0036).
    pub fn with_sender(mut self, sender: crate::alerts::sender::Sender) -> Self {
        self.sender = Some(sender);
        self
    }

    /// Sets the time for which a metrics snapshot of a tenant is reused. A test sets zero.
    pub fn with_metrics_ttl(mut self, ttl: time::Duration) -> Self {
        self.metrics_cache = Arc::new(SnapshotCache::new(ttl));
        self
    }

    /// Sets the time for which the dashboard facts of a tenant are reused. A test sets zero.
    pub fn with_dashboard_ttl(mut self, ttl: time::Duration) -> Self {
        self.dashboard_cache = Arc::new(crate::dashboard::DashboardCache::new(ttl));
        self
    }

    pub fn dashboard_cache(&self) -> &crate::dashboard::DashboardCache {
        &self.dashboard_cache
    }

    /// Absent only in a test that does not set it.
    pub fn sender(&self) -> Option<&crate::alerts::sender::Sender> {
        self.sender.as_ref()
    }

    pub fn log_retention_days(&self) -> u32 {
        self.log_retention_days
    }

    pub fn progress_throttle(
        &self,
    ) -> &crate::progress_throttle::ProgressThrottle<crate::job_data::ProgressUpdate> {
        &self.progress_throttle
    }

    pub fn metrics_cache(&self) -> &SnapshotCache {
        &self.metrics_cache
    }

    pub fn db(&self) -> &Database {
        &self.db
    }

    pub fn trusted_proxies(&self) -> &TrustedProxies {
        &self.trusted_proxies
    }

    pub fn auth(&self) -> &AuthState {
        &self.auth
    }

    pub fn tenancy(&self) -> &TenancyConfig {
        &self.tenancy
    }

    /// The provider is absent only in a test that does not set it.
    pub fn kek(&self) -> Option<&dyn KekProvider> {
        self.kek.as_deref()
    }

    /// The certificate authority is absent only in a test that does not set it.
    pub fn ca(&self) -> Option<&CaStore> {
        self.ca.as_deref()
    }

    /// The job signing service is absent only in a test that does not set it.
    pub fn job_signing(&self) -> Option<&JobSigningService> {
        self.job_signing.as_ref()
    }

    /// The gRPC endpoint that the enrollment response reports to the agent.
    pub fn agent_endpoint(&self) -> Option<&str> {
        self.agent_endpoint.as_deref()
    }

    /// The in-process map of the live streams (ADR-0026).
    pub fn agent_registry(&self) -> &StreamRegistry {
        &self.agent_registry
    }

    pub fn queue_config(&self) -> &QueueConfig {
        &self.queue
    }

    /// Asks the dispatcher for a pass now. A manual run and a finished job call it.
    pub fn wake_dispatcher(&self) {
        self.dispatch_wake.notify_one();
    }

    /// The signal that the dispatcher loop waits on.
    pub fn dispatch_wake(&self) -> Arc<Notify> {
        Arc::clone(&self.dispatch_wake)
    }

    /// The shared registry, for the batch flush of the `last_seen_at` writes.
    pub fn agent_registry_shared(&self) -> Arc<StreamRegistry> {
        Arc::clone(&self.agent_registry)
    }
}

/// The router registers one route for each entry of [`ROUTES`], so the routes table, the router and
/// the OpenAPI document cannot drift apart.
pub fn router(state: AppState) -> Router {
    let (router, _document) = openapi_router().split_for_parts();
    router
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, hide_platform_routes))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(middleware::from_fn(finalize_problem))
        .layer(TraceLayer::new_for_http().make_span_with(RequestSpan))
        .layer(PropagateRequestIdLayer::new(request_id_header()))
        .layer(SetRequestIdLayer::new(request_id_header(), MakeRequestIdV7))
}

fn openapi_router() -> OpenApiRouter<AppState> {
    let mut router = OpenApiRouter::with_openapi(base_openapi());
    for route in ROUTES {
        router = router.routes((route.handler)());
    }
    router
}

/// Hides the platform routes in single mode (ADR-0024).
///
/// The function is one guard for every platform route: the access rule of the route table decides
/// which path is a platform path. The response is the same 404 `not_found` as an unknown path, so
/// the mode reveals no platform route. The guard runs before the handler, so it also covers an
/// unauthenticated request.
async fn hide_platform_routes(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if state.tenancy().mode.is_single() && crate::routes::is_platform_path(request.uri().path()) {
        return ApiError::not_found().into_response();
    }
    next.run(request).await
}

pub fn openapi_document() -> OpenApiDocument {
    static DOCUMENT: OnceLock<OpenApiDocument> = OnceLock::new();
    DOCUMENT
        .get_or_init(|| openapi_router().into_openapi())
        .clone()
}

fn base_openapi() -> OpenApiDocument {
    let mut document = utoipa::openapi::OpenApiBuilder::new()
        .info(
            utoipa::openapi::InfoBuilder::new()
                .title("ResticHub API")
                .version(restichub_common::BUILD_VERSION)
                .description(Some(env!("CARGO_PKG_DESCRIPTION")))
                .license(Some(
                    utoipa::openapi::LicenseBuilder::new()
                        .name("Apache-2.0")
                        .identifier(Some("Apache-2.0"))
                        .build(),
                ))
                .build(),
        )
        .build();
    SecurityAddon.modify(&mut document);
    document
}

/// Adds the request ID to every problem response and logs the internal cause once.
///
/// The extractors and the handlers build an [`ApiError`] with no request ID. The mapper reads the
/// request ID from the request extension and writes it into the body. It logs the cause of an
/// internal error with the same request ID, so the cause is not lost.
async fn finalize_problem(request: Request, next: Next) -> Response {
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .and_then(|id| id.header_value().to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let mut response = next.run(request).await;
    if let Some(cause) = response.extensions_mut().remove::<InternalCause>() {
        tracing::error!(request_id = %request_id, error = %cause.0, "internal error");
    }
    if is_problem(&response) {
        inject_request_id(response, &request_id).await
    } else {
        response
    }
}

fn is_problem(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        == Some(PROBLEM_CONTENT_TYPE)
}

async fn inject_request_id(response: Response, request_id: &str) -> Response {
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = body.collect().await.map(|body| body.to_bytes()) else {
        return Response::from_parts(parts, axum::body::Body::empty());
    };
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Response::from_parts(parts, axum::body::Body::from(bytes));
    };
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "request_id".to_owned(),
            serde_json::Value::String(request_id.to_owned()),
        );
    }
    let Ok(new_bytes) = serde_json::to_vec(&value) else {
        return Response::from_parts(parts, axum::body::Body::from(bytes));
    };
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, axum::body::Body::from(new_bytes))
}

/// Liveness probe. The process is alive. The function does not touch the database.
#[utoipa::path(
    get,
    path = "/healthz",
    responses((status = 200, description = "The process is alive."))
)]
pub(crate) async fn healthz() -> &'static str {
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
pub(crate) async fn readyz(State(state): State<AppState>) -> Response {
    if state.db.is_ready().await {
        StatusCode::OK.into_response()
    } else {
        ApiError::service_unavailable().into_response()
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/openapi.json",
    summary = "The OpenAPI document.",
    responses((status = 200, description = "The OpenAPI document."))
)]
pub(crate) async fn openapi() -> impl IntoResponse {
    Json(openapi_document())
}

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

/// The fallback handler for a path with no route.
///
/// An unknown `/api/...` path keeps the RFC 9457 problem. The comparison ignores the letter case
/// and the repeated slashes, so `/API/v1/x` and `//api/v1/x` cannot reach the UI. A method other
/// than GET and HEAD never serves the UI either. Every other path serves the embedded web UI, so
/// the single-page application router handles the route. The UI never shadows a registered route.
async fn not_found(method: Method, uri: Uri) -> Response {
    if is_api_path(uri.path()) {
        return ApiError::not_found().into_response();
    }
    if method != Method::GET && method != Method::HEAD {
        return ApiError::method_not_allowed().into_response();
    }
    crate::ui::serve(uri)
}

/// The function collapses the repeated slashes and ignores the letter case.
fn is_api_path(path: &str) -> bool {
    let mut collapsed = String::with_capacity(path.len());
    let mut previous_slash = false;
    for character in path.chars() {
        if character == '/' {
            if previous_slash {
                continue;
            }
            previous_slash = true;
        } else {
            previous_slash = false;
        }
        collapsed.push(character);
    }
    let lower = collapsed.to_ascii_lowercase();
    lower == "/api" || lower.starts_with("/api/")
}

async fn method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}

#[derive(Debug, Clone, Copy)]
struct MakeRequestIdV7;

impl MakeRequestId for MakeRequestIdV7 {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        let value = HeaderValue::from_str(&Uuid::now_v7().to_string()).ok()?;
        Some(RequestId::new(value))
    }
}

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

#[cfg(test)]
mod tests {
    use super::is_api_path;

    #[test]
    fn the_api_namespace_is_recognized() {
        assert!(is_api_path("/api"));
        assert!(is_api_path("/api/"));
        assert!(is_api_path("/api/v1/users"));
        // Repeated slashes are collapsed.
        assert!(is_api_path("//api/v1/x"));
        assert!(is_api_path("/api//v1/x"));
        assert!(is_api_path("///api"));
        // Letter case is ignored.
        assert!(is_api_path("/API/v1/x"));
        assert!(is_api_path("/Api"));
    }

    #[test]
    fn a_ui_path_is_not_the_api_namespace() {
        assert!(!is_api_path("/"));
        assert!(!is_api_path("/users"));
        assert!(!is_api_path("/api-docs"));
        assert!(!is_api_path("/apix"));
        assert!(!is_api_path("/platform/tenants"));
    }
}
