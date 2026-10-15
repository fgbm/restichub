//! The emulator against the real control plane, end to end (FR-ID-01, FR-ID-03, FR-ID-07, FR-PR-09,
//! FR-SC-01, FR-RS-03).
//!
//! N emulated agents enroll over HTTPS, open their mTLS streams and announce their capabilities
//! and restore roots. The scheduler fires a backup plan for each agent, the dispatcher delivers the
//! signed job, and a restore request runs on the same agents. A second test lets an emulated agent
//! rotate its certificate.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;
mod rotation_support;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::http::StatusCode;
use restichub_emulator::agent::ProgressSink;
use restichub_emulator::executor::{Executor, StepOutcome};
use restichub_proto::agent::v1::Step;
use serde_json::json;

use queue_support::{Emulator, Env, default_restore_roots, eventually, id_of, test_queue};

/// The number of emulated agents of the first test.
const AGENTS: usize = 6;

/// Finishes each step after a short pause, as a fast host does.
struct Quick;

#[async_trait]
impl Executor for Quick {
    async fn run_step(&self, _step: &Step, _progress: &ProgressSink) -> StepOutcome {
        tokio::time::sleep(Duration::from_millis(20)).await;
        StepOutcome::success()
    }
}

/// One emulated agent with a repository and a scheduled plan.
struct Host {
    emulator: Emulator,
    repository: uuid::Uuid,
    plan: uuid::Uuid,
}

/// A plan that runs every night at 02:00 UTC and runs the latest missed fire time (FR-SC-01).
async fn nightly_plan(env: &Env, profile: uuid::Uuid, agent: restichub_db::AgentId) -> uuid::Uuid {
    let body = json!({
        "name": format!("nightly-{agent}"),
        "profile_id": profile.to_string(),
        "agent_id": agent.to_string(),
        "cron": "0 2 * * *",
        "time_zone": "UTC",
        "missed_run_policy": "latest_only",
    });
    let (status, _, response) = common::send(
        &env.app,
        common::bearer_request("POST", "/api/v1/plans", &env.admin, Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    id_of(&response["plan"])
}

async fn host(env: &Env, index: usize) -> Host {
    let emulator = env
        .start_emulator(&format!("e2e-host-{index}"), Arc::new(Quick))
        .await;
    let repository = env.create_repository(emulator.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = nightly_plan(env, profile, emulator.id).await;
    Host {
        emulator,
        repository,
        plan,
    }
}

/// FR-ID-07, FR-PR-09, FR-SC-01, FR-RS-03: N emulated agents enroll, stream, run the scheduled
/// backup that the scheduler creates and run a restore request.
#[tokio::test(flavor = "multi_thread")]
async fn ac_id_07_n_emulated_agents_enroll_stream_back_up_on_schedule_and_restore() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut hosts = Vec::new();
    for index in 0..AGENTS {
        hosts.push(host(&env, index).await);
    }

    // Enrollment and stream: every agent is online and the registry holds its hello facts.
    assert_eq!(env.state.agent_registry().online_agents().len(), AGENTS);
    let (status, _, list) = common::send(
        &env.app,
        common::bearer_request("GET", "/api/v1/agents", &env.admin, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let items = list["items"].as_array().expect("items");
    assert_eq!(items.len(), AGENTS);
    for item in items {
        assert_eq!(
            item["protocol_version"],
            format!(
                "{}.{}",
                restichub_proto::PROTOCOL_VERSION_MAJOR,
                restichub_proto::PROTOCOL_VERSION_MINOR
            )
        );
        assert_eq!(item["agent_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(item["version_status"], "current");
        assert_eq!(item["restore_roots"], json!(default_restore_roots()));
    }
    for host in &hosts {
        let capabilities = env.stored_capabilities(host.emulator.id).await;
        assert!(
            !capabilities.is_empty(),
            "the emulator reports its capabilities"
        );
    }

    // The scheduled backup: the fire time of every plan has just passed. The emulated agents
    // verify the job against their own clock, so the test moves the fire time and not the clock.
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE plans SET next_run_at = $1")
        .bind(env.now() - time::Duration::seconds(30))
        .execute(&mut *tx)
        .await
        .expect("cannot place the fire time");
    tx.commit().await.expect("cannot commit");
    restichub_cp::scheduler::run_once(env.db.database(), env.clock.as_ref())
        .await
        .expect("the scheduler pass");
    for host in &hosts {
        let plan = host.plan;
        eventually(|| async {
            env.pass().await;
            let jobs = env.plan_jobs(plan).await;
            !jobs.is_empty() && jobs.iter().all(|job| job["status"] == "succeeded")
        })
        .await;
    }

    // The restore: each agent restores into its own folder under the restore root. A restore
    // needs an initialized repository, so the test marks it as the init job would.
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE repositories SET initialized = true")
        .execute(&mut *tx)
        .await
        .expect("cannot mark the repositories initialized");
    tx.commit().await.expect("cannot commit");
    let mut restores = Vec::new();
    for (index, host) in hosts.iter().enumerate() {
        let target = std::env::temp_dir().join(format!("restichub-e2e-restore-{index}"));
        let (status, _, job) = common::send(
            &env.app,
            common::bearer_request(
                "POST",
                &format!("/api/v1/repositories/{}/restore", host.repository),
                &env.operator,
                Some(json!({
                    "snapshot_id": "0123abcd",
                    "target": target.to_str().expect("a UTF-8 path"),
                    "overwrite": "always",
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{job}");
        restores.push(id_of(&job));
    }
    for restore in restores {
        eventually(|| async {
            env.pass().await;
            env.job_status(restore).await == "succeeded"
        })
        .await;
    }

    for host in hosts {
        host.emulator.stop().await;
    }
    env.db.cleanup().await;
}

/// FR-ID-03: an emulated agent asks for a new certificate at two thirds of the lifetime, the
/// control plane signs it, and the agent stays online.
#[tokio::test(flavor = "multi_thread")]
async fn ac_id_03_an_emulated_agent_rotates_its_certificate_over_the_stream() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start_with_lifetime(db, test_queue(), Duration::from_secs(6)).await;
    // The test waits in real time, so the clock of the control plane follows the wall clock.
    let clock = Arc::clone(&env.clock);
    let follower = tokio::spawn(async move {
        loop {
            clock.set(time::OffsetDateTime::now_utc());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let emulator = env.start_emulator("e2e-rotation", Arc::new(Quick)).await;
    let before = rotation_support::row(&env, emulator.id).await;

    eventually(|| async { emulator.config.load.renewal.completed() >= 1 }).await;
    eventually(|| async { rotation_support::row(&env, emulator.id).await.serial != before.serial })
        .await;
    let after = rotation_support::row(&env, emulator.id).await;
    assert!(after.last_rotation.is_some());
    assert_eq!(after.pending_serial, None);
    assert!(env.state.agent_registry().is_online(emulator.id));

    follower.abort();
    emulator.stop().await;
    env.db.cleanup().await;
}
