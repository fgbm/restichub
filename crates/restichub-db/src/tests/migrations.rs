//! Migration version tests.

use super::TestResult;
use crate::test_util::test_database;

#[tokio::test]
async fn db_verify_migrations_detects_a_newer_schema() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };

    // The database matches the embedded migrations.
    db.database().verify_migrations().await?;

    // A newer applied version is reported as ahead (N4).
    let fake = 9_999_999_i64;
    sqlx::query(
        "INSERT INTO _sqlx_migrations \
         (version, description, installed_on, success, checksum, execution_time) \
         VALUES ($1, 'fake', now(), true, $2, 0)",
    )
    .bind(fake)
    .bind(vec![0u8; 32])
    .execute(db.owner_pool())
    .await?;
    assert!(db.database().verify_migrations().await.is_err());

    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = $1")
        .bind(fake)
        .execute(db.owner_pool())
        .await?;
    db.database().verify_migrations().await?;

    db.cleanup().await;
    Ok(())
}
