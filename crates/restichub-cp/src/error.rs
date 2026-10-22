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
use restichub_db::users::AccountField;
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
    AgentCapabilityMissing,
    RestoreTargetDenied,
    TokenUsed,
    TokenAlreadyRevoked,
    TokenExpired,
    AgentRecentlyOnline,
    SetupAlreadyDone,
    InvalidSetupToken,
    RepositoryHostLocked,
    NotRepositoryHost,
    RepositoryInUse,
    RepositoryHasActiveJob,
    ProfileInUse,
    SecretInUse,
    LastAdmin,
    OwnAccountProtected,
    PlatformTenantProtected,
    RepositoryLocationTaken,
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
            Self::AgentCapabilityMissing => "agent_capability_missing",
            Self::RestoreTargetDenied => "restore_target_denied",
            Self::TokenUsed => "token_used",
            Self::TokenAlreadyRevoked => "token_already_revoked",
            Self::TokenExpired => "token_expired",
            Self::AgentRecentlyOnline => "agent_recently_online",
            Self::SetupAlreadyDone => "setup_already_done",
            Self::InvalidSetupToken => "invalid_setup_token",
            Self::RepositoryHostLocked => "repository_host_locked",
            Self::NotRepositoryHost => "not_repository_host",
            Self::RepositoryInUse => "repository_in_use",
            Self::RepositoryHasActiveJob => "repository_has_active_job",
            Self::ProfileInUse => "profile_in_use",
            Self::SecretInUse => "secret_in_use",
            Self::LastAdmin => "last_admin",
            Self::OwnAccountProtected => "own_account_protected",
            Self::PlatformTenantProtected => "platform_tenant_protected",
            Self::RepositoryLocationTaken => "repository_location_taken",
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
    /// The name of the repository that holds the location. Only `repository_location_taken` sets it.
    existing_name: Option<String>,
    /// The `Retry-After` header value in seconds. Set by `rate_limited`, `busy` and a timeout.
    retry_after: Option<u64>,
    // Boxed so that `Result<_, ApiError>` stays below the `result_large_err` limit.
    records: Option<Box<Records>>,
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
            existing_name: None,
            retry_after: None,
            records: None,
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

    /// A `503 service_unavailable` error that tells the client to try again after the given
    /// seconds.
    pub fn unavailable_retry(retry_after: u64) -> Self {
        let mut error = Self::service_unavailable();
        error.retry_after = Some(retry_after.clamp(1, MAX_RETRY_AFTER));
        error
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

    pub fn is_unauthenticated(&self) -> bool {
        self.status == StatusCode::UNAUTHORIZED
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

    /// The username or the email exists in the installation (ADR-0025, ADR-0028).
    ///
    /// `field` names the request field of the taken value, so a form marks only that input.
    pub fn taken(field: AccountField) -> Self {
        let (code, name) = match field {
            AccountField::Username => (ErrorCode::UsernameTaken, "username"),
            AccountField::Email => (ErrorCode::EmailTaken, "email"),
        };
        let mut error = Self::new(StatusCode::CONFLICT, code, "Conflict");
        error.field = Some(name.to_owned());
        error
    }

    /// A write of an account row that violates a unique index is HTTP 409 for the field of that
    /// index. A concurrent write gets there after the check of the caller.
    pub fn account_write(error: DbError) -> Self {
        match AccountField::of_unique_violation(&error) {
            Some(field) => Self::taken(field),
            None => Self::from(error),
        }
    }

    /// The target agent holds no open stream, so the control plane refuses an on-demand job at
    /// once. The MVP has no queue for an on-demand job, so it does not hold the job for a later
    /// delivery. The user interface translates the code (NFR-DOC-04).
    pub fn agent_offline() -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::AgentOffline, "Conflict")
    }

    /// Creates a `409 agent_capability_missing` error: the agent is older than the step kind of
    /// the job (FR-PR-09). The user interface translates the code (NFR-DOC-04).
    pub fn agent_capability_missing() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::AgentCapabilityMissing,
            "Conflict",
        )
    }

    /// Creates a `409 restore_target_denied` error: the target is outside the restore roots of the
    /// agent (FR-RS-03). The user interface translates the code (NFR-DOC-04).
    pub fn restore_target_denied() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::RestoreTargetDenied,
            "Conflict",
        )
    }

    /// Creates a `409 setup_already_done` error: the first-run setup is finished. The user
    /// interface translates the code (NFR-DOC-04).
    pub fn setup_already_done() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::SetupAlreadyDone,
            "Conflict",
        )
    }

    /// Creates a `401 invalid_setup_token` error: the setup token is unknown, used or expired.
    pub fn invalid_setup_token() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::InvalidSetupToken,
            "Unauthorized",
        )
    }

    /// Creates a `409 token_used` error: a host already used the enrollment token. The user
    /// interface translates the code (NFR-DOC-04).
    pub fn token_used() -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::TokenUsed, "Conflict")
    }

    /// Creates a `409 token_already_revoked` error: an Admin revoked the enrollment token before.
    pub fn token_already_revoked() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::TokenAlreadyRevoked,
            "Conflict",
        )
    }

    /// Creates a `409 token_expired` error: the enrollment token is past its time to live.
    pub fn token_expired() -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::TokenExpired, "Conflict")
    }

    /// Creates a `409 agent_recently_online` error: the old record of a replacement had a sign of
    /// life in the online window (FR-ID-11).
    pub fn agent_recently_online() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::AgentRecentlyOnline,
            "Conflict",
        )
    }

    /// Creates a `409 repository_host_locked` error on `maintenance_agent_id`: the data of an
    /// initialized local repository lives on the disk of its host, so the host cannot change.
    pub fn repository_host_locked() -> Self {
        let mut error = Self::new(
            StatusCode::CONFLICT,
            ErrorCode::RepositoryHostLocked,
            "Conflict",
        );
        error.field = Some("maintenance_agent_id".to_owned());
        error
    }

    /// Creates a `409 repository_location_taken` error on a location field: another repository of
    /// the tenant holds the location (FR-RP-11).
    pub fn repository_location_taken(field: &'static str, existing_name: String) -> Self {
        let mut error = Self::new(
            StatusCode::CONFLICT,
            ErrorCode::RepositoryLocationTaken,
            "Conflict",
        );
        error.field = Some(field.to_owned());
        error.existing_name = Some(existing_name);
        error
    }

    /// Creates a `409 not_repository_host` error: the plan runs on another host than its local
    /// repository, so a job cannot reach the path.
    pub fn not_repository_host() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::NotRepositoryHost,
            "Conflict",
        )
    }

    /// Creates a `409` error that names the records which block a delete. The user interface
    /// translates the code and links the records (NFR-DOC-04).
    pub fn blocked(code: ErrorCode, blockers: Blockers) -> Self {
        let mut error = Self::new(StatusCode::CONFLICT, code, "Conflict");
        error.records = Some(Box::new(Records {
            blockers: Some(blockers),
            parameters: Vec::new(),
        }));
        error
    }

    /// Creates a `409` error for a protected record: no list of records explains it.
    pub fn protected(code: ErrorCode) -> Self {
        Self::new(StatusCode::CONFLICT, code, "Conflict")
    }

    /// Creates a `400` error with the names of the parameters that the request lacks (FR-PF-11).
    pub fn parameters(field: impl Into<String>, reason: Reason, names: Vec<String>) -> Self {
        let mut error = Self::invalid_field(field, reason);
        error.records = Some(Box::new(Records {
            blockers: None,
            parameters: names,
        }));
        error
    }

    /// Creates a `400 parameter_kind_mismatch` error for a name that another profile of the same
    /// host declares with another kind (FR-PF-11, ADR-0047).
    ///
    /// The body names the parameter; the cause, which only the log holds, names the other profile.
    pub fn parameter_kind_conflict(field: impl Into<String>, name: String, other: &str) -> Self {
        let mut error = Self::parameters(field, Reason::ParameterKindMismatch, vec![name.clone()]);
        error.cause = Some(format!(
            "the profile {other} declares the parameter {name} with another kind"
        ));
        error
    }

    /// Creates a `400 parameter_missing_in_plans` error with the names of the parameters and the
    /// plans that a change would leave without a required value (FR-PF-11).
    pub fn parameters_in_plans(reason: Reason, names: Vec<String>, blockers: Blockers) -> Self {
        let mut error = Self::parameters("parameters", reason, names);
        if let Some(records) = error.records.as_mut() {
            records.blockers = Some(blockers);
        }
        error
    }

    /// Creates a `409 already_queued` error: the plan already holds a queued manual run.
    pub fn already_queued() -> Self {
        Self::new(StatusCode::CONFLICT, ErrorCode::AlreadyQueued, "Conflict")
    }
}

