//! The default `statement_timeout` of the runtime connections (ADR-0023).
//!
//! A request query that runs without a limit keeps a pool connection after its request ended, so
//! the pool can fill up and stop the control plane.

use std::time::Duration;

use super::TestResult;
use crate::Database;
use crate::test_util::test_database;

const DEFAULT_TIMEOUT: Duration = Duration::from_millis(300);

async fn timeout_setting(database: &Database) -> Result<String, Box<dyn std::error::Error>> {
    let mut tx = database.begin_platform().await?;
    let value: String = sqlx::query_scalar("SHOW statement_timeout")
        .fetch_one(tx.as_conn())
        .await?;
    Ok(value)
}

#[tokio::test]
async fn db_a_statement_that_passes_the_default_timeout_is_cancelled() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let database = db
        .database_with_statement_timeout(1, DEFAULT_TIMEOUT)
        .await?;

    let mut tx = database.begin_platform().await?;
    let error = match sqlx::query("SELECT pg_sleep(5)")
        .execute(tx.as_conn())
        .await
    {
        Ok(_) => panic!("the statement must pass the default timeout"),
        Err(error) => crate::DbError::Query(error),
    };
    assert!(
        error.is_statement_timeout(),
        "the error names the timeout: {error:?}"
    );
    drop(tx);

    database.close().await;
    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_a_pass_can_raise_the_default_and_the_default_returns_after_it() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let database = db
        .database_with_statement_timeout(1, DEFAULT_TIMEOUT)
        .await?;
    assert_eq!(timeout_setting(&database).await?, "300ms");

    let mut tx = database.begin_platform().await?;
    sqlx::query("SELECT set_config('statement_timeout', '10s', true)")
        .execute(tx.as_conn())
        .await?;
    sqlx::query("SELECT pg_sleep(0.6)")
        .execute(tx.as_conn())
        .await?;
    tx.commit().await?;

    // The pool has one connection, so this transaction runs on the connection that was raised.
    assert_eq!(timeout_setting(&database).await?, "300ms");

    database.close().await;
    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_a_tenant_transaction_raises_the_default_with_its_own_timeout() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let database = db
        .database_with_statement_timeout(1, DEFAULT_TIMEOUT)
        .await?;

    let mut tx = database.begin_tenant(crate::PLATFORM_TENANT_ID).await?;
    tx.statement_timeout(time::Duration::seconds(10)).await?;
    sqlx::query("SELECT pg_sleep(0.6)")
        .execute(tx.as_conn())
        .await?;
    tx.commit().await?;

    assert_eq!(timeout_setting(&database).await?, "300ms");

    database.close().await;
    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_a_session_level_timeout_does_not_survive_on_a_pooled_connection() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let database = db
        .database_with_statement_timeout(1, DEFAULT_TIMEOUT)
        .await?;

    {
        let mut tx = database.begin_platform().await?;
        sqlx::query("SELECT set_config('statement_timeout', '0', false)")
            .execute(tx.as_conn())
            .await?;
        tx.commit().await?;
    }

    assert_eq!(timeout_setting(&database).await?, "300ms");

    database.close().await;
    db.cleanup().await;
    Ok(())
}
