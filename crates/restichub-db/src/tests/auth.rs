use super::*;
use crate::test_util::test_database;
use uuid::Uuid;

#[tokio::test]
async fn db_an_api_token_cannot_be_a_platform_admin_in_a_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    let seed = seed_tenant(db.database(), tenant_a).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let bad_role = sqlx::query(
        "INSERT INTO api_tokens \
         (id, tenant_id, name, token_hash, display_prefix, role, created_by) \
         VALUES ($1, $2, 't', $3, 'rh_x', 'platform_admin', $4)",
    )
    .bind(ApiTokenId::new())
    .bind(tenant_a)
    .bind(vec![11u8; 32])
    .bind(seed.user)
    .execute(tx.as_conn())
    .await;
    assert_code(bad_role, "23514");
    drop(tx);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_the_auth_lookups_work_under_the_runtime_role() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    let seed = seed_tenant(db.database(), tenant_b).await?;

    // The lookups run in a tenant transaction of another tenant. The functions must find tenant B.
    let mut tx = db.database().begin_tenant(tenant_a).await?;

    let tenant: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, status FROM auth_find_tenant($1)")
            .bind("tenant-b")
            .fetch_optional(tx.as_conn())
            .await?;
    assert_eq!(tenant.map(|row| row.0), Some(tenant_b.as_uuid()));

    let session: Option<(Uuid, Uuid, Uuid)> =
        sqlx::query_as("SELECT tenant_id, session_id, user_id FROM auth_find_session($1)")
            .bind(&seed.session_hash)
            .fetch_optional(tx.as_conn())
            .await?;
    assert_eq!(session.map(|row| row.0), Some(tenant_b.as_uuid()));
    assert_eq!(session.map(|row| row.1), Some(seed.session_id.as_uuid()));

    let token: Option<(Uuid, Uuid, String)> =
        sqlx::query_as("SELECT tenant_id, token_id, role FROM auth_find_api_token($1)")
            .bind(&seed.api_hash)
            .fetch_optional(tx.as_conn())
            .await?;
    assert_eq!(token.as_ref().map(|row| row.0), Some(tenant_b.as_uuid()));
    assert_eq!(
        token.as_ref().map(|row| row.2.clone()),
        Some("admin".to_owned())
    );

    // Unknown values return nothing.
    let unknown_session: Option<(Uuid,)> =
        sqlx::query_as("SELECT tenant_id FROM auth_find_session($1)")
            .bind(vec![0u8; 32])
            .fetch_optional(tx.as_conn())
            .await?;
    assert!(unknown_session.is_none());
    let unknown_tenant: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM auth_find_tenant($1)")
        .bind("does-not-exist")
        .fetch_optional(tx.as_conn())
        .await?;
    assert!(unknown_tenant.is_none());

    // The runtime role cannot read the other columns of tenant B directly.
    let cross: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE tenant_id = $1")
        .bind(tenant_b)
        .fetch_optional(tx.as_conn())
        .await?;
    assert!(cross.is_none());

    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}
