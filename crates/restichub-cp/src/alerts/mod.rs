//! Alerts and notification channels (FR-MN-01, FR-MN-03, FR-MN-04, AC-MN-01, AC-MN-02, ADR-0035).
//!
//! A loop in every control plane instance evaluates the alert rules of each active tenant and sends
//! the queued messages. The evaluation takes an advisory lock for each tenant and the sender claims
//! each message with `SKIP LOCKED`, so a second instance neither repeats an alert nor a message.
//! The loop is a control plane task, not job scheduling (ADR-0002).

pub mod deliver;
pub mod email;
pub mod evaluate;
pub mod message;
pub mod rpo;
pub mod secret;
pub mod sender;
pub mod settings;
pub mod signature;
pub mod ssrf;
pub mod transport;
pub mod webhook;

use std::sync::Arc;
use std::time::Duration;

use restichub_db::{Database, DbError, PLATFORM_TENANT_ID, TenantStatus, tenants};
use restichub_secrets::KekProvider;
use time::OffsetDateTime;

use crate::clock::Clock;
use sender::Sender;

/// How often the loop evaluates and sends. A violation therefore reaches a channel well inside
/// the five minutes of AC-MN-01.
pub const INTERVAL: Duration = Duration::from_secs(30);

/// Runs one evaluation and one send for every active tenant.
pub async fn run_once(
    db: &Database,
    sender: &Sender,
    provider: &dyn KekProvider,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    let tenant_list = {
        let mut tx = db.begin_platform().await?;
        let list = tenants::list(&mut tx).await?;
        tx.commit().await?;
        list
    };
    for tenant in tenant_list {
        // A suspended tenant gets no evaluation; the platform tenant holds no backup data.
        if tenant.status != TenantStatus::Active || tenant.id == PLATFORM_TENANT_ID {
            continue;
        }
        if let Err(error) = evaluate::evaluate_tenant(db, tenant.id, now).await {
            tracing::warn!(tenant = %tenant.id, %error, "the alert evaluation of a tenant failed");
        }
        if let Err(error) =
            deliver::deliver_tenant(db, sender, provider, tenant.id, &tenant.name, now).await
        {
            tracing::warn!(tenant = %tenant.id, %error, "the alert delivery of a tenant failed");
        }
    }
    Ok(())
}

/// Starts the periodic loop.
pub fn spawn(
    db: Database,
    sender: Sender,
    provider: Arc<dyn KekProvider>,
    clock: Arc<dyn Clock>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // The first tick of `interval` fires immediately. Skip it.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(error) = run_once(&db, &sender, provider.as_ref(), clock.now()).await {
                tracing::warn!(%error, "the alert pass failed");
            }
        }
    })
}
