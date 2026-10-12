//! Database tests for the readiness endpoint and the migrations (issue #15).
//!
//! The tests read `RESTICHUB_TEST_DATABASE_URL`. If the variable is not set, the helper prints a
//! message and the tests do nothing.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use restichub_cp::server::{AppState, router};
use restichub_db::test_util::test_database;
use tower::ServiceExt;

#[tokio::test]
async fn readyz_is_ready_with_a_database() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database) = test_database().await else {
        return Ok(());
    };
    let app = router(AppState::new(database.database().clone()));
    let response = app
        .oneshot(Request::builder().uri("/readyz").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    database.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn the_migrations_create_the_schema_info_table() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database) = test_database().await else {
        return Ok(());
    };
    let count: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.tables WHERE table_name = 'schema_info'",
    )
    .fetch_one(database.pool())
    .await?;
    assert_eq!(count.0, 1);
    database.cleanup().await;
    Ok(())
}
