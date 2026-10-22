//! Database tests for the readiness endpoint and the migrations.
//!
//! The tests read `RESTICHUB_TEST_DATABASE_URL`. If the variable is not set, the helper prints a
//! message and the tests do nothing.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use restichub_cp::server::{AppState, router};
use restichub_cp::supervisor::{Policy, Supervisor};
use restichub_db::test_util::test_database;
use std::time::Duration;
use tower::ServiceExt;

#[tokio::test]
async fn ac_nfr_11_readyz_is_ready_with_a_database() -> Result<(), Box<dyn std::error::Error>> {
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
async fn ac_nfr_11_readyz_fails_while_a_background_task_is_failing()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database) = test_database().await else {
        return Ok(());
    };
    let supervisor = Supervisor::with_policy(Policy {
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(10),
        stable_after: Duration::from_secs(60),
    });
    let app =
        router(AppState::new(database.database().clone()).with_supervisor(supervisor.clone()));
    let ready = |app: axum::Router| async move {
        let request = Request::builder().uri("/readyz").body(Body::empty())?;
        let response = app.oneshot(request).await?;
        Ok::<_, Box<dyn std::error::Error>>(response.status())
    };
    assert_eq!(ready(app.clone()).await?, StatusCode::OK);

    supervisor.spawn_loop("broken_loop", || async { panic!("the loop breaks") });
    tokio::time::timeout(Duration::from_secs(10), async {
        while supervisor.failing().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await?;

    assert_eq!(ready(app).await?, StatusCode::SERVICE_UNAVAILABLE);
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
