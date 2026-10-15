//! The guards of the re-registration and of the replacement of an old host record (FR-ID-10,
//! FR-ID-11, FR-ID-04, ADR-0041).

#![allow(clippy::expect_used)]

mod common;
mod reenroll_support;

use axum::http::StatusCode;
use restichub_db::agents as repo;
use serde_json::json;
use time::{Duration, OffsetDateTime};

use common::{request, seed_job, seed_plan, seed_profile, seed_repository, seed_secret, send};
use reenroll_support::{
    MACHINE, agents, audit_count, enroll, enroll_new, env, find, issue_reenroll_token, serial_of,
};

async fn replace(
    env: &reenroll_support::Env,
    new: restichub_db::AgentId,
    old: restichub_db::AgentId,
) -> (StatusCode, serde_json::Value) {
    let (status, _, body) = send(
        &env.api,
        request(
            "POST",
            &format!("/api/v1/agents/{new}/replace"),
            Some(&env.cookie),
            Some(json!({"old_agent_id": old.to_string()})),
        ),
    )
    .await;
    (status, body)
}

/// Runs one statement with an optional uuid parameter as the owner under the scope of the tenant.
async fn exec(
    db: &restichub_db::test_util::TestDatabase,
    env: &reenroll_support::Env,
    sql: &'static str,
    id: uuid::Uuid,
) {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(env.tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    sqlx::query(sql)
        .bind(id)
        .execute(&mut *tx)
        .await
        .expect("cannot run the statement");
    tx.commit().await.expect("cannot commit");
}

async fn touch(
    db: &restichub_db::test_util::TestDatabase,
    env: &reenroll_support::Env,
    id: restichub_db::AgentId,
    at: OffsetDateTime,
) {
    let (_, hex) = serial_of(db, env, id).await;
    let mut tx = db
        .database()
        .begin_tenant(env.tenant)
        .await
        .expect("cannot begin");
    assert!(
        repo::touch_last_seen_guarded(&mut tx, id, &hex, at)
            .await
            .expect("cannot touch")
    );
    tx.commit().await.expect("cannot commit");
}

#[tokio::test]
async fn ac_id_09_the_replacement_runs_only_from_the_newer_record() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "direction").await;
    let old = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let new = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;

    let items = agents(&env).await;
    assert_eq!(find(&items, new)["possible_duplicate_of"]["is_older"], true);
    assert_eq!(
        find(&items, old)["possible_duplicate_of"]["is_older"],
        false
    );

    let (status, _) = replace(&env, old, new).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the older record cannot replace the newer one"
    );
    let items = agents(&env).await;
    assert_eq!(find(&items, new)["status"], "active");
    assert_eq!(find(&items, old)["status"], "active");
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_09_the_replacement_is_refused_while_the_old_record_is_online() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "online").await;
    let old = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let new = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;

    touch(&db, &env, old, OffsetDateTime::now_utc()).await;
    let (status, body) = replace(&env, new, old).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "agent_recently_online");
    assert_eq!(find(&agents(&env).await, old)["status"], "active");

    touch(
        &db,
        &env,
        old,
        OffsetDateTime::now_utc() - Duration::hours(1),
    )
    .await;
    let (status, body) = replace(&env, new, old).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_10_the_replacement_moves_queued_jobs_and_audits_the_revoke() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "queued").await;
    let old = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let new = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let secret = seed_secret(db.database(), env.tenant, "plan.password").await;
    let repository = seed_repository(db.database(), env.tenant, "repo", secret).await;
    let profile = seed_profile(db.database(), env.tenant, "profile", repository).await;
    seed_plan(db.database(), env.tenant, "plan", profile, old).await;
    let queued = seed_job(db.database(), env.tenant, old, repository).await;

    let (status, body) = replace(&env, new, old).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["queued_jobs_moved"], 1);

    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(env.tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    let owner: (uuid::Uuid,) = sqlx::query_as("SELECT agent_id FROM jobs WHERE id = $1")
        .bind(queued.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .expect("cannot read the job");
    tx.commit().await.expect("cannot commit");
    assert_eq!(owner.0, new.as_uuid(), "the queued job follows the plan");
    assert_eq!(audit_count(&env, "agent_revoke").await, 1);
    assert_eq!(audit_count(&env, "agent_replace").await, 1);
    db.cleanup().await;
}

/// The status, the result message and the stale lock mark of a job.
async fn job_row(
    db: &restichub_db::test_util::TestDatabase,
    env: &reenroll_support::Env,
    id: restichub_db::JobId,
) -> (String, Option<String>, bool) {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(env.tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    let row = sqlx::query_as(
        "SELECT status, result_message, stale_lock_since IS NOT NULL FROM jobs WHERE id = $1",
    )
    .bind(id.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("cannot read the job");
    tx.commit().await.expect("cannot commit");
    row
}

#[tokio::test]
async fn ac_id_10_the_replacement_fails_the_active_jobs_of_the_old_record() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "running").await;
    let old = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let new = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let secret = seed_secret(db.database(), env.tenant, "plan.password").await;
    let first = seed_repository(db.database(), env.tenant, "first", secret).await;
    let second = seed_repository(db.database(), env.tenant, "second", secret).await;
    let running = seed_job(db.database(), env.tenant, old, first).await;
    let sent = seed_job(db.database(), env.tenant, old, second).await;
    exec(
        &db,
        &env,
        "UPDATE jobs SET status = 'running' WHERE id = $1",
        running.as_uuid(),
    )
    .await;
    exec(
        &db,
        &env,
        "UPDATE jobs SET status = 'sent' WHERE id = $1",
        sent.as_uuid(),
    )
    .await;

    let (status, body) = replace(&env, new, old).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (state, message, marked) = job_row(&db, &env, running).await;
    assert_eq!(state, "failed");
    assert_eq!(message.as_deref(), Some("agent_revoked"));
    assert!(marked, "the lock of a running job is a suspect");
    let (state, message, marked) = job_row(&db, &env, sent).await;
    assert_eq!(state, "failed");
    assert_eq!(message.as_deref(), Some("agent_revoked"));
    assert!(!marked, "restic never started for a sent job");
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_07_a_second_bound_token_dies_with_the_first_use() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "second-token").await;
    let id = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;

    let (_, first) = issue_reenroll_token(&env, id).await;
    let (_, second) = issue_reenroll_token(&env, id).await;
    let first = first["token"].as_str().expect("the token").to_owned();
    let second = second["token"].as_str().expect("the token").to_owned();
    // Issuing the second token revoked the first one.
    let (status, _) = enroll(&env, &first, "MINDCORE", Some(MACHINE)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, third) = issue_reenroll_token(&env, id).await;
    let third = third["token"].as_str().expect("the token").to_owned();
    let (status, _) = enroll(&env, &third, "MINDCORE", Some(MACHINE)).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = enroll(&env, &second, "MINDCORE", Some(MACHINE)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "the older token is dead");
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_08_a_bound_token_lives_one_hour_at_most() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "bound-ttl").await;
    let id = enroll_new(&db, &env, "MINDCORE", None).await;
    let (status, _, _) = send(
        &env.api,
        request(
            "POST",
            &format!("/api/v1/agents/{id}/reenroll"),
            Some(&env.cookie),
            Some(json!({"ttl_seconds": 7200})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, body) = send(
        &env.api,
        request(
            "POST",
            &format!("/api/v1/agents/{id}/reenroll"),
            Some(&env.cookie),
            Some(json!({"ttl_seconds": 600})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_08_a_re_registered_host_raises_no_unreachable_alert() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "no-alert").await;
    let id = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    exec(
        &db,
        &env,
        "UPDATE agents SET created_at = now() - interval '90 days' WHERE id = $1",
        id.as_uuid(),
    )
    .await;

    let (_, issued) = issue_reenroll_token(&env, id).await;
    let token = issued["token"].as_str().expect("the token");
    let (status, _) = enroll(&env, token, "MINDCORE", Some(MACHINE)).await;
    assert_eq!(status, StatusCode::CREATED);

    let mut tx = db
        .database()
        .begin_tenant(env.tenant)
        .await
        .expect("cannot begin");
    let facts = restichub_db::dashboard::agent_facts(&mut tx)
        .await
        .expect("cannot read the facts");
    tx.commit().await.expect("cannot commit");
    let fact = facts.iter().find(|fact| fact.id == id).expect("the host");
    let silent = OffsetDateTime::now_utc() - fact.last_contact();
    assert!(
        silent < Duration::minutes(1),
        "the silence counts from the registration: {silent}"
    );
    assert!(fact.first_connected_at.is_none());
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_07_a_refused_bound_token_of_a_revoked_host_is_audited() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "refusal-audit").await;
    let id = enroll_new(&db, &env, "gone", None).await;
    let (_, issued) = issue_reenroll_token(&env, id).await;
    let token = issued["token"].as_str().expect("the token").to_owned();
    let (status, _, _) = send(
        &env.api,
        request(
            "POST",
            &format!("/api/v1/agents/{id}/revoke"),
            Some(&env.cookie),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = enroll(&env, &token, "gone", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(audit_count(&env, "enrollment_token_refuse").await, 1);
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_07_the_reported_hash_replaces_the_old_one() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "hash-reset").await;
    let id = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let other = enroll_new(&db, &env, "MINDCORE-2", Some(MACHINE)).await;
    assert!(!find(&agents(&env).await, id)["possible_duplicate_of"].is_null());

    let (_, issued) = issue_reenroll_token(&env, id).await;
    let token = issued["token"].as_str().expect("the token");
    let (status, _) = enroll(&env, token, "MINDCORE", None).await;
    assert_eq!(status, StatusCode::CREATED);
    let items = agents(&env).await;
    assert!(find(&items, id)["possible_duplicate_of"].is_null());
    assert!(find(&items, other)["possible_duplicate_of"].is_null());
    db.cleanup().await;
}
