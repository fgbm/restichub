//! One tenant with a slow database does not hold the dispatch of another tenant (AC-NFR-01).
//!
//! A pass of one tenant ends with a statement timeout, counts as a failed pass for the reconnect
//! grace and leaves the dispatcher to the next tenant of the installation.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use serde_json::json;
use uuid::Uuid;

use queue_support::{Env, test_queue};
use restichub_db::TenantId;

/// Creates a tenant that holds a `sent` job with a cancel request whose lease passed, and returns
/// the identifier of that job. The pass of the tenant updates the row without `SKIP LOCKED`, so a
/// lock on it stops the pass.
async fn seed_slow_tenant(env: &Env) -> (TenantId, Uuid) {
    let tenant = common::create_tenant_with_key(env.db.database(), "slow-tenant").await;
    let job = Uuid::now_v7();
    let mut tx = env.db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot scope the tenant");
    let agent = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO agents (id, tenant_id, hostname, os, arch, agent_version, \
                             certificate_serial, certificate_expires_at, status) \
         VALUES ($1, $2, 'slow-host', 'linux', 'amd64', '0.1.0', decode(md5($1::text), 'hex'), \
                 now() + interval '30 days', 'active')",
    )
    .bind(agent)
    .bind(tenant)
    .execute(&mut *tx)
    .await
    .expect("cannot create the agent");
    sqlx::query(
        "INSERT INTO jobs (id, tenant_id, kind, agent_id, status, cancel_requested, lease_until, \
                           created_at, updated_at) \
         VALUES ($1, $2, 'backup', $3, 'sent', true, now() - interval '1 minute', \
                 now() - interval '2 minutes', now())",
    )
    .bind(job)
    .bind(tenant)
    .bind(agent)
    .execute(&mut *tx)
    .await
    .expect("cannot create the job");
    tx.commit().await.expect("cannot commit");
    (tenant, job)
}

#[tokio::test]
async fn ac_nfr_01_a_slow_tenant_does_not_hold_the_dispatch_of_another_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("fast-host").await;
    let repository = env.create_repository(agent.id, "canary-password").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job = env.run_plan(plan).await;

    let (slow, blocked) = seed_slow_tenant(&env).await;
    // The pass of the slow tenant waits here until its statement timeout.
    let mut held = env
        .db
        .owner_pool()
        .begin()
        .await
        .expect("cannot begin the lock");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(slow.to_string())
        .execute(&mut *held)
        .await
        .expect("cannot scope the tenant");
    let locked = sqlx::query("SELECT id FROM jobs WHERE id = $1 FOR UPDATE")
        .bind(blocked)
        .execute(&mut *held)
        .await
        .expect("cannot lock the job");
    assert_eq!(locked.rows_affected(), 1, "the job row must be locked");

    tokio::time::timeout(Duration::from_secs(60), env.pass())
        .await
        .expect("the pass of the slow tenant must not hold the dispatcher");

    let signed = agent.job().await;
    let delivered = env.verify(&signed, &agent).await;
    assert_eq!(
        delivered.job_id,
        job.to_string(),
        "the job of the other tenant goes out while the slow tenant fails"
    );

    held.rollback().await.expect("cannot release the lock");
    env.db.cleanup().await;
}
