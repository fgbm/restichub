//! The scheduler: it turns a due plan into one job for each fire time (FR-SC-01, FR-SC-02,
//! FR-SC-07, ADR-0010).
//!
//! One pass lists the tenants and processes the due plans of each active tenant in its own
//! transaction, the same tenant iteration as the expiry sweep of [`crate::maintenance`]. The select
//! locks each due plan with `SKIP LOCKED`, so two control plane instances create exactly one job
//! for a `(plan_id, fire_time)` pair (AC-SC-02).
//!
//! The scheduler creates a job row only: the kind `backup`, the agent, the repository, the plan and
//! the fire time. The row holds no secret and no compiled command; the dispatcher of
//! [`crate::queue`] recompiles the stored profile when it sends the job.
//!
//! The same pass runs the maintenance schedules of the repositories ([`crate::maintenance_scheduler`]).
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
use crate::queue::RECONNECT_GRACE;
use crate::schedule::{Schedule, ScheduleError};

/// How often the scheduler looks for a due plan (ADR-0010).
pub const SCHEDULER_INTERVAL: Duration = Duration::from_secs(5);

/// The largest number of due plans that one tenant pass claims. The bound keeps one pass short when
/// a tenant holds a large number of plans.
const PLANS_PER_TENANT: i64 = 200;

/// The largest number of jobs that one plan creates in one pass (FR-SC-07).
///
/// After a long downtime a plan can miss a large number of fire times. The cap bounds the work: the
/// pass writes a job for the latest [`MAX_MISSED_RUNS`] fire times that a job still outlives and
/// advances the plan past `now`, so the older fire times are dropped. The plan states the rule in
/// the documentation.
pub const MAX_MISSED_RUNS: usize = 100;

/// The lifetime of a scheduled job, counted from its fire time (FR-SC-07).
///
/// The scheduler creates no job whose fire time plus this lifetime is not after `now`, because such
/// a job would expire at once. The expiry sweep of [`crate::maintenance`] moves a job that the agent
/// did not run before this deadline to the terminal `expired` status.
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
        if let Err(error) = crate::maintenance_scheduler::process_tenant(db, tenant.id, now).await {
            tracing::warn!(tenant = %tenant.id, %error, "the maintenance pass of a tenant failed");
        }
    }
    Ok(())
}

/// Marks the undo point of one plan inside the tenant pass.
const PLAN_SAVEPOINT: &str = "scheduler_plan";

/// The grace window around a fire time. A fire time within it is on time and always runs; the
/// missed run policy applies only to the older fire times (FR-SC-07).
///
/// The window is a small multiple of the pass interval, so a plan whose fire time just passed runs
/// even when the pass is a little late.
pub const ON_TIME_GRACE: time::Duration = time::Duration::seconds(60);

