//! The API error type (ADR-0023, section Errors).
//!
//! The control plane returns an error as an RFC 9457 `application/problem+json` response. The
//! body holds a stable `code` in snake_case and the request ID. The body holds no internal detail.
//!
//! The error carries no request ID: the extractors build an error before the request reaches a
//! handler, and the request ID lives in the request extension. The response mapper of the server
//! adds the request ID to the body and logs the internal cause once, where the request ID is
//! known.

use std::error::Error as StdError;

use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use restichub_db::DbError;
use restichub_secrets::{CryptoError, KekError};
use serde::Serialize;
use utoipa::ToSchema;

use crate::password::PasswordError;
use crate::token::TokenError;
use crate::validation::{Reason, ValidationError};

pub const PROBLEM_CONTENT_TYPE: &str = "application/problem+json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    ServiceUnavailable,
    InternalError,
    Unauthenticated,
    InvalidCredentials,
    Forbidden,
    CsrfRequired,
    Conflict,
    InvalidArgument,
    PayloadTooLarge,
    TenantKeyMissing,
    AmbiguousCredentials,
    MethodNotAllowed,
    RateLimited,
    Busy,
    SharedAccount,
    UsernameTaken,
    EmailTaken,
    AgentOffline,
    AlreadyQueued,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::ServiceUnavailable => "service_unavailable",
            Self::InternalError => "internal_error",
            Self::Unauthenticated => "unauthenticated",
            Self::InvalidCredentials => "invalid_credentials",
            Self::Forbidden => "forbidden",
            Self::CsrfRequired => "csrf_required",
            Self::Conflict => "conflict",
            Self::InvalidArgument => "invalid_argument",
            Self::PayloadTooLarge => "payload_too_large",
            Self::TenantKeyMissing => "tenant_key_missing",
            Self::AmbiguousCredentials => "ambiguous_credentials",
            Self::MethodNotAllowed => "method_not_allowed",
            Self::RateLimited => "rate_limited",
            Self::Busy => "busy",
            Self::SharedAccount => "shared_account",
            Self::UsernameTaken => "username_taken",
            Self::EmailTaken => "email_taken",
            Self::AgentOffline => "agent_offline",
            Self::AlreadyQueued => "already_queued",
        }
    }
}

/// The type holds no request ID. The response mapper of the server adds the request ID to the body
/// and logs the internal cause once.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: ErrorCode,
    title: &'static str,
    field: Option<String>,
    reason: Option<Reason>,
    /// The `Retry-After` header value in seconds. Only `rate_limited` and `busy` set it.
    retry_after: Option<u64>,
    /// The internal detail. The response body never holds it; the mapper logs it with the request
    /// ID. The value holds no secret: every constructor takes a message, never a value.
    cause: Option<String>,
}

