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
async fn the_migrations_create_the_platform_tenant() -> Result<(), Box<dyn std::error::Error>> {
    let Some(database) = test_database().await else {
        return Ok(());
    };
    let mut transaction = database.database().begin_platform().await?;
    let tenants = restichub_db::tenants::list(&mut transaction).await?;
    transaction.commit().await?;
    assert!(
        tenants
            .iter()
            .any(|tenant| tenant.id == restichub_db::PLATFORM_TENANT_ID)
    );
    database.cleanup().await;
    Ok(())
}
