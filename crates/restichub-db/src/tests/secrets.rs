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

#[tokio::test]
async fn db_secret_usage_finds_a_hook_secret_in_the_json_of_a_profile() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;
    seed_tenant(db.database(), tenant_b).await?;
    let secret = SecretId::new();

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    sqlx::query(
        "INSERT INTO secrets (id, tenant_id, name, purpose, key_version, nonce, ciphertext) \
         VALUES ($1, $2, 'hook', 'hook_secret', 1, $3, $4)",
    )
    .bind(secret)
    .bind(tenant_a)
    .bind(vec![1u8; 12])
    .bind(vec![2u8; 32])
    .execute(tx.as_conn())
    .await?;
    sqlx::query(
        "UPDATE profiles SET scripts = jsonb_build_array(jsonb_build_object('value', $1::text)) \
         WHERE name = 'profile'",
    )
    .bind(secret.to_string())
    .execute(tx.as_conn())
    .await?;
    let usage = crate::secrets::list_usage(&mut tx).await?;
    let hook: Vec<_> = usage
        .iter()
        .filter(|item| item.secret_id == secret)
        .collect();
    assert_eq!(hook.len(), 1);
    assert_eq!(hook[0].kind, crate::SecretUsageKind::Profile);
    assert_eq!(hook[0].object_name, "profile");
    tx.commit().await?;

    let mut tx = db.database().begin_tenant(tenant_b).await?;
    let foreign = crate::secrets::list_usage(&mut tx).await?;
    assert!(foreign.iter().all(|item| item.secret_id != secret));
    drop(tx);

    db.cleanup().await;
    Ok(())
}