impl ApiError {
    fn new(status: StatusCode, code: ErrorCode, title: &'static str) -> Self {
        Self {
            status,
            code,
            title,
            field: None,
            reason: None,
            retry_after: None,
            cause: None,
        }
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "Not Found")
    }

    pub fn service_unavailable() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::ServiceUnavailable,
            "Service Unavailable",
        )
    }

    /// Creates a `500 internal_error` error with a cause.
    ///
    /// The response mapper logs the cause with the request ID. The body holds no internal detail.
    pub fn internal(cause: impl Into<String>) -> Self {
        let mut error = Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::InternalError,
            "Internal Server Error",
        );
        error.cause = Some(cause.into());
        error
    }

    pub fn unauthenticated() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthenticated,
            "Unauthorized",
        )
    }

    /// Sign-in returns this error for every failure: an unknown tenant, an unknown user, a wrong
    /// password, a disabled user and a suspended tenant. The control plane logs the real reason and
    /// never puts it in the response.
    pub fn invalid_credentials() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::InvalidCredentials,
            "Unauthorized",
        )
    }

    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, ErrorCode::Forbidden, "Forbidden")
    }

    /// A cookie-authenticated unsafe request needs the header `X-ResticHub-CSRF: 1`.
    pub fn csrf_required() -> Self {
        Self::new(StatusCode::FORBIDDEN, ErrorCode::CsrfRequired, "Forbidden")
    }

    pub fn conflict() -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::Conflict, "Conflict")
    }

    /// Creates a `400 invalid_argument` error for a field.
    ///
    /// The field name is a stable identifier of the request schema, not user input.
    pub fn invalid_argument(field: impl Into<String>) -> Self {
        let mut error = Self::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidArgument,
            "Bad Request",
        );
        error.field = Some(field.into());
        error
    }

    pub fn invalid_field(field: impl Into<String>, reason: Reason) -> Self {
        let mut error = Self::invalid_argument(field);
        error.reason = Some(reason);
        error
    }

    /// Creates a `400 invalid_argument` error for a field, with the cause of the failure.
    ///
    /// The cause is the message of a parse failure. The response mapper logs it with the request
    /// ID; the body never holds it.
    pub fn invalid_field_cause(
        field: impl Into<String>,
        reason: Reason,
        cause: impl Into<String>,
    ) -> Self {
        let mut error = Self::invalid_field(field, reason);
        error.cause = Some(cause.into());
        error
    }

    /// A request body that is larger than the body limit of the route gives this error. The
    /// response holds no parser detail.
    pub fn payload_too_large() -> Self {
        Self::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::PayloadTooLarge,
            "Payload Too Large",
        )
    }

    /// The tenant holds no active key row, so the control plane cannot encrypt a secret.
    pub fn tenant_key_missing() -> Self {
        let mut error = Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::TenantKeyMissing,
            "Internal Server Error",
        );
        error.cause = Some("the tenant holds no active key row".to_owned());
        error
    }

    /// A request holds a session cookie and a bearer token at the same time.
    pub fn ambiguous_credentials() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::AmbiguousCredentials,
            "Bad Request",
        )
    }

    pub fn method_not_allowed() -> Self {
        Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            ErrorCode::MethodNotAllowed,
            "Method Not Allowed",
        )
    }

    pub fn rate_limited(retry_after: u64) -> Self {
        let mut error = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::RateLimited,
            "Too Many Requests",
        );
        error.retry_after = Some(retry_after.clamp(1, MAX_RETRY_AFTER));
        error
    }

    /// The control plane returns this error when the password hash semaphore is saturated.
    pub fn busy(retry_after: u64) -> Self {
        let mut error = Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Busy,
            "Service Unavailable",
        );
        error.retry_after = Some(retry_after.clamp(1, MAX_RETRY_AFTER));
        error
    }

    /// The account has an active membership in another tenant. Only the user or a Platform Admin
    /// changes its password or its display name (ADR-0025).
    pub fn shared_account() -> Self {
        Self::new(StatusCode::FORBIDDEN, ErrorCode::SharedAccount, "Forbidden")
    }

    /// The username exists in the installation. The create endpoint of a tenant Admin answers with
    /// it (ADR-0025).
    pub fn username_taken() -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::UsernameTaken, "Conflict")
    }

    /// The same, with the request field that holds the taken name.
    pub fn username_taken_field(field: impl Into<String>) -> Self {
        let mut error = Self::username_taken();
        error.field = Some(field.into());
        error
    }

    /// The email exists in the installation. The platform account create and update answer with it
    /// (ADR-0028).
    pub fn email_taken() -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::EmailTaken, "Conflict")
    }

    /// The target agent holds no open stream, so the control plane refuses an on-demand job at
    /// once. The MVP has no queue for an on-demand job, so it does not hold the job for a later
    /// delivery. The user interface translates the code (NFR-DOC-04).
    pub fn agent_offline() -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::AgentOffline, "Conflict")
    }

    /// Creates a `409 already_queued` error: the plan already holds a queued manual run.
    pub fn already_queued() -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::AlreadyQueued, "Conflict")
    }
}

/// In seconds.
const MAX_RETRY_AFTER: u64 = 3600;

/// Returns the full cause chain of an error as one message.
///
/// The function keeps every source, so a wrapped database error is not lost.
fn cause_chain(error: &dyn StdError) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(error) = source {
        message.push_str(": ");
        message.push_str(&error.to_string());
        source = error.source();
    }
    message
}

