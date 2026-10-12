//! Tests for the embedded web UI fallback (issue #21).
//!
//! The tests build a router with a lazy database. They never touch the database. The immutable
//! asset test needs a UI build; without one the test prints a message and does nothing, as the
//! database tests do without `RESTICHUB_TEST_DATABASE_URL`.

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use restichub_cp::server::{AppState, router};
use restichub_db::{Database, DatabaseConfig};
use tower::ServiceExt;

/// Builds a router with a lazy database. The UI fallback does not touch the database.
fn app() -> Result<Router, Box<dyn std::error::Error>> {
    let config = DatabaseConfig {
        url: "postgres://restichub@127.0.0.1:1/restichub".to_owned(),
        max_connections: 1,
        acquire_timeout: Duration::from_millis(100),
    };
    let database = Database::connect_lazy(&config, None)?;
    Ok(router(AppState::new(database)))
}

async fn get(uri: &str) -> Result<axum::response::Response, Box<dyn std::error::Error>> {
    Ok(app()?
        .oneshot(Request::builder().uri(uri).body(Body::empty())?)
        .await?)
}

fn header_of(response: &axum::response::Response, name: header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[tokio::test]
async fn an_unknown_api_route_is_not_shadowed_by_the_ui() -> Result<(), Box<dyn std::error::Error>>
{
    let response = get("/api/v1/does-not-exist").await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        header_of(&response, header::CONTENT_TYPE).as_deref(),
        Some("application/problem+json")
    );
    Ok(())
}

#[tokio::test]
async fn the_spa_fallback_serves_the_index_with_no_cache() -> Result<(), Box<dyn std::error::Error>>
{
    let response = get("/users/42").await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header_of(&response, header::CONTENT_TYPE).as_deref(),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(
        header_of(&response, header::CACHE_CONTROL).as_deref(),
        Some("no-cache")
    );
    Ok(())
}

#[tokio::test]
async fn an_immutable_asset_is_cached_for_one_year() -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = restichub_cp::ui::asset_paths()
        .into_iter()
        .find(|path| path.starts_with("_app/immutable/"))
    else {
        println!("skipped: the UI was not built");
        return Ok(());
    };
    let response = get(&format!("/{path}")).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header_of(&response, header::CACHE_CONTROL).as_deref(),
        Some("public, max-age=31536000, immutable")
    );
    Ok(())
}

#[tokio::test]
async fn a_ui_response_holds_the_security_headers() -> Result<(), Box<dyn std::error::Error>> {
    let response = get("/").await?;
    let csp = header_of(&response, header::CONTENT_SECURITY_POLICY).unwrap_or_default();
    assert!(csp.contains("default-src 'self'"), "{csp}");
    assert!(csp.contains("script-src 'self'"), "{csp}");
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    assert_eq!(
        header_of(&response, header::X_CONTENT_TYPE_OPTIONS).as_deref(),
        Some("nosniff")
    );
    assert_eq!(
        header_of(&response, header::REFERRER_POLICY).as_deref(),
        Some("no-referrer")
    );
    Ok(())
}
