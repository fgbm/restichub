//! Alerts and notification channels (FR-MN-01, FR-MN-03, FR-MN-04, AC-MN-01, AC-MN-02, ADR-0036).
//!
//! A loop in every control plane instance evaluates the alert rules of each active tenant and sends
//! the queued messages. The evaluation takes an advisory lock for each tenant and the sender claims
//! each message with `SKIP LOCKED`, so a second instance neither repeats an alert nor a message.
//! The loop is a control plane task, not job scheduling (ADR-0002).

pub mod deliver;
pub mod email;
mod error_labels;
pub mod evaluate;
pub mod message;
pub mod rpo;
pub mod secret;
pub mod sender;
pub mod settings;
pub mod signature;
pub mod ssrf;
pub mod text;
pub mod transport;
pub mod webhook;

use std::sync::Arc;
use std::time::{Duration, Instant};

use restichub_db::{Database, DbError, PLATFORM_TENANT_ID, TenantStatus, tenants};
use restichub_secrets::KekProvider;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::clock::Clock;
use crate::queue::AgentView;
use crate::supervisor::Supervisor;
use sender::Sender;

/// How often the loop evaluates and sends. A violation therefore reaches a channel well inside
/// the five minutes of AC-MN-01.
pub const INTERVAL: Duration = Duration::from_secs(30);

/// The most tenants that one pass handles at the same time. A slow target of one tenant holds one
/// slot, so it cannot delay the messages of the others.
pub const CONCURRENCY: usize = 8;

/// The longest time that the evaluation and the delivery of one tenant may take together. The
/// sends of a tenant run in turn: a batch of ten at the longest send of 30 seconds is 300 seconds.
pub const TENANT_BUDGET: Duration = Duration::from_secs(330);

/// After this time a pass starts no further tenant. The order of the list rotates with the pass,
/// so no tenant is always the last one.
pub const PASS_BUDGET: Duration = Duration::from_secs(900);

/// What one pass may do. A test sets its own values; the loop uses [`PassLimits::default`].
#[derive(Debug, Clone, Copy)]
pub struct PassLimits {
    /// How many tenants the pass handles at the same time.
    pub concurrency: usize,
    /// The longest time of the evaluation and the delivery of one tenant together.
    pub tenant_budget: Duration,
    /// The position in the tenant list that the pass starts at.
    pub offset: usize,
}

impl Default for PassLimits {
    fn default() -> Self {
        Self {
            concurrency: CONCURRENCY,
            tenant_budget: TENANT_BUDGET,
            offset: 0,
        }
    }
}

/// Runs one evaluation and one send for every active tenant, up to [`CONCURRENCY`] tenants at the
/// same time. Each tenant reads the clock when it starts, so a tenant that waits for a slot does
/// not evaluate with a time from before the wait.
pub async fn run_once(
    db: &Database,
    sender: &Sender,
    provider: Arc<dyn KekProvider>,
    clock: Arc<dyn Clock>,
    view: Arc<AgentView>,
) -> Result<(), DbError> {
    run_once_with(db, sender, provider, clock, view, PassLimits::default()).await
}

