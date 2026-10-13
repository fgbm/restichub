//! The periodic maintenance of the control plane.
//!
//! The task removes the sessions past the absolute lifetime and the expired and used setup tokens.
//! It is a maintenance task of the control plane, not job scheduling (ADR-0002). It runs in one
//! process; two instances both run it and the operations are idempotent.

use std::sync::Arc;
use std::time::Duration;

use restichub_db::{Database, DbError, PLATFORM_TENANT_ID, sessions, setup, tenants};
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::job_signing::{JobSigningError, JobSigningService};
use crate::rate_limit::RateLimiter;

pub const CLEANUP_INTERVAL: Duration = Duration::from_secs(3600);

pub const LIMITER_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

pub const SIGNING_KEY_RETIRE_INTERVAL: Duration = Duration::from_secs(3600);

/// Removes the expired sessions and the expired and used setup tokens.
///
/// The runtime role cannot cross the tenant boundary, so the function lists the tenants and cleans
/// each tenant in its own transaction. A failure of one tenant is logged and does not stop the
/// other tenants.
pub async fn cleanup(db: &Database, now: OffsetDateTime) -> Result<(), DbError> {
    let tenant_list = {
        let mut tx = db.begin_platform().await?;
        let list = tenants::list(&mut tx).await?;
        tx.commit().await?;
        list
    };
    for tenant in tenant_list {
        if let Err(error) = cleanup_tenant(db, tenant.id, now).await {
            tracing::warn!(tenant = %tenant.id, %error, "the maintenance cleanup of a tenant failed");
        }
    }
    if let Err(error) = cleanup_setup(db, now).await {
        tracing::warn!(%error, "the maintenance cleanup of the setup tokens failed");
    }
    Ok(())
}

async fn cleanup_tenant(
    db: &Database,
    tenant: restichub_db::TenantId,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    let mut tx = db.begin_tenant(tenant).await?;
    sessions::delete_expired(&mut tx, now).await?;
    tx.commit().await?;
    Ok(())
}

async fn cleanup_setup(db: &Database, now: OffsetDateTime) -> Result<(), DbError> {
    let mut tx = db.begin_tenant(PLATFORM_TENANT_ID).await?;
    setup::delete_expired_or_used(&mut tx, now).await?;
    tx.commit().await?;
    Ok(())
}

/// Retires the job signing keys whose rotation overlap ended (ADR-0026).
///
/// The clock is injectable, so a test advances the overlap window without a sleep.
pub async fn retire_signing_keys(
    db: &Database,
    signing: &JobSigningService,
    clock: &dyn Clock,
) -> Result<(), JobSigningError> {
    signing.retire_due(db, clock.now()).await?;
    Ok(())
}

/// Starts the periodic retirement of the job signing keys.
///
/// The task bounds the rotation overlap without a manual call (ADR-0026). It runs in one process;
/// two instances both run it and the retirement is idempotent.
pub fn spawn_signing_key_retirement(
    db: Database,
    signing: JobSigningService,
    clock: Arc<dyn Clock>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // The first tick of `interval` fires immediately. Skip it.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(error) = retire_signing_keys(&db, &signing, clock.as_ref()).await {
                tracing::warn!(%error, "the job signing key retirement failed");
            }
        }
    })
}

/// Starts the periodic maintenance task.
///
/// The task waits one interval before the first run, because the server prepares the setup token at
/// start.
pub fn spawn(db: Database, interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // The first tick of `interval` fires immediately. Skip it.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(error) = cleanup(&db, OffsetDateTime::now_utc()).await {
                tracing::warn!(%error, "the maintenance cleanup failed");
            }
        }
    })
}

/// Starts the periodic rate limiter sweep.
///
/// The sweep removes the full and the idle buckets of the user map, the IP map and the account
/// create maps.
pub fn spawn_limiter_sweep(
    user_limiter: Arc<RateLimiter>,
    ip_limiter: Arc<RateLimiter>,
    create_user_limiter: Arc<RateLimiter>,
    create_user_tenant_limiter: Arc<RateLimiter>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let now = std::time::Instant::now();
            user_limiter.sweep(now);
            ip_limiter.sweep(now);
            create_user_limiter.sweep(now);
            create_user_tenant_limiter.sweep(now);
        }
    })
}
