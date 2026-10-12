//! Secret storage tests: the tenant key and the active key rule.

use super::*;
use crate::test_util::test_database;

#[tokio::test]
async fn db_secrets_require_a_tenant_key_and_one_active_key() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;

    // A secret with no matching key row fails (23503).
    let unknown_key = sqlx::query(
        "INSERT INTO secrets \
         (id, tenant_id, name, purpose, key_version, nonce, ciphertext) \
         VALUES ($1, $2, 's2', 'p', 99, $3, $4)",
    )
    .bind(SecretId::new())
    .bind(tenant_a)
    .bind(vec![5u8; 12])
    .bind(vec![6u8; 32])
    .execute(tx.as_conn())
    .await;
    assert_code(unknown_key, "23503");
    drop(tx);

    // A second active key row for the same tenant fails (23505).
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let second_active = sqlx::query(
        "INSERT INTO tenant_keys (tenant_id, key_version, kek_id, wrapped_dek, nonce) \
         VALUES ($1, 2, 'kek-2', $2, $3)",
    )
    .bind(tenant_a)
    .bind(vec![3u8; 48])
    .bind(vec![4u8; 12])
    .execute(tx.as_conn())
    .await;
    assert_code(second_active, "23505");
    drop(tx);

    // The runtime role cannot delete a key row (42501).
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let delete_key = sqlx::query("DELETE FROM tenant_keys")
        .execute(tx.as_conn())
        .await;
    assert_code(delete_key, "42501");
    drop(tx);

    db.cleanup().await;
    Ok(())
}
