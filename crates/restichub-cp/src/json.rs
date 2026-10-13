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

/// A body that is larger than the limit gives HTTP 413. Every other rejection gives HTTP 400. The
/// response holds no parser detail.
fn map_rejection(rejection: JsonRejection) -> ApiError {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::payload_too_large()
    } else {
        ApiError::invalid_field("body", Reason::InvalidBody)
    }
}
