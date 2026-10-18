//! The profile parameters and their values in the schema (FR-PF-09 to FR-PF-12, ADR-0047).

use super::*;
use crate::test_util::test_database;

/// The declarations of a profile are a JSON array (FR-PF-09).
#[tokio::test]
async fn db_the_parameters_of_a_profile_are_an_array() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    // A new profile starts with no parameter.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let stored: (serde_json::Value,) =
        sqlx::query_as("SELECT parameters FROM profiles WHERE tenant_id = $1")
            .bind(tenant_a)
            .fetch_one(tx.as_conn())
            .await?;
    assert_eq!(stored.0, serde_json::json!([]));
    let object = sqlx::query("UPDATE profiles SET parameters = '{}'::jsonb WHERE tenant_id = $1")
        .bind(tenant_a)
        .execute(tx.as_conn())
        .await;
    assert_code(object, "23514");
    drop(tx);

    db.cleanup().await;
    Ok(())
}

/// The kind decides the column that holds the value (FR-PF-10).
#[tokio::test]
async fn db_a_value_holds_exactly_one_of_the_value_and_the_secret() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    // Each negative write runs in its own transaction, so an aborted transaction does not hide the
    // result of the next one.
    // Each negative write runs in its own transaction, so an aborted transaction does not hide the
    // result of the next one.
    let database = db.database().clone();
    let bad = |sql: &'static str, database: crate::Database| async move {
        let mut tx = database
            .begin_tenant(tenant_a)
            .await
            .expect("a transaction");
        sqlx::query(sql).bind(tenant_a).execute(tx.as_conn()).await
    };
    // A text value without a value, a secret value with a text value and an unknown kind.
    let missing = bad(
        "INSERT INTO agent_parameter_values (tenant_id, agent_id, name, kind, value) \
         VALUES ($1, (SELECT id FROM agents WHERE tenant_id = $1), 'pg_dir', 'text', NULL)",
        database.clone(),
    )
    .await;
    assert_code(missing, "23514");
    let both = bad(
        "INSERT INTO agent_parameter_values (tenant_id, agent_id, name, kind, value) \
         VALUES ($1, (SELECT id FROM agents WHERE tenant_id = $1), 'pg_password', 'secret', '/srv')",
        database.clone(),
    )
    .await;
    assert_code(both, "23514");
    let unknown = bad(
        "INSERT INTO agent_parameter_values (tenant_id, agent_id, name, kind, value) \
         VALUES ($1, (SELECT id FROM agents WHERE tenant_id = $1), 'pg_user', 'number', '1')",
        database.clone(),
    )
    .await;
    assert_code(unknown, "23514");

    db.cleanup().await;
    Ok(())
}

/// A secret that a value names cannot go (FR-PF-10, FR-PF-12).
#[tokio::test]
async fn db_a_secret_of_a_parameter_value_blocks_its_delete() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let secret: (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO secrets (id, tenant_id, name, purpose, key_version, nonce, ciphertext) \
         VALUES ($1, $2, 'pg-password', 'hook_secret', 1, $3, $4) RETURNING id",
    )
    .bind(crate::SecretId::new())
    .bind(tenant_a)
    .bind(vec![7u8; 12])
    .bind(vec![8u8; 32])
    .fetch_one(tx.as_conn())
    .await?;
    sqlx::query(
        "INSERT INTO agent_parameter_values (tenant_id, agent_id, name, kind, secret_id) \
         VALUES ($1, (SELECT id FROM agents WHERE tenant_id = $1), 'pg_password', 'secret', $2)",
    )
    .bind(tenant_a)
    .bind(secret.0)
    .execute(tx.as_conn())
    .await?;
    let delete = sqlx::query("DELETE FROM secrets WHERE id = $1")
        .bind(secret.0)
        .execute(tx.as_conn())
        .await;
    assert_code(delete, "23503");
    drop(tx);

    db.cleanup().await;
    Ok(())
}

/// A plan delete takes its overrides with it (FR-PF-10).
#[tokio::test]
async fn db_a_plan_delete_removes_its_parameter_values() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let before = count_rows(&mut tx, "plan_parameter_values").await?;
    assert_eq!(before, 1);
    sqlx::query("DELETE FROM plans WHERE tenant_id = $1")
        .bind(tenant_a)
        .execute(tx.as_conn())
        .await?;
    assert_eq!(count_rows(&mut tx, "plan_parameter_values").await?, 0);
    // The values of the host belong to the host, so they stay.
    assert_eq!(count_rows(&mut tx, "agent_parameter_values").await?, 1);
    tx.commit().await?;

    db.cleanup().await;
    Ok(())
}
