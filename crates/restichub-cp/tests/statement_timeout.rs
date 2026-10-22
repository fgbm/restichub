//! The answer of a request whose database statement passes its timeout.

use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use restichub_cp::error::ApiError;
use restichub_db::test_util::test_database;

#[tokio::test]
async fn ac_nfr_01_a_request_statement_that_passes_its_timeout_answers_503_with_retry_after()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(database) = test_database().await else {
        return Ok(());
    };
    let tenant = restichub_db::PLATFORM_TENANT_ID;

    // The first transaction holds the lock, so the second one waits until its timeout cancels it.
    let mut holder = database.database().begin_tenant(tenant).await?;
    holder.lock("statement-timeout").await?;
    let mut waiter = database.database().begin_tenant(tenant).await?;
    waiter
        .statement_timeout(time::Duration::milliseconds(100))
        .await?;
    let error = waiter
        .lock("statement-timeout")
        .await
        .expect_err("the lock wait must pass the timeout");
    assert!(error.is_statement_timeout(), "{error:?}");

    let response = ApiError::from(error).into_response();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(header::RETRY_AFTER));

    drop(waiter);
    holder.commit().await?;
    database.cleanup().await;
    Ok(())
}
