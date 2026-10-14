//! The scheduler of the repository maintenance: it turns a due maintenance schedule into one job
//! for the maintenance agent (FR-RP-08, FR-SC-02, FR-SC-04, ADR-0034).
//!
//! The pass runs in the transaction of the tenant after the plan pass of [`crate::scheduler`]. It
//! creates the default schedules of a repository that has none, sets the first fire time of an
//! unarmed schedule and creates the jobs of the due schedules. The select locks each due row with
//! `SKIP LOCKED`, and the unique key `(maintenance_schedule_id, fire_time, attempt)` keeps the
//! create idempotent, so two control plane instances create one job for a fire time.
//!
//! A repository without a maintenance agent, with a revoked one, or that is not initialized gets no
//! job; its schedule still advances. A repository that holds an exclusive job waits: the
//! schedule stays due, and the next pass creates the job after the other job ends.

use restichub_db::jobs::{self, NewJob};
use restichub_db::maintenance::{self as schedules, DueMaintenance, NewMaintenanceSchedule};
use restichub_db::queue as queue_repo;
use restichub_db::{AgentStatus, Database, DbError, JobId, MaintenanceTask, TenantId, TenantTx};
use time::OffsetDateTime;

use crate::schedule::{Schedule, ScheduleError};

/// The default cron expression of a task: the retention cycle on Sunday at 03:00 and the check on
/// the first day of the month at 04:00.
pub fn default_cron(task: MaintenanceTask) -> &'static str {
    match task {
        MaintenanceTask::Retention => "0 3 * * 0",
        MaintenanceTask::Check => "0 4 1 * *",
    }
}

pub const DEFAULT_TIME_ZONE: &str = "UTC";

/// The lifetime of a scheduled maintenance job, counted from its fire time (FR-SC-07).
pub const MAINTENANCE_JOB_LIFETIME: time::Duration = time::Duration::hours(24);

/// The largest number of due schedules that one tenant pass claims.
const SCHEDULES_PER_PASS: i64 = 200;

/// The most fire times that the walk of a missed run covers.
const MAX_WALKED_RUNS: usize = 100;

const SCHEDULE_SAVEPOINT: &str = "maintenance_schedule";

/// Creates the default schedules of a new repository (FR-RP-08).
pub async fn create_defaults(
    tx: &mut TenantTx<'_>,
    repository_id: restichub_db::RepositoryId,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    for task in MaintenanceTask::ALL {
        insert_default(tx, repository_id, task, now).await?;
    }
    Ok(())
}

async fn insert_default(
    tx: &mut TenantTx<'_>,
    repository_id: restichub_db::RepositoryId,
    task: MaintenanceTask,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    let cron = default_cron(task);
    // The defaults are valid, so the first fire time exists. A failure leaves the row unarmed, and
    // the next pass arms it.
    let next_run_at = Schedule::parse(cron, DEFAULT_TIME_ZONE)
        .and_then(|schedule| schedule.next_after(now))
        .ok();
    schedules::create(
        tx,
        &NewMaintenanceSchedule {
            repository_id,
            task,
            cron,
            time_zone: DEFAULT_TIME_ZONE,
            enabled: true,
            next_run_at,
        },
    )
    .await?;
    Ok(())
}

/// Runs the maintenance pass of one tenant (FR-RP-08).
pub async fn process_tenant(
    db: &Database,
    tenant: TenantId,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    let mut tx = db.begin_tenant(tenant).await?;
    for (repository_id, task) in schedules::missing_defaults(&mut tx, SCHEDULES_PER_PASS).await? {
        tx.savepoint(SCHEDULE_SAVEPOINT).await?;
        match insert_default(&mut tx, repository_id, task, now).await {
            Ok(()) => tx.release_savepoint(SCHEDULE_SAVEPOINT).await?,
            Err(error) => {
                tx.rollback_to_savepoint(SCHEDULE_SAVEPOINT).await?;
                tracing::warn!(repository = %repository_id, %error, "the default maintenance schedule failed");
            }
        }
    }
    for schedule in schedules::claim_unarmed(&mut tx, SCHEDULES_PER_PASS).await? {
        let armed = Schedule::parse(&schedule.cron, &schedule.time_zone)
            .and_then(|parsed| parsed.next_after(now));
        match armed {
            Ok(next_run_at) => {
                schedules::arm(&mut tx, schedule.id, next_run_at).await?;
            }
            Err(error) => {
                tracing::warn!(schedule = %schedule.id, %error, "the maintenance schedule cannot be armed");
            }
        }
    }
    let due = schedules::claim_due(&mut tx, now, SCHEDULES_PER_PASS).await?;
    for schedule in &due {
        tx.savepoint(SCHEDULE_SAVEPOINT).await?;
        match process_schedule(&mut tx, schedule, now).await {
            Ok(()) => tx.release_savepoint(SCHEDULE_SAVEPOINT).await?,
            Err(error) => {
                tx.rollback_to_savepoint(SCHEDULE_SAVEPOINT).await?;
                tracing::warn!(schedule = %schedule.id, %error, "the scheduler skipped a maintenance schedule");
            }
        }
    }
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum ScheduleFailure {
    #[error("{0}")]
    Db(#[from] DbError),
    #[error("{0}")]
    Schedule(#[from] ScheduleError),
}

async fn process_schedule(
    tx: &mut TenantTx<'_>,
    due: &DueMaintenance,
    now: OffsetDateTime,
) -> Result<(), ScheduleFailure> {
    let schedule = Schedule::parse(&due.cron, &due.time_zone)?;
    let next_run_at = schedule.next_after(now)?;

    // No maintenance agent, a revoked one, or an uninitialized repository: no job, and the schedule
    // advances so the next pass does not process it again (FR-RP-08).
    let agent = match (due.maintenance_agent_id, due.agent_status) {
        (Some(agent), Some(AgentStatus::Active)) if due.initialized => Some(agent),
        _ => None,
    };
    let Some(agent_id) = agent else {
        schedules::advance(tx, due.id, next_run_at, due.next_run_at).await?;
        return Ok(());
    };

    // The latest unexpired fire time only: a missed run does not repeat a cycle of the repository.
    let missed = schedule.fire_times_up_to(
        due.next_run_at,
        now,
        MAX_WALKED_RUNS,
        MAINTENANCE_JOB_LIFETIME,
    )?;
    let Some(fire_time) = missed.last().copied() else {
        schedules::advance(tx, due.id, next_run_at, due.next_run_at).await?;
        return Ok(());
    };

    if due.task == MaintenanceTask::Retention
        && !schedules::has_retention_in_use(tx, due.repository_id).await?
    {
        schedules::advance(tx, due.id, next_run_at, fire_time).await?;
        return Ok(());
    }
    // The repository holds one exclusive job at a time. The schedule stays due until it ends.
    if queue_repo::has_active_exclusive_for_repository(tx, due.repository_id).await? {
        return Ok(());
    }

    let job = NewJob {
        id: JobId::new(),
        kind: due.task.job_kind(),
        agent_id,
        repository_id: Some(due.repository_id),
        plan_id: None,
        fire_time: Some(fire_time),
        expires_at: Some(fire_time + MAINTENANCE_JOB_LIFETIME),
        location_fingerprint: None,
        created_by: None,
    };
    jobs::create_maintenance(tx, &job, due.id).await?;
    schedules::advance(tx, due.id, next_run_at, fire_time).await?;
    Ok(())
}