async fn process_tenant(
    db: &Database,
    tenant: TenantId,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    let mut tx = db.begin_tenant(tenant).await?;
    let due = plans::claim_due(&mut tx, now, PLANS_PER_TENANT).await?;
    for plan in &due {
        // One plan runs in its own savepoint: a failure rolls back this plan only, and the pass
        // continues with the next one.
        tx.savepoint(PLAN_SAVEPOINT).await?;
        match process_plan(&mut tx, plan, now).await {
            Ok(()) => tx.release_savepoint(PLAN_SAVEPOINT).await?,
            Err(error) => {
                tx.rollback_to_savepoint(PLAN_SAVEPOINT).await?;
                tracing::warn!(
                    plan = %plan.id,
                    code = error.code(),
                    %error,
                    "the scheduler skipped a plan"
                );
            }
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Creates the jobs of one due plan in the open transaction (FR-SC-02, FR-SC-07).
async fn process_plan(
    tx: &mut restichub_db::TenantTx<'_>,
    plan: &restichub_db::plans::DuePlan,
    now: OffsetDateTime,
) -> Result<(), PlanError> {
    let schedule = Schedule::parse(&plan.cron, &plan.time_zone)?;
    let next_run_at = schedule.next_after(now)?;

    // A revoked agent creates no jobs; the plan still advances (AC-TN-03, FR-ID-04).
    if plan.agent_status != AgentStatus::Active {
        plans::advance(tx, plan.id, next_run_at, plan.next_run_at).await?;
        return Ok(());
    }

    let missed = schedule.fire_times_up_to(
        plan.next_run_at,
        now,
        MAX_MISSED_RUNS,
        SCHEDULED_JOB_LIFETIME,
    )?;
    // A `skip` job is for the on-time run only. A host that reconnects in the grace of ADR-0052
    // runs it; the expiry sweep ends it otherwise (FR-SC-07).
    let lifetime = match plan.missed_run_policy {
        MissedRunPolicy::Skip => RECONNECT_GRACE,
        MissedRunPolicy::RunAll | MissedRunPolicy::LatestOnly => SCHEDULED_JOB_LIFETIME,
    };
    let mut earliest_created: Option<OffsetDateTime> = None;
    for fire_time in select_fires(plan.missed_run_policy, &missed, now) {
        let job = jobs::NewJob {
            id: JobId::new(),
            kind: JobKind::Backup,
            agent_id: plan.agent_id,
            repository_id: Some(plan.repository_id),
            plan_id: Some(plan.id),
            fire_time: Some(fire_time),
            expires_at: Some(fire_time + lifetime),
            location_fingerprint: None,
            created_by: None,
        };
        if jobs::create_scheduled(tx, &job).await?.is_some() {
            // `select_fires` lists the on-time fire time first, so the first one is not the oldest.
            earliest_created = Some(earliest_created.map_or(fire_time, |e| e.min(fire_time)));
        }
    }
    // The jobs that an offline host did not take yet are older than the new ones. The latest run
    // replaces them, so the host does not run them back to back when it returns (FR-SC-07). A job
    // older than its lifetime expires in the sweep, so the search stops there.
    if plan.missed_run_policy == MissedRunPolicy::LatestOnly
        && let Some(earliest) = earliest_created
    {
        jobs::expire_superseded(tx, plan.id, earliest, now - SCHEDULED_JOB_LIFETIME, now).await?;
    }

    let last_fire_time = missed.last().copied().unwrap_or(plan.next_run_at);
    plans::advance(tx, plan.id, next_run_at, last_fire_time).await?;
    Ok(())
}

/// The failure of one plan inside a tenant pass (FR-SC-02).
///
/// A schedule that the binary cannot compute and a database error are both per-plan failures: the
/// savepoint of the plan rolls back its work and the pass logs the plan and the code.
#[derive(Debug, thiserror::Error)]
enum PlanError {
    #[error("{0}")]
    Db(#[from] DbError),
    #[error("{0}")]
    Schedule(#[from] ScheduleError),
}

impl PlanError {
    /// Gives a stable short label for the log line.
    fn code(&self) -> &'static str {
        match self {
            Self::Db(error) => error.code(),
            Self::Schedule(_) => "schedule",
        }
    }
}

/// The fire times that the scheduler creates a job for (FR-SC-07).
///
/// A fire time within [`ON_TIME_GRACE`] of `now` is on time and always runs. The missed run policy
/// applies only to the older fire times: `run_all` keeps them all, `latest_only` keeps the latest,
/// and `skip` keeps none. The on-time fire times come first.
fn select_fires(
    policy: MissedRunPolicy,
    missed: &[OffsetDateTime],
    now: OffsetDateTime,
) -> Vec<OffsetDateTime> {
    let (on_time, older): (Vec<_>, Vec<_>) = missed
        .iter()
        .copied()
        .partition(|fire_time| *fire_time + ON_TIME_GRACE > now);
    let mut fires = on_time;
    match policy {
        MissedRunPolicy::RunAll => fires.extend(older),
        MissedRunPolicy::LatestOnly => fires.extend(older.last().copied()),
        MissedRunPolicy::Skip => {}
    }
    fires
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
