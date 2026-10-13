//! The JSON request-body extractor of the control plane (ADR-0023, section Errors).
//!
//! [`ApiJson`] wraps [`axum::Json`]. It maps every rejection to the RFC 9457 problem response of
//! the control plane. A body that is not valid JSON, a body that does not match the schema and a
//! missing `Content-Type` give HTTP 400 `invalid_argument`. A body that is larger than the body
//! limit of the route gives HTTP 413 `payload_too_large`.
//!
//! The response holds no detail of the parser. A serde message can echo a part of the body, and a
//! secret value can be in that body, so the control plane never returns the parser detail. The
//! extractor also never logs the body.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;

use crate::error::ApiError;
use crate::validation::Reason;

/// A JSON request body with the problem-response mapping of the control plane.
///
/// Use this type for the request body of a handler instead of [`axum::Json`]. The rejection is
/// always a [`crate::error::ProblemBody`].
#[derive(Debug)]
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(request, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) => Err(map_rejection(rejection)),
        }
    }
}

/// The fixed prefix of the message of a data error. The `serde_path_to_error` path follows it.
const JSON_DATA_ERROR_PREFIX: &str = "Failed to deserialize the JSON body into the target type: ";

/// A body that is larger than the limit gives HTTP 413. Every other rejection gives HTTP 400. The
/// response holds no parser detail. A data error names the field of the schema that failed, so a
/// form can show the message next to the input.
fn map_rejection(rejection: JsonRejection) -> ApiError {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return ApiError::payload_too_large();
    }
    let field = match &rejection {
        JsonRejection::JsonDataError(error) => body_error_field(&error.body_text()),
        _ => None,
    };
    ApiError::invalid_field(
        field.unwrap_or_else(|| "body".to_owned()),
        Reason::InvalidBody,
    )
}

/// Reads the schema path from the message of a data error.
///
/// The path comes from the request type, not from user input. The function accepts only a path of
/// letters, digits, `_` and `.`, so an unexpected value falls back to `body`.
fn body_error_field(message: &str) -> Option<String> {
    let rest = message.strip_prefix(JSON_DATA_ERROR_PREFIX)?;
    let path = rest.split_once(": ").map_or(rest, |(path, _)| path).trim();
    if path.is_empty() || path == "." {
        return None;
    }
    let valid = path
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '.'));
    valid.then(|| path.to_owned())
}
