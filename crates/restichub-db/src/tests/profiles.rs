use super::*;
use crate::test_util::test_database;

/// The composite foreign key refuses a repository of another tenant (FR-TN-01).
#[tokio::test]
async fn db_a_profile_refuses_a_foreign_tenant_repository() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;
    seed_tenant(db.database(), tenant_b).await?;

    let foreign_repository = {
        let mut tx = db.database().begin_tenant(tenant_b).await?;
        let (id,): (uuid::Uuid,) = sqlx::query_as("SELECT id FROM repositories")
            .fetch_one(tx.as_conn())
            .await?;
        tx.commit().await?;
        id
    };

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let foreign = sqlx::query(
        "INSERT INTO profiles (id, tenant_id, name, repository_id, paths_all) \
         VALUES ($1, $2, 'foreign-repo', $3, ARRAY['/srv'])",
    )
    .bind(crate::ProfileId::new())
    .bind(tenant_a)
    .bind(foreign_repository)
    .execute(tx.as_conn())
    .await;
    assert_code(foreign, "23503");
    drop(tx);

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
        "INSERT INTO profiles (id, tenant_id, name, repository_id, paths_all) \
         VALUES ($1, $2, 'profile', (SELECT id FROM repositories), ARRAY['/srv'])",
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
