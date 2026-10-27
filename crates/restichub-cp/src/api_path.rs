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
        // The bytes are not text at all, so no `Reason` is true: every reason names a value that
        // the client can correct.
        ErrorKind::InvalidUtf8InPathParam { key } => {
            ApiError::invalid_argument_cause(key, "a path parameter is not valid UTF-8")
        }
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
        // The message of a rejection can hold the value from the URL, so the log holds a fixed
        // phrase and the numbers of the template only.
        ErrorKind::WrongNumberOfParameters { got, expected } => ApiError::internal(format!(
            "the handler reads {expected} path parameters and the route holds {got}"
        )),
        ErrorKind::UnsupportedType { name } => ApiError::internal(format!(
            "the handler reads a path parameter into the unsupported type `{name}`"
        )),
        _ => ApiError::internal("a path parameter of the route was rejected"),
    }
}

fn parse_error(name: String, expected_type: &str) -> ApiError {
    let cause = format!("the path parameter `{name}` is not a valid {expected_type}");
    ApiError::invalid_field_cause(name, Reason::InvalidFormat, cause)
}

/// Reads the name of the parameter at `index` from the route template.
///
/// A tuple of path parameters takes the names of the template in order. The name of a tail
/// parameter drops the `*`, which is the key of the capture in `axum::extract::Path`. The template
/// is a route of the control plane, so the name is not client input.
fn name_at(template: Option<&str>, index: usize) -> Option<String> {
    template?
        .split('/')
        .filter_map(|segment| segment.strip_prefix('{')?.strip_suffix('}'))
        .map(|name| name.strip_prefix('*').unwrap_or(name))
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

    use super::{ApiPath, name_at};

    async fn id(ApiPath(id): ApiPath<String>) -> Json<String> {
        Json(id)
    }

    async fn pair(ApiPath((agent, plan)): ApiPath<(String, String)>) -> Json<String> {
        Json(format!("{agent}/{plan}"))
    }

    async fn numbers(ApiPath((agent, plan)): ApiPath<(u32, u32)>) -> Json<String> {
        Json(format!("{agent}/{plan}"))
    }

    async fn single(ApiPath(value): ApiPath<String>) -> Json<String> {
        Json(value)
    }

    async fn tail(ApiPath(rest): ApiPath<String>) -> Json<String> {
        Json(rest)
    }

    async fn named(
        ApiPath(params): ApiPath<std::collections::BTreeMap<String, String>>,
    ) -> Json<String> {
        let pairs: Vec<String> = params
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        Json(pairs.join(","))
    }

    fn app() -> Router {
        Router::new()
            .route("/jobs/{id}", get(id))
            .route("/agents/{agent_id}/plans/{plan_id}", get(pair))
            .route("/counts/{agent_id}/plans/{plan_id}", get(numbers))
            .route("/single/{agent_id}/plans/{plan_id}", get(single))
            .route("/files/{*rest}", get(tail))
            .route("/tail/{agent_id}/{*rest}", get(numbers))
            .route("/named/{agent_id}/{*rest}", get(named))
    }

    struct Reply {
        status: StatusCode,
        content_type: String,
        text: String,
        /// The cause that the response mapper logs, empty when the error carries none.
        log: String,
    }

    impl Reply {
        fn problem(&self) -> serde_json::Value {
            serde_json::from_str(&self.text).unwrap_or(serde_json::Value::Null)
        }
    }

    /// Sends a request and keeps the parts of the answer that a test reads.
    async fn call(uri: &str) -> Reply {
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
        let log = response
            .extensions()
            .get::<crate::error::InternalCause>()
            .map(|cause| cause.0.clone())
            .unwrap_or_default();
        let text = response
            .into_body()
            .collect()
            .await
            .expect("the body")
            .to_bytes();
        Reply {
            status,
            content_type,
            text: String::from_utf8_lossy(&text).into_owned(),
            log,
        }
    }

    #[tokio::test]
    async fn a_path_parameter_that_is_not_utf8_is_a_problem() {
        let reply = call("/jobs/%FF").await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
        assert_eq!(reply.content_type, "application/problem+json");
        let problem = reply.problem();
        assert_eq!(problem["code"], "invalid_argument");
        assert_eq!(problem["field"], "id");
        assert!(
            problem.get("reason").is_none(),
            "no reason is true for a value that is not text"
        );
    }

    #[tokio::test]
    async fn a_second_path_parameter_that_is_not_utf8_is_a_problem() {
        let reply = call("/agents/1/plans/%FF").await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
        assert_eq!(reply.problem()["field"], "plan_id");
    }

    #[tokio::test]
    async fn a_good_path_parameter_reaches_the_handler() {
        let reply = call("/agents/1/plans/2").await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.text, "\"1/2\"");
    }

    /// The index of a tuple is the position of the parameter in the route template.
    #[tokio::test]
    async fn a_number_that_does_not_parse_names_its_parameter() {
        let reply = call("/counts/1/plans/secret").await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
        let problem = reply.problem();
        assert_eq!(problem["code"], "invalid_argument");
        assert_eq!(problem["field"], "plan_id");
        assert!(
            !reply.text.contains("secret"),
            "the problem echoes the value: {}",
            reply.text
        );
        assert!(
            !reply.log.contains("secret"),
            "the log echoes the value: {}",
            reply.log
        );
    }

    /// A handler that takes fewer parameters than the route holds is a bug of the control plane.
    #[tokio::test]
    async fn a_wrong_number_of_parameters_is_internal() {
        let reply = call("/single/VALUE-1/plans/VALUE-2").await;
        assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
        let problem = reply.problem();
        assert_eq!(problem["code"], "internal_error");
        assert!(
            problem.get("field").is_none(),
            "the problem names no parameter for a bug of the handler"
        );
        assert!(
            !reply.text.contains("VALUE-1") && !reply.text.contains("VALUE-2"),
            "the problem echoes the value: {}",
            reply.text
        );
        assert!(
            !reply.log.contains("VALUE-1") && !reply.log.contains("VALUE-2"),
            "the log echoes the value: {}",
            reply.log
        );
    }

    /// A tail parameter is one capture of the whole rest of the path, as in `axum::extract::Path`.
    #[tokio::test]
    async fn a_tail_parameter_holds_the_rest_of_the_path() {
        let reply = call("/files/a/b/c").await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.text, "\"a/b/c\"");
    }

    /// The name of a tail parameter drops the `*` of the template, as the key of the capture does.
    #[tokio::test]
    async fn a_tail_parameter_is_not_named_with_its_marker() {
        let reply = call("/tail/1/secret").await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
        let problem = reply.problem();
        assert_eq!(problem["field"], "rest");
        assert!(
            problem["field"].as_str() != Some("*rest"),
            "the marker of the tail is not part of the name"
        );
        assert!(
            !reply.text.contains("secret"),
            "the problem echoes the value"
        );
    }

    /// The keys of the captures of the router: the marker of the tail is not part of its key.
    #[tokio::test]
    async fn the_key_of_a_tail_parameter_drops_the_marker() {
        let reply = call("/named/1/secret").await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.text, "\"agent_id=1,rest=secret\"");
    }

    #[test]
    fn the_names_of_the_template_are_read_in_order() {
        let template = Some("/agents/{agent_id}/plans/{plan_id}");
        assert_eq!(name_at(template, 0).as_deref(), Some("agent_id"));
        assert_eq!(name_at(template, 1).as_deref(), Some("plan_id"));
        assert_eq!(name_at(template, 2), None);
        assert_eq!(name_at(None, 0), None);
        assert_eq!(
            name_at(Some("/files/{id}/{*rest}"), 1).as_deref(),
            Some("rest")
        );
    }
}
