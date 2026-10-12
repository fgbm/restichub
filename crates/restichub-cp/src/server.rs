//! The HTTP server of the control plane.

use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode, request::Parts};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use restichub_db::Database;
use tower_http::request_id::{
    MakeRequestId, PropagateRequestIdLayer, RequestId, SetRequestIdLayer,
};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{MakeSpan, TraceLayer};
use tracing::Span;
use utoipa::OpenApi;
use uuid::Uuid;

use crate::error::ApiError;

const REQUEST_ID_HEADER: &str = "x-request-id";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The application state of the HTTP server.
#[derive(Debug, Clone)]
pub struct AppState {
    db: Database,
}

impl AppState {
    /// Creates the application state.
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

/// Builds the HTTP router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api/v1/openapi.json", get(openapi))
        .fallback(not_found)
        .with_state(state)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(TraceLayer::new_for_http().make_span_with(RequestSpan))
        .layer(PropagateRequestIdLayer::new(request_id_header()))
        .layer(SetRequestIdLayer::new(request_id_header(), MakeRequestIdV7))
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
    paths(healthz, readyz, openapi)
)]
struct ApiDoc;

/// The fallback handler for an unknown route.
async fn not_found(request_id: RequestIdHeader) -> ApiError {
    ApiError::not_found(&request_id.0)
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
