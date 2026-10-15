//! The control plane audits the refusal of an agent and the message of an agent for a job that is
//! not its own (AC-PR-01, AC-PR-05).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use restichub_proto::agent::v1::{JobRejected, RejectReason, agent_message};
use serde_json::Value;
use uuid::Uuid;

use queue_support::{Env, decode_job_id, test_queue};

struct AuditRow {
    actor_type: String,
    actor_id: Option<Uuid>,
    object_id: Option<Uuid>,
    diff: Value,
}

async fn audit_rows(env: &Env, action: &str) -> Vec<AuditRow> {
    let mut tx = env.scoped().await;
    let rows = sqlx::query_as::<_, (String, Option<Uuid>, Option<Uuid>, Option<Value>)>(
        "SELECT actor_type, actor_id, object_id, diff FROM audit_log WHERE action = $1",
    )
    .bind(action)
    .fetch_all(&mut *tx)
    .await
    .expect("cannot read the audit log");
    tx.commit().await.expect("cannot end the read");
    rows.into_iter()
        .map(|(actor_type, actor_id, object_id, diff)| AuditRow {
            actor_type,
            actor_id,
            object_id,
            diff: diff.unwrap_or(Value::Null),
        })
        .collect()
}

async fn wait_for_rows(env: &Env, action: &str) -> Vec<AuditRow> {
    for _ in 0..100 {
        let rows = audit_rows(env, action).await;
        if !rows.is_empty() {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Vec::new()
}

#[tokio::test(flavor = "multi_thread")]
async fn ac_pr_01_the_control_plane_audits_the_refusal_of_a_job() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("reject-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let plan = env.run_fresh_plan(repository, agent.id).await;
    env.pass().await;
    let id = decode_job_id(&agent.job().await);
    assert_eq!(id, plan.to_string());

    agent
        .tx
        .send(restichub_proto::agent::v1::AgentMessage {
            payload: Some(agent_message::Payload::JobRejected(JobRejected {
                job_id: id.clone(),
                reason: RejectReason::BadSignature as i32,
            })),
        })
        .await
        .expect("cannot send the rejection");

    let rows = wait_for_rows(&env, "job_rejected").await;
    assert_eq!(rows.len(), 1, "one record for one refusal");
    assert_eq!(rows[0].actor_type, "agent");
    assert_eq!(rows[0].actor_id, Some(agent.id.as_uuid()));
    assert_eq!(rows[0].object_id, Some(plan));
    assert!(
        rows[0].diff.to_string().contains("bad_signature"),
        "{}",
        rows[0].diff
    );
    env.db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ac_pr_01_a_refusal_that_names_no_job_is_audited_without_an_object() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("forged-host").await;
    agent
        .tx
        .send(restichub_proto::agent::v1::AgentMessage {
            payload: Some(agent_message::Payload::JobRejected(JobRejected {
                job_id: String::new(),
                reason: RejectReason::BadSignature as i32,
            })),
        })
        .await
        .expect("cannot send the rejection");

    let rows = wait_for_rows(&env, "job_rejected").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].object_id, None);
    assert_eq!(rows[0].actor_id, Some(agent.id.as_uuid()));
    env.db.cleanup().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ac_pr_05_a_message_for_the_job_of_another_agent_is_audited() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let attacker = env.connect_agent("attacker-host").await;
    let mut victim = env.connect_agent("victim-host").await;
    let repository = env.create_repository(victim.id, "pw").await;
    let backup = env.run_fresh_plan(repository, victim.id).await;
    env.pass().await;
    let job = decode_job_id(&victim.job().await);

    attacker.ack(&job).await;
    attacker
        .result(
            &job,
            restichub_proto::agent::v1::JobStatus::Succeeded,
            restichub_proto::agent::v1::StepStatus::Success,
        )
        .await;
    attacker
        .tx
        .send(restichub_proto::agent::v1::AgentMessage {
            payload: Some(agent_message::Payload::JobRejected(JobRejected {
                job_id: job.clone(),
                reason: RejectReason::Replay as i32,
            })),
        })
        .await
        .expect("cannot send the rejection");

    for _ in 0..100 {
        if audit_rows(&env, "agent_message_refused").await.len() >= 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let rows = audit_rows(&env, "agent_message_refused").await;
    let mut kinds = Vec::new();
    for row in &rows {
        assert_eq!(row.actor_type, "agent");
        assert_eq!(row.actor_id, Some(attacker.id.as_uuid()));
        assert_eq!(row.object_id, Some(backup));
        assert_eq!(row.diff["new"]["reason"], "foreign_job", "{}", row.diff);
        kinds.push(
            row.diff["new"]["message"]
                .as_str()
                .expect("a kind")
                .to_owned(),
        );
    }
    kinds.sort();
    assert_eq!(
        kinds,
        ["job_ack", "job_rejected", "job_result", "job_result_data"]
    );
    assert_eq!(env.job_status(backup).await, "sent");
    assert!(audit_rows(&env, "job_rejected").await.is_empty());
    env.db.cleanup().await;
}
