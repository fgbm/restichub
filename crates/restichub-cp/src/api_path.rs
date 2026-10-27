//! The path-parameter extractor of the API (ADR-0023, section Errors).
//!
//! [`ApiPath`] reads the path parameters like [`axum::extract::Path`] and answers a bad parameter
//! with the problem response of the control plane. The rejection of [`axum::extract::Path`] is a
//! plain-text `400` that bypasses that response.
//!
//! A parameter that is not valid UTF-8 after the percent decoding and a parameter that does not
//! parse give HTTP 400 `invalid_argument` that names the parameter. A rejection that means a bug
//! in a handler, such as a wrong number of parameters, gives HTTP 500 `internal_error`.
//!
//! The `field` and the cause of the log never echo the value: a parameter of a client can hold a
//! secret, and the name comes from the route template of the control plane.

use axum::extract::path::ErrorKind;
use axum::extract::rejection::PathRejection;
use axum::extract::{FromRequestParts, MatchedPath, Path};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

use crate::error::ApiError;
use crate::validation::Reason;

/// Path parameters with the problem-response mapping of the control plane.
///
/// Use this type in a handler of the API instead of [`axum::extract::Path`].
#[derive(Debug)]
pub struct ApiPath<T>(pub T);

impl<T, S> FromRequestParts<S> for ApiPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let template = parts
            .extensions
            .get::<MatchedPath>()
            .map(|matched| matched.as_str().to_owned());
        match Path::<T>::from_request_parts(parts, state).await {
            Ok(Path(value)) => Ok(Self(value)),
            Err(rejection) => Err(map_rejection(rejection, template.as_deref())),
        }
    }
}

/// The `field` of a parameter whose name the rejection does not hold, in the way `body` names a
/// JSON field and `query` names a query parameter.
const UNNAMED: &str = "path";

fn map_rejection(rejection: PathRejection, template: Option<&str>) -> ApiError {
    let PathRejection::FailedToDeserializePathParams(error) = rejection else {
        return ApiError::internal("the route holds no path parameter");
    };
    match error.into_kind() {
        ErrorKind::InvalidUtf8InPathParam { key } => ApiError::invalid_field_cause(
            key,
            Reason::InvalidFormat,
            "a path parameter is not valid UTF-8",
        ),
        ErrorKind::ParseErrorAtKey {
            key, expected_type, ..
        } => parse_error(key, expected_type),
        ErrorKind::DeserializeError { key, .. } => parse_error(key, "value"),
        ErrorKind::ParseError { expected_type, .. } => parse_error(
            name_at(template, 0).unwrap_or_else(|| UNNAMED.to_owned()),
            expected_type,
        ),
        ErrorKind::ParseErrorAtIndex {
            index,
            expected_type,
            ..
        } => parse_error(
            name_at(template, index).unwrap_or_else(|| UNNAMED.to_owned()),
            expected_type,
        ),
        other => ApiError::internal(format!(
            "a path parameter of the route was rejected: {other}"
        )),
    }
}

fn parse_error(name: String, expected_type: &str) -> ApiError {
    let cause = format!("the path parameter `{name}` is not a valid {expected_type}");
    ApiError::invalid_field_cause(name, Reason::InvalidFormat, cause)
}

/// Reads the name of the parameter at `index` from the route template.
///
/// A tuple of path parameters takes the names of the template in order. The template is a route of
/// the control plane, so the name is not client input.
fn name_at(template: Option<&str>, index: usize) -> Option<String> {
    template?
        .split('/')
        .filter_map(|segment| segment.strip_prefix('{')?.strip_suffix('}'))
        .nth(index)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use axum::Json;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::ApiPath;

    async fn id(ApiPath(id): ApiPath<String>) -> Json<String> {
        Json(id)
    }

    async fn pair(ApiPath((agent, plan)): ApiPath<(String, String)>) -> Json<String> {
        Json(format!("{agent}/{plan}"))
    }

    fn app() -> Router {
        Router::new()
            .route("/jobs/{id}", get(id))
            .route("/agents/{agent_id}/plans/{plan_id}", get(pair))
    }

    /// Sends a request and returns the status, the content type and the body.
    async fn call(uri: &str) -> (StatusCode, String, serde_json::Value) {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("the request"),
            )
            .await
            .expect("the response");
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("the body")
            .to_bytes();
        let value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        (status, content_type, value)
    }

    #[tokio::test]
    async fn a_path_parameter_that_is_not_utf8_is_a_problem() {
        let (status, content_type, problem) = call("/jobs/%FF").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(content_type, "application/problem+json");
        assert_eq!(problem["code"], "invalid_argument");
        assert_eq!(problem["field"], "id");
    }

    #[tokio::test]
    async fn a_second_path_parameter_that_is_not_utf8_is_a_problem() {
        let (status, _, problem) = call("/agents/1/plans/%FF").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(problem["field"], "plan_id");
    }

    #[tokio::test]
    async fn a_good_path_parameter_reaches_the_handler() {
        let (status, _, body) = call("/agents/1/plans/2").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "1/2");
    }
}
