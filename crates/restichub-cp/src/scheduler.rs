//! The scheduler: it turns a due plan into one job for each fire time (FR-SC-01, FR-SC-02,
//! FR-SC-07, ADR-0010).
//!
//! One pass lists the tenants and processes the due plans of each active tenant in its own
//! transaction, the same tenant iteration as the expiry sweep of [`crate::maintenance`]. The select
//! locks each due plan with `SKIP LOCKED`, so two control plane instances create exactly one job
//! for a `(plan_id, fire_time)` pair (AC-SC-02).
//!
//! The scheduler creates a job row only: the kind `backup`, the agent, the repository, the plan and
//! the fire time. The row holds no secret and no compiled command; the control plane recompiles the
//! stored profile when it dispatches the job (a later part of epic #7).
//!
//! A suspended tenant and a plan whose agent is revoked create no jobs (AC-TN-03). A plan whose
//! agent is revoked still advances its fire time, so the control plane does not reprocess it on
//! every pass.

use std::sync::Arc;
use std::time::Duration;

use restichub_db::{
    AgentStatus, Database, DbError, JobId, JobKind, MissedRunPolicy, TenantId, TenantStatus, jobs,
    plans, tenants,
};
use time::OffsetDateTime;

use crate::clock::Clock;
use crate::schedule::Schedule;

/// How often the scheduler looks for a due plan (ADR-0010).
pub const SCHEDULER_INTERVAL: Duration = Duration::from_secs(5);

/// The largest number of due plans that one tenant pass claims. The bound keeps one pass short when
/// a tenant holds a large number of plans.
const PLANS_PER_TENANT: i64 = 200;

/// The largest number of jobs that one plan creates in one pass (FR-SC-07).
///
/// After a long downtime a plan can miss a large number of fire times. The cap bounds the work: the
/// pass writes a job for the first [`MAX_MISSED_RUNS`] fire times and advances the plan past `now`,
/// so the fire times beyond the cap are dropped. The plan states the rule in the documentation.
pub const MAX_MISSED_RUNS: usize = 100;

/// The lifetime of a scheduled job, counted from its fire time (FR-SC-07).
///
/// The expiry sweep of [`crate::maintenance`] moves a job that the agent did not run before this
/// deadline to the terminal `expired` status.
const SCHEDULED_JOB_LIFETIME: time::Duration = time::Duration::hours(24);

/// Runs one scheduler pass over every active tenant (FR-SC-01).
///
/// The clock is injectable, so a test drives the daylight saving cases and the missed run policies
/// without a sleep.
pub async fn run_once(db: &Database, clock: &dyn Clock) -> Result<(), DbError> {
    let now = clock.now();
    let tenant_list = {
        let mut tx = db.begin_platform().await?;
        let list = tenants::list(&mut tx).await?;
        tx.commit().await?;
        list
    };
    for tenant in tenant_list {
        // A suspended tenant creates no jobs (AC-TN-03).
        if tenant.status != TenantStatus::Active {
            continue;
        }
        if let Err(error) = process_tenant(db, tenant.id, now).await {
            tracing::warn!(tenant = %tenant.id, %error, "the scheduler pass of a tenant failed");
        }
    }
    Ok(())
}

async fn process_tenant(
    db: &Database,
    tenant: TenantId,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    let mut tx = db.begin_tenant(tenant).await?;
    let due = plans::claim_due(&mut tx, now, PLANS_PER_TENANT).await?;
    for plan in &due {
        let Ok(schedule) = Schedule::parse(&plan.cron, &plan.time_zone) else {
            // The control plane validates a plan before it writes the row, so a row that does not
            // parse is a schema the binary does not understand. Leave the fire time, so a fixed
            // row is processed.
            tracing::warn!(plan = %plan.id, "the scheduler skipped a plan with an unknown schedule");
            continue;
        };
        let Ok(next_run_at) = schedule.next_after(now) else {
            tracing::warn!(plan = %plan.id, "the scheduler could not advance a plan");
            continue;
        };

        // A revoked agent creates no jobs; the plan still advances (AC-TN-03, FR-ID-04).
        if plan.agent_status != AgentStatus::Active {
            plans::advance(&mut tx, plan.id, next_run_at, plan.next_run_at).await?;
            continue;
        }

        let missed = match schedule.fire_times_up_to(plan.next_run_at, now, MAX_MISSED_RUNS) {
            Ok(missed) => missed,
            Err(error) => {
                // Leave the fire time, so a transient failure is retried on the next pass.
                tracing::warn!(plan = %plan.id, %error, "the scheduler could not compute the missed fire times");
                continue;
            }
        };
        for fire_time in select_fires(plan.missed_run_policy, &missed) {
            let job = jobs::NewJob {
                id: JobId::new(),
                kind: JobKind::Backup,
                agent_id: plan.agent_id,
                repository_id: Some(plan.repository_id),
                plan_id: Some(plan.id),
                fire_time: Some(fire_time),
                expires_at: Some(fire_time + SCHEDULED_JOB_LIFETIME),
                location_fingerprint: None,
                created_by: None,
            };
            jobs::create_scheduled(&mut tx, &job).await?;
        }

        let last_fire_time = missed.last().copied().unwrap_or(plan.next_run_at);
        plans::advance(&mut tx, plan.id, next_run_at, last_fire_time).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// The fire times that the missed run policy keeps (FR-SC-07).
fn select_fires(policy: MissedRunPolicy, missed: &[OffsetDateTime]) -> Vec<OffsetDateTime> {
    match policy {
        MissedRunPolicy::RunAll => missed.to_vec(),
        MissedRunPolicy::LatestOnly => missed.last().copied().into_iter().collect(),
        MissedRunPolicy::Skip => Vec::new(),
    }
}

/// Starts the periodic scheduler pass.
pub fn spawn(
    db: Database,
    clock: Arc<dyn Clock>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        // The first tick of `interval` fires immediately. Skip it.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(error) = run_once(&db, clock.as_ref()).await {
                tracing::warn!(%error, "the scheduler pass failed");
            }
        }
    })
}