/// Like [`run_once`], with the limits chosen by the caller. A rotation of the tenant list by
/// `offset` lets a later pass start with the tenants that the earlier pass left out.
pub async fn run_once_with(
    db: &Database,
    sender: &Sender,
    provider: Arc<dyn KekProvider>,
    clock: Arc<dyn Clock>,
    view: Arc<AgentView>,
    limits: PassLimits,
) -> Result<(), DbError> {
    let PassLimits {
        concurrency,
        tenant_budget,
        offset,
    } = limits;
    let mut tenant_list = {
        let mut tx = db.begin_platform().await?;
        let list = tenants::list(&mut tx).await?;
        tx.commit().await?;
        list
    };
    // A suspended tenant gets no evaluation; the platform tenant holds no backup data.
    tenant_list
        .retain(|tenant| tenant.status == TenantStatus::Active && tenant.id != PLATFORM_TENANT_ID);
    if !tenant_list.is_empty() {
        let shift = offset % tenant_list.len();
        tenant_list.rotate_left(shift);
    }

    let started = Instant::now();
    let slots = Arc::new(Semaphore::new(concurrency.max(1)));
    let mut tasks = JoinSet::new();
    for tenant in tenant_list {
        let Ok(permit) = slots.clone().acquire_owned().await else {
            break;
        };
        if started.elapsed() > PASS_BUDGET {
            tracing::warn!("the alert pass ran out of time, so the rest waits for the next pass");
            break;
        }
        let (db, sender, provider, clock, view) = (
            db.clone(),
            sender.clone(),
            provider.clone(),
            clock.clone(),
            view.clone(),
        );
        tasks.spawn(async move {
            let _permit = permit;
            let deadline = tokio::time::Instant::now() + tenant_budget;
            let work = handle_tenant(
                &db,
                &sender,
                provider.as_ref(),
                clock.as_ref(),
                &view,
                &tenant,
                deadline,
            );
            if tokio::time::timeout(tenant_budget, work).await.is_err() {
                tracing::warn!(tenant = %tenant.id, "the alert work of a tenant ran out of time");
            }
        });
    }
    while let Some(joined) = tasks.join_next().await {
        if let Err(error) = joined {
            tracing::warn!(%error, "an alert task of a tenant ended abnormally");
        }
    }
    Ok(())
}

async fn handle_tenant(
    db: &Database,
    sender: &Sender,
    provider: &dyn KekProvider,
    clock: &dyn Clock,
    view: &AgentView,
    tenant: &restichub_db::tenants::Tenant,
    deadline: tokio::time::Instant,
) {
    match evaluate::evaluate_tenant(db, tenant.id, clock.now(), view).await {
        Ok(report) => {
            if let Some(events) = sender.events()
                && (report.opened > 0 || report.resolved > 0)
            {
                events.alert_changed(tenant.id);
            }
        }
        Err(error) if error.is_statement_timeout() => {
            // Every pass fails the same way until the history shrinks, and no alert of the tenant
            // opens or closes in the meantime (AC-MN-01).
            tracing::error!(
                tenant = %tenant.id,
                seconds = evaluate::EVALUATION_STATEMENT_TIMEOUT.whole_seconds(),
                %error,
                "the alert evaluation of a tenant passed its statement timeout"
            );
        }
        Err(error) => {
            tracing::warn!(tenant = %tenant.id, %error, "the alert evaluation of a tenant failed");
        }
    }
    if let Err(error) = deliver::deliver_tenant(
        db,
        sender,
        provider,
        clock,
        tenant.id,
        &tenant.name,
        deadline,
    )
    .await
    {
        tracing::warn!(tenant = %tenant.id, %error, "the alert delivery of a tenant failed");
    }
}

/// Starts the periodic loop.
pub fn spawn(
    supervisor: &Supervisor,
    db: Database,
    sender: Sender,
    provider: Arc<dyn KekProvider>,
    clock: Arc<dyn Clock>,
    view: Arc<AgentView>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    supervisor.spawn_loop("alerts", move |start| {
        let (db, sender) = (db.clone(), sender.clone());
        let (provider, clock, view) = (provider.clone(), clock.clone(), view.clone());
        async move {
            let mut ticker = tokio::time::interval(interval);
            // A pass that takes longer than the interval must not queue the missed ticks.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            start.skip_first_tick(&mut ticker).await;
            let mut pass = 0_usize;
            loop {
                ticker.tick().await;
                if let Err(error) = run_once_with(
                    &db,
                    &sender,
                    provider.clone(),
                    clock.clone(),
                    view.clone(),
                    PassLimits {
                        offset: pass,
                        ..PassLimits::default()
                    },
                )
                .await
                {
                    tracing::warn!(%error, "the alert pass failed");
                }
                pass = pass.wrapping_add(1);
            }
        }
    })
}
