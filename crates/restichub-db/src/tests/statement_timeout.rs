//! The default `statement_timeout` of the runtime connections (ADR-0023).
//!
//! A request query that runs without a limit keeps a pool connection after its request ended, so
//! the pool can fill up and stop the control plane.

use std::time::Duration;

use super::TestResult;
use crate::{Database, DatabaseConfig};

const DEFAULT_TIMEOUT: Duration = Duration::from_millis(300);

async fn timeout_setting(database: &Database) -> Result<String, Box<dyn std::error::Error>> {
    let mut tx = database.begin_platform().await?;
    let value: String = sqlx::query_scalar("SHOW statement_timeout")
        .fetch_one(tx.as_conn())
        .await?;
    Ok(value)
}

async fn connect_with_timeout() -> Result<Database, Box<dyn std::error::Error>> {
    let config = DatabaseConfig {
        url: std::env::var("RESTICHUB_TEST_DATABASE_URL")?,
        max_connections: 1,
        acquire_timeout: Duration::from_secs(5),
        statement_timeout: Some(DEFAULT_TIMEOUT),
    };
    Ok(Database::connect(&config, None).await?)
}

#[tokio::test]
async fn db_a_statement_that_passes_the_default_timeout_is_cancelled() -> TestResult {
    if std::env::var("RESTICHUB_TEST_DATABASE_URL").is_err() {
        return Ok(());
    }
    let database = connect_with_timeout().await?;

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
    Ok(())
}

#[tokio::test]
async fn db_a_pass_can_raise_the_default_and_the_default_returns_after_it() -> TestResult {
    if std::env::var("RESTICHUB_TEST_DATABASE_URL").is_err() {
        return Ok(());
    }
    let database = connect_with_timeout().await?;
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
    Ok(())
}

#[tokio::test]
async fn db_a_session_level_timeout_does_not_survive_on_a_pooled_connection() -> TestResult {
    if std::env::var("RESTICHUB_TEST_DATABASE_URL").is_err() {
        return Ok(());
    }
    let database = connect_with_timeout().await?;

    {
        let mut tx = database.begin_platform().await?;
        sqlx::query("SELECT set_config('statement_timeout', '0', false)")
            .execute(tx.as_conn())
            .await?;
        tx.commit().await?;
    }

    assert_eq!(timeout_setting(&database).await?, "300ms");

    database.close().await;
    Ok(())
}
