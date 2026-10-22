//! The answer of a request whose database statement passes its timeout, and the pass limits that
//! must not use the default.

use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use restichub_cp::alerts::evaluate::evaluate_tenant;
use restichub_cp::delivery::DeliveryError;
use restichub_cp::error::ApiError;
use restichub_cp::job_signing::JobSigningError;
use restichub_cp::queue::AgentView;
use restichub_db::test_util::test_database;
use restichub_db::{Database, DbError, PLATFORM_TENANT_ID};
use time::OffsetDateTime;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A lock wait that passes a short statement timeout.
async fn timed_out(database: &Database) -> Result<DbError, DbError> {
    let mut holder = database.begin_tenant(PLATFORM_TENANT_ID).await?;
    holder.lock("statement-timeout").await?;
    let mut waiter = database.begin_tenant(PLATFORM_TENANT_ID).await?;
    waiter
        .statement_timeout(time::Duration::milliseconds(100))
        .await?;
    let Err(error) = waiter.lock("statement-timeout").await else {
        panic!("the lock wait must pass the timeout");
    };
    assert!(error.is_statement_timeout(), "{error:?}");
    drop(waiter);
    holder.commit().await?;
    Ok(error)
}

async fn assert_busy(error: ApiError) -> TestResult {
    let response = error.into_response();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::RETRY_AFTER], "5");
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024).await?;
    let body: serde_json::Value = serde_json::from_slice(&body)?;
    assert_eq!(body["code"], "busy", "{body}");
    Ok(())
}

#[tokio::test]
async fn a_request_statement_that_passes_its_timeout_answers_503_busy_with_retry_after()
-> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };

    assert_busy(ApiError::from(timed_out(db.database()).await?)).await?;
    assert_busy(ApiError::from(JobSigningError::from(
        timed_out(db.database()).await?,
    )))
    .await?;
    assert_busy(ApiError::from(DeliveryError::from(
        timed_out(db.database()).await?,
    )))
    .await?;

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn the_alert_evaluation_sets_its_own_statement_timeout() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    // The default of the connection is far below the time that the pass waits for the table.
    let database = db
        .database_with_statement_timeout(2, std::time::Duration::from_millis(100))
        .await?;

    let mut blocker = db.owner_pool().begin().await?;
    sqlx::query("LOCK TABLE alert_rules IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await?;
    let release = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        blocker.commit().await
    });

    let report = evaluate_tenant(
        &database,
        PLATFORM_TENANT_ID,
        OffsetDateTime::now_utc(),
        &AgentView::default(),
    )
    .await?;
    release.await??;
    assert!(!report.skipped);

    database.close().await;
    db.cleanup().await;
    Ok(())
}
