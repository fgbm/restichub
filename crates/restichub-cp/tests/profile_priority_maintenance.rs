//! The priority of a profile in the maintenance job of a repository (FR-AG-15, AC-AG-14): the
//! `forget` step carries it, the `prune` and the `check` step of the repository do not.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use axum::http::StatusCode;
use restichub_db::AgentId;
use restichub_proto::agent::v1::{Job, ProcessPriority, step};
use serde_json::json;
use uuid::Uuid;

use queue_support::{Env, test_queue};

async fn initialized_local_repository(env: &Env, agent: AgentId) -> Uuid {
    let repository = env.create_repository(agent, "pw").await;
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE repositories SET initialized = true WHERE id = $1")
        .bind(repository)
        .execute(&mut *tx)
        .await
        .expect("cannot mark the repository initialized");
    tx.commit().await.expect("cannot commit the mark");
    repository
}

async fn maintenance_job(env: &Env, task: &str) -> Job {
    let mut maintenance = env.connect_agent("maintenance-host").await;
    let repository = initialized_local_repository(env, maintenance.id).await;
    let profile = env
        .create_profile(json!({
            "options": {"priority": "low"},
            "retention": {"keep_last": 2},
            "check": {"read_data_subset": "1/2"},
        }))
        .await;
    env.create_plan(profile, maintenance.id, repository).await;

    let (status, queued) = env
        .call(
            &env.operator,
            "POST",
            &format!("/api/v1/repositories/{repository}/maintenance/run"),
            Some(json!({"task": task})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    env.pass().await;
    let signed = maintenance.job().await;
    env.verify(&signed, &maintenance).await
}

fn priorities(job: &Job) -> Vec<(String, i32)> {
    job.steps
        .iter()
        .filter_map(|step| {
            let priority = match step.kind.as_ref()? {
                step::Kind::Forget(forget) => forget.priority,
                step::Kind::Prune(prune) => prune.priority,
                step::Kind::Check(check) => check.priority,
                _ => return None,
            };
            Some((step.step_id.clone(), priority))
        })
        .collect()
}

#[tokio::test]
async fn ac_ag_14_the_retention_job_gives_the_low_priority_to_forget_only() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let job = maintenance_job(&env, "retention").await;
    assert_eq!(
        priorities(&job),
        vec![
            ("forget-1".to_owned(), ProcessPriority::Low as i32),
            ("prune".to_owned(), ProcessPriority::Unspecified as i32),
        ]
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_ag_14_the_check_job_keeps_the_normal_priority() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let job = maintenance_job(&env, "check").await;
    assert_eq!(
        priorities(&job),
        vec![("check".to_owned(), ProcessPriority::Unspecified as i32)]
    );
    env.db.cleanup().await;
}
