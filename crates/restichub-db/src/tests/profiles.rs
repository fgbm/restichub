use super::*;
use crate::test_util::test_database;

/// A profile holds no repository of its own (ADR-0047).
#[tokio::test]
async fn db_a_profile_holds_no_repository() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let column: Option<(String,)> = sqlx::query_as(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_name = 'profiles' AND column_name = 'repository_id'",
    )
    .fetch_optional(tx.as_conn())
    .await?;
    assert!(column.is_none(), "the profile still holds a repository");
    tx.commit().await?;

    db.cleanup().await;
    Ok(())
}

/// The name is unique for the tenant (FR-PF-01).
#[tokio::test]
async fn db_a_profile_name_is_unique_for_the_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let duplicate = sqlx::query(
        "INSERT INTO profiles (id, tenant_id, name, paths_all) \
         VALUES ($1, $2, 'profile', ARRAY['/srv'])",
    )
    .bind(crate::ProfileId::new())
    .bind(tenant_a)
    .execute(tx.as_conn())
    .await;
    assert_code(duplicate, "23505");
    drop(tx);

    db.cleanup().await;
    Ok(())
}
