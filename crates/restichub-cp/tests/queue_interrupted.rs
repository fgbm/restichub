//! A backup that a stop of the service, a restart of the agent or a reboot interrupts ends as
//! cancelled and raises no job alert; every other interrupted job, a real failure and a lost job
//! stay failed and raise the alert (FR-MN-03, AC-MN-10, AC-SC-10).

#![allow(clippy::expect_used)]

mod alert_support;
mod common;
mod queue_support;

use alert_support::{judged_view, local_sender, plan_created_at};
use restichub_cp::clock::ManualClock;
use restichub_db::{JobKind, PlanId};
use restichub_proto::agent::v1::{JobStatus, StepStatus};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use queue_support::{Env, FakeAgent, decode_job_id, eventually, test_queue};

const INTERRUPTED: &str = "interrupted";

struct World {
    cp: Env,
    agent: FakeAgent,
    plan: Uuid,
    repository: Uuid,
}

async fn world(db: restichub_db::test_util::TestDatabase) -> World {
    let cp = Env::start(db, test_queue()).await;
    let agent = cp.connect_agent("interrupted-host").await;
    let repository = cp.create_repository(agent.id, "pw").await;
    let profile = cp.create_profile(json!({})).await;
    let plan = cp.create_plan(profile, agent.id, repository).await;
    World {
        cp,
        agent,
        plan,
        repository,
    }
}

/// Starts one backup on the agent and returns the job and its identifier on the wire.
async fn running_job(w: &mut World) -> (Uuid, String) {
    let job = w.cp.run_plan(w.plan).await;
    w.cp.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    w.agent.start(&w.cp, &id).await;
    (job, id)
}

async fn ended(cp: &Env, job: Uuid) -> Value {
    eventually(|| async { cp.job_status(job).await != "running" }).await;
    cp.job(job).await
}

/// Runs one alert pass at `now` and returns the kinds of the open alerts.
async fn open_alerts(cp: &Env, now: OffsetDateTime) -> Vec<String> {
    restichub_cp::alerts::run_once(
        cp.db.database(),
        &local_sender(None),
        common::test_kek(),
        ManualClock::new(now).shared(),
        judged_view(&cp.db, now).await,
    )
    .await
    .expect("the alert pass works");
    let mut tx = cp.scoped().await;
    let kinds = sqlx::query_scalar::<_, String>(
        "SELECT kind FROM alert_state WHERE status = 'open' ORDER BY kind",
    )
    .fetch_all(&mut *tx)
    .await
    .expect("cannot read the alerts");
    tx.commit().await.expect("cannot end the read");
    kinds
}

/// Starts a job of a kind that no plan runs and returns it with its identifier on the wire.
async fn running_maintenance(w: &mut World, kind: JobKind) -> (Uuid, String) {
    let job = w.cp.enqueue(kind, w.agent.id, w.repository, None).await;
    w.cp.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    w.agent.start(&w.cp, &id).await;
    (job, id)
}

async fn stale_marked(cp: &Env, job: Uuid) -> bool {
    let mut tx = cp.scoped().await;
    let marked = sqlx::query_scalar::<_, bool>(
        "SELECT stale_lock_since IS NOT NULL FROM jobs WHERE id = $1",
    )
    .bind(job)
    .fetch_one(&mut *tx)
    .await
    .expect("cannot read the mark");
    tx.commit().await.expect("cannot end the read");
    marked
}

/// AC-MN-10: the agent stops the running job on a stop of its service, and the cancelled result
/// reaches the control plane after the agent connects again.
#[tokio::test]
async fn ac_mn_10_a_service_stop_ends_the_job_as_cancelled() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    let (job, id) = running_job(&mut w).await;
    w.agent
        .result_with_error(
            &id,
            JobStatus::Cancelled,
            StepStatus::Cancelled,
            Some("cancelled"),
        )
        .await;
    let body = ended(&w.cp, job).await;
    assert_eq!(body["result_code"], "cancelled", "{body}");
    assert_eq!(body["status"], "failed");
    w.cp.db.cleanup().await;
}