/// The records and the parameter names of a refusal, boxed together.
#[derive(Debug)]
struct Records {
    blockers: Option<Blockers>,
    parameters: Vec<String>,
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

/// In seconds.
const STATEMENT_TIMEOUT_RETRY_AFTER: u64 = 5;

impl From<DbError> for ApiError {
    fn from(error: DbError) -> Self {
        if error.is_statement_timeout() {
            tracing::warn!(cause = %cause_chain(&error), "a request query passed its statement timeout");
            return Self::unavailable_retry(STATEMENT_TIMEOUT_RETRY_AFTER);
        }
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
        match error {
            crate::job_signing::JobSigningError::StaleCurrentKey => Self::conflict(),
            other => Self::internal(cause_chain(&other)),
        }
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
    fn into_response(mut self) -> Response {
        let body = ProblemBody {
            type_: "about:blank".to_owned(),
            title: self.title.to_owned(),
            status: self.status.as_u16(),
            code: self.code,
            // The response mapper fills the request ID from the request extension.
            request_id: String::new(),
            field: self.field,
            reason: self.reason.map(|reason| reason.as_str().to_owned()),
            blockers: self
                .records
                .as_mut()
                .and_then(|records| records.blockers.take()),
            parameters: self
                .records
                .as_mut()
                .filter(|records| !records.parameters.is_empty())
                .map(|records| std::mem::take(&mut records.parameters)),
            existing_name: self.existing_name,
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
    /// Set by `invalid_argument`, `username_taken` and `email_taken`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// In snake_case; only `invalid_argument` sets it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The records that block a delete, and the plans that a change would leave without a required
    /// parameter. Only the `*_in_use` codes, `repository_has_active_job` and
    /// `parameter_missing_in_plans` set it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blockers: Option<Blockers>,
    /// Only a validation refusal sets it: `parameter_missing`, `parameter_kind_mismatch`,
    /// `parameter_not_declared` and `parameter_needs_default` (FR-PF-05, FR-PF-11); the last two
    /// list one name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Vec<String>>,
    /// The name of the repository that holds the location; only `repository_location_taken` sets
    /// it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlockerKind {
    Repository,
    Channel,
    Profile,
    JobTemplate,
    Plan,
    Job,
    /// A host that holds a parameter value (FR-PF-10).
    Host,
}

impl From<restichub_db::SecretUsageKind> for BlockerKind {
    fn from(kind: restichub_db::SecretUsageKind) -> Self {
        match kind {
            restichub_db::SecretUsageKind::Repository => Self::Repository,
            restichub_db::SecretUsageKind::Channel => Self::Channel,
            restichub_db::SecretUsageKind::Profile => Self::Profile,
            restichub_db::SecretUsageKind::JobTemplate => Self::JobTemplate,
            restichub_db::SecretUsageKind::Host => Self::Host,
            restichub_db::SecretUsageKind::Plan => Self::Plan,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Blocker {
    pub kind: BlockerKind,
    pub id: String,
    /// For a job: its kind.
    pub name: String,
}

/// At most `restichub_db::referrers::REFERRER_LIMIT` records, and the count of all of them.
#[derive(Debug, Serialize, ToSchema)]
pub struct Blockers {
    pub items: Vec<Blocker>,
    pub total: i64,
}

impl Blockers {
    pub fn new(kind: BlockerKind, referrers: &restichub_db::referrers::Referrers) -> Self {
        Self {
            items: referrers
                .items
                .iter()
                .map(|item| Blocker {
                    kind,
                    id: item.id.to_string(),
                    name: item.name.clone(),
                })
                .collect(),
            total: referrers.total,
        }
    }
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
