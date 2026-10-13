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
/// The path names a field of the request type. The function refuses any value that can hold user
/// input, so the problem body never echoes a part of the request:
///
/// - An `unknown field` error names the key that the client sent, so the function falls back to
///   `body`.
/// - The path must hold only letters, digits, `_` and `.`.
fn body_error_field(message: &str) -> Option<String> {
    let rest = message.strip_prefix(JSON_DATA_ERROR_PREFIX)?;
    let (path, detail) = rest.split_once(": ").unwrap_or((rest, ""));
    if detail.starts_with("unknown field") {
        return None;
    }
    let path = path.trim();
    if path.is_empty() || path == "." {
        return None;
    }
    let valid = path
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '.'));
    valid.then(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use axum::Json;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;
    use serde::Deserialize;

    use super::map_rejection;

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct Known {
        role: String,
    }

    /// Returns the `field` of the problem body for a body that does not match the schema.
    async fn field_of(body: &str) -> Option<String> {
        let rejection =
            Json::<Known>::from_bytes(body.as_bytes()).expect_err("the body must be refused");
        let response = map_rejection(rejection).into_response();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map(|body| body.to_bytes())
            .ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        value
            .get("field")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    }

    #[tokio::test]
    async fn a_known_field_error_names_the_field() {
        // The value of a known field is wrong; the path is the field of the schema.
        assert_eq!(field_of(r#"{"role": 5}"#).await.as_deref(), Some("role"));
    }

    #[tokio::test]
    async fn an_unknown_field_never_echoes_the_client_key() {
        // A key that the client sent, including a markup name, never reaches the problem body.
        assert_eq!(
            field_of(r#"{"<script>": 1}"#).await.as_deref(),
            Some("body")
        );
        assert_eq!(
            field_of(r#"{"canary_field_name": 1}"#).await.as_deref(),
            Some("body")
        );
    }
}