/// AC-MN-10: an interrupted backup ends as cancelled and keeps the reason. It raises no job alert,
/// and the RPO alert of the plan fires when no backup succeeds in time.
#[tokio::test]
async fn ac_mn_10_an_interrupted_backup_is_cancelled_and_only_the_rpo_alert_fires() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    let (job, id) = running_job(&mut w).await;
    w.agent
        .result_with_error(
            &id,
            JobStatus::Failed,
            StepStatus::Failed,
            Some(INTERRUPTED),
        )
        .await;
    let body = ended(&w.cp, job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_code"], "cancelled", "{body}");
    assert_eq!(body["result_message"], INTERRUPTED);

    let soon = w.cp.now() + Duration::minutes(1);
    assert!(
        open_alerts(&w.cp, soon).await.is_empty(),
        "the interrupted backup raised an alert"
    );

    let created = plan_created_at(&w.cp.db, w.cp.tenant, PlanId::from_uuid(w.plan)).await;
    let later = created + Duration::days(400);
    assert_eq!(open_alerts(&w.cp, later).await, ["rpo_violated"]);
    w.cp.db.cleanup().await;
}

/// AC-MN-10: no RPO alert covers the maintenance, so an interrupted prune stays failed and raises
/// the job alert.
#[tokio::test]
async fn ac_mn_10_an_interrupted_prune_stays_failed_and_raises_the_alert() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    let (job, id) = running_maintenance(&mut w, JobKind::Prune).await;
    w.agent
        .result_with_error(
            &id,
            JobStatus::Failed,
            StepStatus::Failed,
            Some(INTERRUPTED),
        )
        .await;
    let body = ended(&w.cp, job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_code"], "failed", "{body}");
    assert_eq!(body["result_message"], INTERRUPTED);

    let soon = w.cp.now() + Duration::minutes(1);
    assert_eq!(open_alerts(&w.cp, soon).await, ["job_failed"]);
    w.cp.db.cleanup().await;
}

/// AC-MN-10: a restic error and a panic of the agent are failures, not interruptions.
#[tokio::test]
async fn ac_mn_10_a_real_failure_stays_failed() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    for code in ["step_failed", "agent_internal_error"] {
        let (job, id) = running_job(&mut w).await;
        w.agent
            .result_with_error(&id, JobStatus::Failed, StepStatus::Failed, Some(code))
            .await;
        let body = ended(&w.cp, job).await;
        assert_eq!(body["result_code"], "failed", "{code}: {body}");
        assert_eq!(body["result_message"], code);
    }
    w.cp.db.cleanup().await;
}

/// AC-SC-10: an agent that restarts and does not list the job has lost it. The job fails with the
/// alert, and its lock stays a suspect for the automatic unlock.
#[tokio::test]
async fn ac_sc_10_an_unlisted_job_fails_with_the_alert_and_a_stale_mark() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    let (job, _id) = running_job(&mut w).await;
    let _restarted = w.cp.reconnect_agent_holding(&w.agent, Vec::new()).await;
    let body = ended(&w.cp, job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_code"], "failed", "{body}");
    assert_eq!(body["result_message"], "agent_lost");
    assert!(stale_marked(&w.cp, job).await, "the lock may be stale");

    let soon = w.cp.now() + Duration::minutes(1);
    assert_eq!(open_alerts(&w.cp, soon).await, ["job_failed"]);
    w.cp.db.cleanup().await;
}

/// AC-PR-07: a job whose agent stays silent for 30 minutes fails with the alert.
#[tokio::test]
async fn ac_pr_07_a_silent_agent_fails_the_job_with_the_alert() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    let (job, _id) = running_job(&mut w).await;
    let agent_id = w.agent.id;
    drop(w.agent);
    eventually(|| async { !w.cp.state.agent_registry().is_online(agent_id) }).await;
    w.cp.clock.advance(31 * 60);
    w.cp.pass().await;
    let body = w.cp.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_code"], "failed", "{body}");
    assert_eq!(body["result_message"], "agent_lost");

    let soon = w.cp.now() + Duration::minutes(1);
    assert!(
        open_alerts(&w.cp, soon)
            .await
            .contains(&"job_failed".to_owned())
    );
    w.cp.db.cleanup().await;
}
