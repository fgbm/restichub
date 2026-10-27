//! Router tests for the control plane HTTP server.

mod common;

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use restichub_cp::server::{AppState, router};
use restichub_db::{Database, DatabaseConfig, PLATFORM_TENANT_ID};
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

/// Builds a router with a lazy database. The database points to no server, so the readiness
/// check fails. The liveness and the document tests do not touch the database.
fn app() -> Result<Router, Box<dyn std::error::Error>> {
    let config = DatabaseConfig {
        url: "postgres://restichub@127.0.0.1:1/restichub".to_owned(),
        max_connections: 1,
        acquire_timeout: Duration::from_millis(100),
        statement_timeout: None,
    };
    let database = Database::connect_lazy(&config, None)?;
    Ok(router(AppState::new(database)))
}

async fn body_json(
    response: axum::response::Response,
) -> Result<Value, Box<dyn std::error::Error>> {
    let bytes = response.into_body().collect().await?.to_bytes();
    Ok(serde_json::from_slice(&bytes)?)
}

#[tokio::test]
async fn ac_nfr_11_healthz_is_alive() -> Result<(), Box<dyn std::error::Error>> {
    let response = app()?
        .oneshot(Request::builder().uri("/healthz").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    Ok(())
}

#[tokio::test]
async fn the_openapi_document_holds_the_routes() -> Result<(), Box<dyn std::error::Error>> {
    let response = app()?
        .oneshot(
            Request::builder()
                .uri("/api/v1/openapi.json")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let document = body_json(response).await?;
    assert_eq!(
        document
            .get("info")
            .and_then(|info| info.get("title"))
            .and_then(Value::as_str),
        Some("ResticHub API")
    );
    assert_eq!(
        document
            .get("info")
            .and_then(|info| info.get("version"))
            .and_then(Value::as_str),
        Some(restichub_common::BUILD_VERSION)
    );
    let paths = document.get("paths");
    assert!(paths.and_then(|paths| paths.get("/healthz")).is_some());
    assert!(paths.and_then(|paths| paths.get("/readyz")).is_some());
    Ok(())
}

#[tokio::test]
async fn a_path_parameter_is_in_its_route_template() -> Result<(), Box<dyn std::error::Error>> {
    let response = app()?
        .oneshot(
            Request::builder()
                .uri("/api/v1/openapi.json")
                .body(Body::empty())?,
        )
        .await?;
    let document = body_json(response).await?;
    let paths = document
        .get("paths")
        .and_then(Value::as_object)
        .expect("the document holds paths");
    for (path, item) in paths {
        for (method, operation) in item.as_object().into_iter().flatten() {
            let parameters = operation
                .get("parameters")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            for parameter in parameters {
                let name = parameter
                    .get("name")
                    .and_then(Value::as_str)
                    .expect("a name");
                let place = parameter
                    .get("in")
                    .and_then(Value::as_str)
                    .expect("a place");
                let in_template = path.contains(&format!("{{{name}}}"));
                assert_eq!(
                    place == "path",
                    in_template,
                    "{method} {path}: the parameter `{name}` is documented as `in: {place}`"
                );
            }
        }
    }
    let audit = &paths["/api/v1/audit"]["get"]["parameters"];
    let names: Vec<&str> = audit
        .as_array()
        .expect("the audit parameters")
        .iter()
        .filter_map(|parameter| parameter.get("name").and_then(Value::as_str))
        .collect();
    assert!(
        !names.contains(&"tenant_id"),
        "the tenant route ignores tenant_id: {names:?}"
    );
    Ok(())
}

/// The About ResticHub data answers a session without an organization, and the document names the
/// fields (AC-AU-06).
///
/// The version is the one fact that every role and every mode reads. The platform facts are the
/// Platform Admin's only.
#[tokio::test]
async fn ac_au_06_about_answers_in_a_session_without_a_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = common::app(db.database());

    // A Platform Admin with no organization: the session tenant is the internal platform tenant.
    let admin = common::seed_session(
        db.database(),
        PLATFORM_TENANT_ID,
        "about-root",
        "platform_admin",
    )
    .await;
    let (status, _, body) = common::send(
        &app,
        common::request("GET", "/api/v1/about", Some(&admin), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("version").is_some(), "{body}");
    assert!(body.get("revision").is_some(), "{body}");
    assert_eq!(
        body.get("tenancy_mode").and_then(Value::as_str),
        Some("multi")
    );
    assert!(
        body.get("schema_level").and_then(Value::as_u64).is_some(),
        "{body}"
    );

    // An account with no membership and no right reads the version and no platform fact.
    let plain = common::seed_memberless_account(&db, "about-plain", "phc").await;
    let cookie = common::session_cookie(db.database(), PLATFORM_TENANT_ID, plain).await;
    let (status, _, body) = common::send(
        &app,
        common::request("GET", "/api/v1/about", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("version").is_some(), "{body}");
    assert!(body.get("schema_level").is_none(), "{body}");
    assert!(body.get("tenancy_mode").is_none(), "{body}");

    // The document names the route, its security and its fields.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/openapi.json")
                .body(Body::empty())
                .expect("cannot build the request"),
        )
        .await
        .expect("the request failed");
    let document = body_json(response).await.expect("cannot read the document");
    let operation = &document["paths"]["/api/v1/about"]["get"];
    assert!(
        operation.get("security").is_some(),
        "the route needs the session schemes"
    );
    let schema = &document["components"]["schemas"]["AboutResponse"]["properties"];
    for field in [
        "version",
        "prerelease",
        "revision",
        "protocol",
        "agent_protocol",
        "tenancy_mode",
        "schema_level",
    ] {
        assert!(schema.get(field).is_some(), "the schema has no {field}");
    }

    db.cleanup().await;
}

#[tokio::test]
async fn an_unknown_api_route_is_a_problem() -> Result<(), Box<dyn std::error::Error>> {
    let response = app()?
        .oneshot(Request::builder().uri("/api/v1/nope").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    assert_eq!(content_type.as_deref(), Some("application/problem+json"));
    let problem = body_json(response).await?;
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("not_found")
    );
    // The response mapper adds the request ID from the request extension.
    assert!(
        problem
            .get("request_id")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty()),
        "the problem body must hold the request ID"
    );
    Ok(())
}

#[tokio::test]
async fn the_request_id_is_echoed() -> Result<(), Box<dyn std::error::Error>> {
    let response = app()?
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .header("x-request-id", "test-request-id")
                .body(Body::empty())?,
        )
        .await?;
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    assert_eq!(request_id.as_deref(), Some("test-request-id"));
    Ok(())
}

#[tokio::test]
async fn a_missing_request_id_is_generated_as_uuid_v7() -> Result<(), Box<dyn std::error::Error>> {
    let response = app()?
        .oneshot(Request::builder().uri("/healthz").body(Body::empty())?)
        .await?;
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .ok_or("the response holds no request ID")?;
    let version = Uuid::parse_str(&request_id)?.get_version_num();
    assert_eq!(version, 7, "the request ID {request_id} is not UUIDv7");
    Ok(())
}
