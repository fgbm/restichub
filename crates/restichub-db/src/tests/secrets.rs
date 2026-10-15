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

async fn insert_hook_secret(
    tx: &mut crate::TenantTx<'_>,
    tenant: crate::TenantId,
    name: &str,
) -> Result<SecretId, Box<dyn std::error::Error>> {
    let secret = SecretId::new();
    sqlx::query(
        "INSERT INTO secrets (id, tenant_id, name, purpose, key_version, nonce, ciphertext) \
         VALUES ($1, $2, $3, 'hook_secret', 1, $4, $5)",
    )
    .bind(secret)
    .bind(tenant)
    .bind(name)
    .bind(vec![1u8; 12])
    .bind(vec![2u8; 32])
    .execute(tx.as_conn())
    .await?;
    Ok(secret)
}

/// The JSON shape of a `ScriptDef` as the control plane saves it.
fn script_with(body: &str, kind: &str, value: &str) -> serde_json::Value {
    serde_json::json!([{
        "name": "dump",
        "interpreter": "bash",
        "phase": "before",
        "body": body,
        "timeout_seconds": 60,
        "failure_policy": "abort_backup",
        "env": [{"name": "PGPASSWORD", "kind": kind, "value": value}],
    }])
}

#[tokio::test]
async fn db_secret_usage_finds_a_secret_ref_of_a_profile_and_a_template_only() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;
    seed_tenant(db.database(), tenant_b).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let referenced = insert_hook_secret(&mut tx, tenant_a, "referenced").await?;
    let mentioned = insert_hook_secret(&mut tx, tenant_a, "mentioned").await?;
    let templated = insert_hook_secret(&mut tx, tenant_a, "templated").await?;
    let plain = insert_hook_secret(&mut tx, tenant_a, "plain").await?;
    // The body and a string variable name the identifiers of two other secrets: no usage.
    let mut scripts = script_with(
        &format!("echo {mentioned}"),
        "secret_ref",
        &referenced.to_string(),
    );
    scripts[0]["env"]
        .as_array_mut()
        .expect("env")
        .push(serde_json::json!({"name": "NOTE", "kind": "string", "value": plain.to_string()}));
    sqlx::query("UPDATE profiles SET scripts = $1 WHERE name = 'profile'")
        .bind(&scripts)
        .execute(tx.as_conn())
        .await?;
    sqlx::query(
        "INSERT INTO job_templates (id, tenant_id, name, scripts) VALUES ($1, $2, 'template', $3)",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(tenant_a)
    .bind(script_with("echo", "secret_ref", &templated.to_string()))
    .execute(tx.as_conn())
    .await?;

    let usage = crate::secrets::list_usage(&mut tx, None).await?;
    let of =
        |id: SecretId| -> Vec<_> { usage.iter().filter(|item| item.secret_id == id).collect() };
    assert_eq!(of(referenced).len(), 1);
    assert_eq!(of(referenced)[0].kind, crate::SecretUsageKind::Profile);
    assert_eq!(of(referenced)[0].object_name, "profile");
    assert_eq!(of(templated).len(), 1);
    assert_eq!(of(templated)[0].kind, crate::SecretUsageKind::JobTemplate);
    assert_eq!(of(templated)[0].object_name, "template");
    assert!(of(mentioned).is_empty());
    assert!(of(plain).is_empty());

    let only = crate::secrets::list_usage(&mut tx, Some(templated)).await?;
    assert_eq!(only.len(), 1);
    assert_eq!(only[0].secret_id, templated);
    tx.commit().await?;

    let mut tx = db.database().begin_tenant(tenant_b).await?;
    let foreign = crate::secrets::list_usage(&mut tx, None).await?;
    assert!(foreign.iter().all(|item| item.secret_id != referenced));
    drop(tx);

    db.cleanup().await;
    Ok(())
}

/// FR-RP-05: a delete waits for a save that references the secret, then sees the reference.
#[tokio::test]
async fn db_a_delete_waits_for_a_save_that_references_the_secret() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant).await?;
    let mut setup = db.database().begin_tenant(tenant).await?;
    let secret = insert_hook_secret(&mut setup, tenant, "raced").await?;
    setup.commit().await?;

    // The save locks the row of the secret before the delete starts.
    let mut save = db.database().begin_tenant(tenant).await?;
    assert!(
        crate::secrets::get_for_key_share(&mut save, secret)
            .await?
            .is_some()
    );

    let delete = async {
        let mut tx = db.database().begin_tenant(tenant).await?;
        crate::secrets::get_for_update(&mut tx, secret).await?;
        let usage = crate::secrets::list_usage(&mut tx, Some(secret)).await?;
        tx.commit().await?;
        Ok::<_, Box<dyn std::error::Error>>(usage.len())
    };
    let commit_save = async {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        sqlx::query("UPDATE profiles SET scripts = $1 WHERE name = 'profile'")
            .bind(script_with("echo", "secret_ref", &secret.to_string()))
            .execute(save.as_conn())
            .await?;
        save.commit().await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    };
    let (used_by, saved) = tokio::join!(delete, commit_save);
    saved?;
    assert_eq!(used_by?, 1, "the delete must see the committed reference");

    db.cleanup().await;
    Ok(())
}
