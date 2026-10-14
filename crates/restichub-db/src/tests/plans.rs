use super::*;
use crate::test_util::test_database;

/// The composite foreign keys refuse a profile or an agent of another tenant (FR-TN-01).
#[tokio::test]
async fn db_a_plan_refuses_a_foreign_tenant_profile_or_agent() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;
    seed_tenant(db.database(), tenant_b).await?;

    let (foreign_profile, foreign_agent) = {
        let mut tx = db.database().begin_tenant(tenant_b).await?;
        let profile: (uuid::Uuid,) = sqlx::query_as("SELECT id FROM profiles")
            .fetch_one(tx.as_conn())
            .await?;
        let agent: (uuid::Uuid,) = sqlx::query_as("SELECT id FROM agents")
            .fetch_one(tx.as_conn())
            .await?;
        tx.commit().await?;
        (profile.0, agent.0)
    };

    let (local_profile, local_agent) = {
        let mut tx = db.database().begin_tenant(tenant_a).await?;
        let profile: (uuid::Uuid,) = sqlx::query_as("SELECT id FROM profiles")
            .fetch_one(tx.as_conn())
            .await?;
        let agent: (uuid::Uuid,) = sqlx::query_as("SELECT id FROM agents")
            .fetch_one(tx.as_conn())
            .await?;
        tx.commit().await?;
        (profile.0, agent.0)
    };

    {
        let mut tx = db.database().begin_tenant(tenant_a).await?;
        let foreign_profile_row = sqlx::query(
            "INSERT INTO plans (id, tenant_id, name, profile_id, agent_id, cron, time_zone, \
                    missed_run_policy) \
             VALUES ($1, $2, 'foreign-profile', $3, $4, '0 2 * * *', 'UTC', 'run_all')",
        )
        .bind(crate::PlanId::new())
        .bind(tenant_a)
        .bind(foreign_profile)
        .bind(local_agent)
        .execute(tx.as_conn())
        .await;
        assert_code(foreign_profile_row, "23503");
    }

    {
        let mut tx = db.database().begin_tenant(tenant_a).await?;
        let foreign_agent_row = sqlx::query(
            "INSERT INTO plans (id, tenant_id, name, profile_id, agent_id, cron, time_zone, \
                    missed_run_policy) \
             VALUES ($1, $2, 'foreign-agent', $3, $4, '0 2 * * *', 'UTC', 'run_all')",
        )
        .bind(crate::PlanId::new())
        .bind(tenant_a)
        .bind(local_profile)
        .bind(foreign_agent)
        .execute(tx.as_conn())
        .await;
        assert_code(foreign_agent_row, "23503");
    }

    db.cleanup().await;
    Ok(())
}

/// The name is unique for the tenant (FR-SC-01).
#[tokio::test]
async fn db_a_plan_name_is_unique_for_the_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let duplicate = sqlx::query(
        "INSERT INTO plans (id, tenant_id, name, profile_id, agent_id, cron, time_zone, \
                missed_run_policy) \
         VALUES ($1, $2, 'plan', (SELECT id FROM profiles), (SELECT id FROM agents), \
                 '0 2 * * *', 'UTC', 'run_all')",
    )
    .bind(crate::PlanId::new())
    .bind(tenant_a)
    .execute(tx.as_conn())
    .await;
    assert_code(duplicate, "23505");
    drop(tx);

    db.cleanup().await;
    Ok(())
}

/// The unique key `(plan_id, fire_time)` makes the job creation idempotent (FR-SC-02, AC-SC-02).
#[tokio::test]
async fn db_a_scheduled_job_is_unique_for_a_plan_and_fire_time() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let plan: (uuid::Uuid,) = sqlx::query_as("SELECT id FROM plans")
        .fetch_one(tx.as_conn())
        .await?;
    let fire_time = time::OffsetDateTime::now_utc();
    let insert = || {
        sqlx::query(
            "INSERT INTO jobs (id, tenant_id, kind, agent_id, repository_id, plan_id, fire_time, \
                    status) \
             VALUES ($1, $2, 'backup', (SELECT id FROM agents), (SELECT id FROM repositories), \
                     $3, $4, 'queued')",
        )
        .bind(crate::JobId::new())
        .bind(tenant_a)
        .bind(plan.0)
        .bind(fire_time)
    };

    insert().execute(tx.as_conn()).await?;
    let duplicate = insert().execute(tx.as_conn()).await;
    assert_code(duplicate, "23505");
    drop(tx);

    db.cleanup().await;
    Ok(())
}