impl From<DbError> for ApiError {
    fn from(error: DbError) -> Self {
        Self::internal(cause_chain(&error))
    }
}

impl From<CryptoError> for ApiError {
    fn from(error: CryptoError) -> Self {
        Self::internal(cause_chain(&error))
    }
}

impl From<crate::job_signing::JobSigningError> for ApiError {
    fn from(error: crate::job_signing::JobSigningError) -> Self {
        Self::internal(cause_chain(&error))
    }
}

impl From<crate::delivery::DeliveryError> for ApiError {
    fn from(error: crate::delivery::DeliveryError) -> Self {
        match error {
            crate::delivery::DeliveryError::NotFound => {
                Self::internal("the repository binds a secret that does not exist")
            }
            other => Self::internal(cause_chain(&other)),
        }
    }
}

impl From<KekError> for ApiError {
    fn from(error: KekError) -> Self {
        Self::internal(cause_chain(&error))
    }
}

impl From<TokenError> for ApiError {
    fn from(error: TokenError) -> Self {
        Self::internal(cause_chain(&error))
    }
}

impl From<ValidationError> for ApiError {
    fn from(error: ValidationError) -> Self {
        Self::invalid_field(error.field, error.reason)
    }
}

impl From<PasswordError> for ApiError {
    fn from(error: PasswordError) -> Self {
        match error {
            PasswordError::Busy => Self::busy(crate::password::HASH_WAIT.as_secs()),
            other => Self::internal(cause_chain(&other)),
        }
    }
}

/// A duplicate value is a conflict. A foreign key violation is a conflict too: the referenced row
/// is absent, so the caller must correct the request. Every other error is internal, with the cause
/// logged.
pub fn map_write_error(error: DbError) -> ApiError {
    if error.is_unique_violation() || error.is_foreign_key_violation() {
        ApiError::conflict()
    } else {
        ApiError::from(error)
    }
}

/// The internal cause of a response, read by the response mapper.
///
/// The type lives in the response extension, so the mapper logs the cause once, with the request
/// ID. The body never holds it.
#[derive(Debug, Clone)]
pub struct InternalCause(pub String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ProblemBody {
            type_: "about:blank".to_owned(),
            title: self.title.to_owned(),
            status: self.status.as_u16(),
            code: self.code,
            // The response mapper fills the request ID from the request extension.
            request_id: String::new(),
            field: self.field,
            reason: self.reason.map(|reason| reason.as_str().to_owned()),
        };
        let mut response = (
            self.status,
            [(header::CONTENT_TYPE, PROBLEM_CONTENT_TYPE)],
            Json(body),
        )
            .into_response();
        if let Some(retry_after) = self.retry_after
            && let Ok(value) = header::HeaderValue::from_str(&retry_after.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        if let Some(cause) = self.cause {
            response.extensions_mut().insert(InternalCause(cause));
        }
        response
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProblemBody {
    #[serde(rename = "type")]
    pub type_: String,
    pub title: String,
    pub status: u16,
    /// In snake_case.
    pub code: ErrorCode,
    pub request_id: String,
    /// Only `invalid_argument` sets it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// In snake_case; only `invalid_argument` sets it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The shared responses of a protected route: 401 and 403 with the problem body.
///
/// A handler uses the type in its `#[utoipa::path]` attribute, so the two responses have one
/// definition.
#[derive(utoipa::IntoResponses)]
pub enum ProtectedResponses {
    #[response(status = 401, description = "The request is not authenticated.")]
    Unauthenticated(ProblemBody),
    #[response(status = 403, description = "The role is not sufficient.")]
    Forbidden(ProblemBody),
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    use super::{ApiError, ErrorCode};
    use crate::validation::Reason;

    #[tokio::test]
    async fn the_problem_body_holds_the_code_and_the_field() {
        let response = ApiError::invalid_field("username", Reason::TooLong).into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
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
        let field = body
            .as_ref()
            .and_then(|value| value.get("field"))
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        assert_eq!(code.as_deref(), Some(ErrorCode::InvalidArgument.as_str()));
        assert_eq!(field.as_deref(), Some("username"));
    }
}
