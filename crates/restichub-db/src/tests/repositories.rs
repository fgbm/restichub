use super::*;
use crate::test_util::test_database;

/// The backend settings check refuses a mixed row (FR-RP-01).
#[tokio::test]
async fn db_a_repository_refuses_the_settings_of_another_backend() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    // An S3 backend that names no bucket must fail.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let missing_bucket = sqlx::query(
        "INSERT INTO repositories (id, tenant_id, name, backend, s3_endpoint, password_secret_id) \
         VALUES ($1, $2, 's3-no-bucket', 's3', 'https://s3.example.net', \
                 (SELECT id FROM secrets WHERE tenant_id = $2))",
    )
    .bind(crate::RepositoryId::new())
    .bind(tenant_a)
    .execute(tx.as_conn())
    .await;
    assert_code(missing_bucket, "23514");
    drop(tx);

    // A local backend that also carries an S3 endpoint.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let mixed = sqlx::query(
        "INSERT INTO repositories \
         (id, tenant_id, name, backend, local_path, s3_endpoint, password_secret_id) \
         VALUES ($1, $2, 'mixed', 'local', '/mnt/backup', 'https://s3.example.net', \
                 (SELECT id FROM secrets WHERE tenant_id = $2))",
    )
    .bind(crate::RepositoryId::new())
    .bind(tenant_a)
    .execute(tx.as_conn())
    .await;
    assert_code(mixed, "23514");
    drop(tx);

    db.cleanup().await;
    Ok(())
}

/// A queued backup does not lock the repository location; a maintenance job or a sent backup does
/// (FR-SC-04).
#[tokio::test]
async fn db_a_queued_backup_does_not_lock_the_repository() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    // A repository that holds one queued backup only.
    let repository = crate::RepositoryId::new();
    sqlx::query(
        "INSERT INTO repositories \
         (id, tenant_id, name, backend, local_path, password_secret_id, maintenance_agent_id) \
         VALUES ($1, $2, 'backup-only', 'local', '/mnt/backup/only', \
                 (SELECT id FROM secrets WHERE tenant_id = $2), \
                 (SELECT id FROM agents WHERE tenant_id = $2))",
    )
    .bind(repository)
    .bind(tenant_a)
    .execute(tx.as_conn())
    .await?;
    sqlx::query(
        "INSERT INTO jobs (id, tenant_id, kind, agent_id, repository_id, status) \
         VALUES ($1, $2, 'backup', (SELECT id FROM agents WHERE tenant_id = $2), $3, 'queued')",
    )
    .bind(crate::JobId::new())
    .bind(tenant_a)
    .bind(repository)
    .execute(tx.as_conn())
    .await?;

    assert!(
        !crate::jobs::has_active_for_repository(&mut tx, repository).await?,
        "a queued backup does not lock the repository"
    );

    sqlx::query("UPDATE jobs SET status = 'sent' WHERE repository_id = $1 AND kind = 'backup'")
        .bind(repository)
        .execute(tx.as_conn())
        .await?;
    assert!(
        crate::jobs::has_active_for_repository(&mut tx, repository).await?,
        "a sent backup locks the repository"
    );
    tx.commit().await?;

    db.cleanup().await;
    Ok(())
}

/// The composite foreign key refuses a maintenance agent of another tenant (FR-RP-08).
#[tokio::test]
async fn db_a_repository_refuses_a_foreign_tenant_agent() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;
    seed_tenant(db.database(), tenant_b).await?;

    let foreign_agent = {
        let mut tx = db.database().begin_tenant(tenant_b).await?;
        let (id,): (uuid::Uuid,) = sqlx::query_as("SELECT id FROM agents")
            .fetch_one(tx.as_conn())
            .await?;
        tx.commit().await?;
        id
    };

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let foreign = sqlx::query(
        "INSERT INTO repositories \
         (id, tenant_id, name, backend, local_path, password_secret_id, maintenance_agent_id) \
         VALUES ($1, $2, 'foreign-agent', 'local', '/mnt/backup', \
                 (SELECT id FROM secrets WHERE tenant_id = $2), $3)",
    )
    .bind(crate::RepositoryId::new())
    .bind(tenant_a)
    .bind(foreign_agent)
    .execute(tx.as_conn())
    .await;
    assert_code(foreign, "23503");
    drop(tx);

    db.cleanup().await;
    Ok(())
}
