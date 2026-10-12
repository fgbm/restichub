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
}

impl ApiError {
    /// Creates a `404 not_found` error.
    pub fn not_found(request_id: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            title: "Not Found",
            request_id: request_id.to_owned(),
        }
    }

    /// Creates a `503 service_unavailable` error.
    pub fn service_unavailable(request_id: &str) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "service_unavailable",
            title: "Service Unavailable",
            request_id: request_id.to_owned(),
        }
    }

    /// Creates a `500 internal_error` error.
    ///
    /// The function logs the cause with the request ID. The response body holds no internal
    /// detail.
    pub fn internal(request_id: &str, error: &dyn std::error::Error) -> Self {
        tracing::error!(request_id = %request_id, error = %error, "internal error");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            title: "Internal Server Error",
            request_id: request_id.to_owned(),
        }
    }

    /// Creates a `500 internal_error` error with no cause.
    pub fn internal_error(request_id: &str) -> Self {
        tracing::error!(request_id = %request_id, "internal error");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            title: "Internal Server Error",
            request_id: request_id.to_owned(),
        }
    }

    /// Creates a `401 unauthenticated` error.
    pub fn unauthenticated(request_id: &str) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthenticated",
            title: "Unauthorized",
            request_id: request_id.to_owned(),
        }
    }

    /// Creates a `403 forbidden` error.
    pub fn forbidden(request_id: &str) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "forbidden",
            title: "Forbidden",
            request_id: request_id.to_owned(),
        }
    }

    /// Creates a `400 invalid_argument` error.
    pub fn invalid_argument(request_id: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_argument",
            title: "Bad Request",
            request_id: request_id.to_owned(),
        }
    }

    /// Creates a `405 method_not_allowed` error.
    pub fn method_not_allowed(request_id: &str) -> Self {
        Self {
            status: StatusCode::METHOD_NOT_ALLOWED,
            code: "method_not_allowed",
            title: "Method Not Allowed",
            request_id: request_id.to_owned(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Problem {
            type_: "about:blank".to_owned(),
            title: self.title.to_owned(),
            status: self.status.as_u16(),
            code: self.code.to_owned(),
            request_id: self.request_id,
        };
        (
            self.status,
            [(header::CONTENT_TYPE, "application/problem+json")],
            Json(body),
        )
            .into_response()
    }
}

/// The body of an RFC 9457 problem response.
#[derive(Debug, Serialize)]
struct Problem {
    #[serde(rename = "type")]
    type_: String,
    title: String,
    status: u16,
    code: String,
    request_id: String,
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
