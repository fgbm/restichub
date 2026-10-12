//! The API error type (ADR-0023, section Errors).
//!
//! The control plane returns an error as an RFC 9457 `application/problem+json` response. The
//! body holds a stable `code` in snake_case and the request ID. The body holds no internal
//! detail. The control plane logs the internal detail with the request ID.

use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// An API error.
#[derive(Debug, Clone)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    title: &'static str,
    request_id: String,
    /// The `Retry-After` header value in seconds. Only `rate_limited` sets it.
    retry_after: Option<u64>,
}

impl ApiError {
    /// Creates an error with no `Retry-After` header.
    fn new(status: StatusCode, code: &'static str, title: &'static str, request_id: &str) -> Self {
        Self {
            status,
            code,
            title,
            request_id: request_id.to_owned(),
            retry_after: None,
        }
    }

    /// Creates a `404 not_found` error.
    pub fn not_found(request_id: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "Not Found", request_id)
    }

    /// Creates a `503 service_unavailable` error.
    pub fn service_unavailable(request_id: &str) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
            "Service Unavailable",
            request_id,
        )
    }

    /// Creates a `500 internal_error` error.
    ///
    /// The function logs the cause with the request ID. The response body holds no internal
    /// detail.
    pub fn internal(request_id: &str, error: &dyn std::error::Error) -> Self {
        tracing::error!(request_id = %request_id, error = %error, "internal error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal Server Error",
            request_id,
        )
    }

    /// Creates a `500 internal_error` error with no cause.
    pub fn internal_error(request_id: &str) -> Self {
        tracing::error!(request_id = %request_id, "internal error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal Server Error",
            request_id,
        )
    }

    /// Creates a `401 unauthenticated` error.
    pub fn unauthenticated(request_id: &str) -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "Unauthorized",
            request_id,
        )
    }

    /// Creates a `401 invalid_credentials` error.
    ///
    /// Sign-in returns this error for every failure: an unknown tenant, an unknown user, a wrong
    /// password, a disabled user and a suspended tenant. The control plane logs the real reason
    /// with the request ID and never puts it in the response.
    pub fn invalid_credentials(request_id: &str) -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "Unauthorized",
            request_id,
        )
    }

    /// Creates a `403 forbidden` error.
    pub fn forbidden(request_id: &str) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", "Forbidden", request_id)
    }

    /// Creates a `403 csrf_required` error.
    ///
    /// A cookie-authenticated unsafe request needs the header `X-ResticHub-CSRF: 1`.
    pub fn csrf_required(request_id: &str) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "csrf_required",
            "Forbidden",
            request_id,
        )
    }

    /// Creates a `409 conflict` error.
    pub fn conflict(request_id: &str) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", "Conflict", request_id)
    }

    /// Creates a `400 invalid_argument` error.
    pub fn invalid_argument(request_id: &str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "invalid_argument",
            "Bad Request",
            request_id,
        )
    }

    /// Creates a `400 ambiguous_credentials` error.
    ///
    /// A request holds a session cookie and a bearer token at the same time.
    pub fn ambiguous_credentials(request_id: &str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "ambiguous_credentials",
            "Bad Request",
            request_id,
        )
    }

    /// Creates a `405 method_not_allowed` error.
    pub fn method_not_allowed(request_id: &str) -> Self {
        Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "Method Not Allowed",
            request_id,
        )
    }

    /// Creates a `429 rate_limited` error with a `Retry-After` header.
    pub fn rate_limited(request_id: &str, retry_after: u64) -> Self {
        let mut error = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too Many Requests",
            request_id,
        );
        error.retry_after = Some(retry_after.clamp(1, MAX_RETRY_AFTER));
        error
    }

    /// Creates a `503 busy` error with a `Retry-After` header.
    ///
    /// The control plane returns this error when the password hash semaphore is saturated.
    pub fn busy(request_id: &str, retry_after: u64) -> Self {
        let mut error = Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "Service Unavailable",
            request_id,
        );
        error.retry_after = Some(retry_after.clamp(1, MAX_RETRY_AFTER));
        error
    }

    /// Builds an error from a password operation failure.
    ///
    /// A saturated hash semaphore becomes `503 busy`. Every other failure is internal.
    pub fn from_password(request_id: &str, error: &crate::password::PasswordError) -> Self {
        match error {
            crate::password::PasswordError::Busy => {
                Self::busy(request_id, crate::password::HASH_WAIT.as_secs())
            }
            other => Self::internal(request_id, other),
        }
    }
}

/// The largest `Retry-After` value in seconds.
const MAX_RETRY_AFTER: u64 = 3600;

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ProblemBody {
            type_: "about:blank".to_owned(),
            title: self.title.to_owned(),
            status: self.status.as_u16(),
            code: self.code.to_owned(),
            request_id: self.request_id,
        };
        let mut response = (
            self.status,
            [(header::CONTENT_TYPE, "application/problem+json")],
            Json(body),
        )
            .into_response();
        if let Some(retry_after) = self.retry_after
            && let Ok(value) = header::HeaderValue::from_str(&retry_after.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
    }
}

/// The body of an RFC 9457 problem response.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProblemBody {
    /// The type of the problem.
    #[serde(rename = "type")]
    pub type_: String,
    /// The title of the problem.
    pub title: String,
    /// The HTTP status code.
    pub status: u16,
    /// The stable code in snake_case.
    pub code: String,
    /// The request ID of the request.
    pub request_id: String,
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    use super::ApiError;

    #[tokio::test]
    async fn the_problem_body_holds_the_code_and_the_request_id() {
        let response = ApiError::not_found("request-1").into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        assert_eq!(content_type.as_deref(), Some("application/problem+json"));
        let body = response
            .into_body()
            .collect()
            .await
            .map(|body| body.to_bytes())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        let code = body
            .as_ref()
            .and_then(|value| value.get("code"))
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        let request_id = body
            .as_ref()
            .and_then(|value| value.get("request_id"))
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        assert_eq!(code.as_deref(), Some("not_found"));
        assert_eq!(request_id.as_deref(), Some("request-1"));
    }
}
