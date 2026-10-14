//! The job list, the cancel action, the manual run and the `unlock_all` action (FR-SC-06,
//! FR-SC-09, AC-SC-06, AC-TN-01).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use axum::http::StatusCode;
use restichub_proto::agent::v1::step;
use serde_json::json;
use uuid::Uuid;

use queue_support::{Env, decode_job_id, id_of, test_queue};

struct World {
    env: Env,
    agent: queue_support::FakeAgent,
    repository: Uuid,
    plan: Uuid,
}

async fn world(db: restichub_db::test_util::TestDatabase) -> World {
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("api-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, agent.id).await;
    World {
        env,
        agent,
        repository,
        plan,
    }
}

#[tokio::test]
async fn fr_sc_09_the_role_matrix_of_the_run_and_cancel_actions() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let w = world(db).await;
    let env = &w.env;
    let run = format!("/api/v1/plans/{}/run", w.plan);

    let (status, _) = env.call(&env.viewer, "POST", &run, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a Viewer cannot run a plan");
    let (status, body) = env.call(&env.operator, "POST", &run, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["status"], "queued");
    assert_eq!(body["kind"], "backup");
    let job = id_of(&body);
    let second_profile = env.create_profile(w.repository, json!({})).await;
    let second_plan = env.create_plan(second_profile, w.agent.id).await;
    let (status, _) = env
        .call(
            &env.admin,
            "POST",
            &format!("/api/v1/plans/{second_plan}/run"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "an Admin can run a plan");

    let cancel = format!("/api/v1/jobs/{job}/cancel");
    let (status, _) = env.call(&env.viewer, "POST", &cancel, None).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a Viewer cannot cancel a job"
    );
    let (status, body) = env.call(&env.operator, "POST", &cancel, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_code"], "cancelled");
    let (status, _) = env.call(&env.operator, "POST", &cancel, None).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a finished job cannot be cancelled"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_09_a_manual_run_for_an_online_agent_reaches_the_agent() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    let job = w.env.run_plan(w.plan).await;
    w.env.pass().await;
    assert_eq!(decode_job_id(&w.agent.job().await), job.to_string());
    w.env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_09_the_cancel_of_a_running_job_sends_the_cancel_message() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    w.agent.start(env, &id).await;

    let (status, body) = env
        .call(
            &env.operator,
            "POST",
            &format!("/api/v1/jobs/{job}/cancel"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(
        w.agent.try_cancel(Duration::from_secs(5)).await,
        Some(job.to_string())
    );

    // The agent reports the cancelled result and the job ends.
    w.agent
        .result(
            &id,
            restichub_proto::agent::v1::JobStatus::Cancelled,
            restichub_proto::agent::v1::StepStatus::Cancelled,
        )
        .await;
    queue_support::eventually(|| async { env.job_status(job).await == "failed" }).await;
    assert_eq!(env.job(job).await["result_code"], "cancelled");

    let audit: Vec<String> = {
        let mut tx = env.scoped().await;
        let rows = sqlx::query_scalar("SELECT action FROM audit_log WHERE action = 'job_cancel'")
            .fetch_all(&mut *tx)
            .await
            .expect("cannot read the audit log");
        tx.commit().await.expect("cannot end the read");
        rows
    };
    assert_eq!(audit.len(), 1, "the cancel writes one audit record");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_09_a_cancelled_sent_job_is_not_delivered_again() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    env.pass().await;
    w.agent.job().await;

    let (status, body) = env
        .call(
            &env.operator,
            "POST",
            &format!("/api/v1/jobs/{job}/cancel"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(
        w.agent.try_cancel(Duration::from_secs(5)).await,
        Some(job.to_string())
    );
    env.clock.advance(120);
    env.pass().await;
    assert!(
        w.agent.try_job(Duration::from_millis(300)).await.is_none(),
        "the lease ended with the cancel"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_09_the_cancel_of_a_job_on_an_offline_agent_gives_agent_offline() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    let job = w.env.run_plan(w.plan).await;
    w.env.pass().await;
    let id = decode_job_id(&w.agent.job().await);
    w.agent.start(&w.env, &id).await;
    let agent_id = w.agent.id;
    drop(w.agent);
    let env = &w.env;
    queue_support::eventually(|| async { !env.state.agent_registry().is_online(agent_id) }).await;

    let (status, body) = env
        .call(
            &env.operator,
            "POST",
            &format!("/api/v1/jobs/{job}/cancel"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "agent_offline");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_09_the_list_filters_and_paginates() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let w = world(db).await;
    let env = &w.env;
    env.run_plan(w.plan).await;
    for _ in 0..5 {
        let profile = env.create_profile(w.repository, json!({})).await;
        let plan = env.create_plan(profile, w.agent.id).await;
        env.run_plan(plan).await;
    }

    let (status, body) = env
        .call(
            &env.viewer,
            "GET",
            &format!("/api/v1/jobs?plan_id={}", w.plan),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().expect("items").len(), 1);
    assert!(body["items"][0]["plan_name"].is_string());
    assert_eq!(body["items"][0]["agent_hostname"], "api-host");
    assert!(body["items"][0]["repository_name"].is_string());

    let (_, body) = env
        .call(
            &env.viewer,
            "GET",
            &format!(
                "/api/v1/jobs?repository_id={}&agent_id={}&status=queued",
                w.repository, w.agent.id
            ),
            None,
        )
        .await;
    assert_eq!(body["items"].as_array().expect("items").len(), 6);
    let (_, body) = env
        .call(&env.viewer, "GET", "/api/v1/jobs?status=succeeded", None)
        .await;
    assert!(body["items"].as_array().expect("items").is_empty());

    // Keyset pagination: three pages of two jobs, newest first, no repeat.
    let mut seen = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..3 {
        let uri = match &cursor {
            Some(cursor) => format!("/api/v1/jobs?limit=2&cursor={cursor}"),
            None => "/api/v1/jobs?limit=2".to_owned(),
        };
        let (status, body) = env.call(&env.viewer, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for item in body["items"].as_array().expect("items") {
            seen.push(item["id"].as_str().expect("id").to_owned());
        }
        cursor = body["next_cursor"].as_str().map(str::to_owned);
    }
    assert_eq!(seen.len(), 6);
    let unique: std::collections::HashSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), 6, "no job repeats across the pages");
    assert!(cursor.is_none(), "the last page holds no cursor");

    let (status, _) = env
        .call(&env.viewer, "GET", "/api/v1/jobs?plan_id=nope", None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_01_another_tenant_gets_404_on_the_queue_endpoints() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let w = world(db).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;

    let other = common::create_tenant_with_key(env.db.database(), "other-queue-tenant").await;
    let token = common::seed_api_token(env.db.database(), other, "admin").await;

    let (status, _) = env
        .call(
            &token,
            "POST",
            &format!("/api/v1/plans/{}/run", w.plan),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = env
        .call(&token, "POST", &format!("/api/v1/jobs/{job}/cancel"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = env
        .call(&token, "GET", &format!("/api/v1/jobs/{job}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = env
        .call(
            &token,
            "POST",
            &format!("/api/v1/repositories/{}/unlock-all", w.repository),
            Some(json!({"confirm": "x"})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = env.call(&token, "GET", "/api/v1/jobs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["items"].as_array().expect("items").is_empty());
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_06_unlock_all_needs_an_admin_and_the_repository_name() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut w = world(db).await;
    let env = &w.env;
    let (_, repository) = env
        .call(
            &env.admin,
            "GET",
            &format!("/api/v1/repositories/{}", w.repository),
            None,
        )
        .await;
    let name = repository["name"].as_str().expect("name").to_owned();
    let uri = format!("/api/v1/repositories/{}/unlock-all", w.repository);

    let (status, _) = env
        .call(&env.operator, "POST", &uri, Some(json!({"confirm": name})))
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "only an Admin can run unlock_all"
    );
    let (status, body) = env
        .call(&env.admin, "POST", &uri, Some(json!({"confirm": "wrong"})))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "confirm");

    // A waiting job on the repository refuses the action.
    let waiting = env.run_plan(w.plan).await;
    let (status, _) = env
        .call(&env.admin, "POST", &uri, Some(json!({"confirm": name})))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    env.call(
        &env.operator,
        "POST",
        &format!("/api/v1/jobs/{waiting}/cancel"),
        None,
    )
    .await;

    let (status, body) = env
        .call(&env.admin, "POST", &uri, Some(json!({"confirm": name})))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["kind"], "unlock_all");
    env.pass().await;
    let signed = w.agent.job().await;
    let job = env.verify(&signed, &w.agent).await;
    assert!(matches!(job.steps[0].kind, Some(step::Kind::UnlockAll(_))));

    let mut tx = env.scoped().await;
    let events: Vec<String> =
        sqlx::query_scalar("SELECT action FROM audit_log WHERE action = 'repository_unlock_all'")
            .fetch_all(&mut *tx)
            .await
            .expect("cannot read the audit log");
    tx.commit().await.expect("cannot end the read");
    assert_eq!(events.len(), 1, "the audit log holds the event");
    env.db.cleanup().await;
}

#[tokio::test]
async fn a_revoked_agent_fails_its_queued_jobs() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let w = world(db).await;
    let env = &w.env;
    let job = env.run_plan(w.plan).await;
    let (status, body) = env
        .call(
            &env.admin,
            "POST",
            &format!("/api/v1/agents/{}/revoke", w.agent.id),
            None,
        )
        .await;
    assert!(status.is_success(), "{body}");
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_message"], "agent_revoked");
    env.db.cleanup().await;
}
