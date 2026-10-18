//! The parameter values across the replace of a host record (FR-ID-11, FR-PF-11, FR-PF-12,
//! ADR-0047).
//!
//! The move gives the plans of the old record the successor, and the values follow. A name cannot
//! have two kinds on one host, and the audit names the values that moved.

#![allow(clippy::expect_used)]

mod common;
mod parameters_support;

use axum::http::StatusCode;
use restichub_db::test_util::TestDatabase;
use restichub_db::{AgentId, TenantId};
use serde_json::json;

use common::{
    app_with_kek, create_tenant_with_key, request, seed_agent, seed_plan, seed_profile,
    seed_repository, seed_secret, seed_session, send,
};
use parameters_support::audit_diff;

/// Two records of one host: the old one first, so the successor is the newer row (FR-ID-11).
async fn duplicate_pair(db: &TestDatabase, tenant: TenantId) -> (AgentId, AgentId) {
    let old = seed_agent(db.database(), tenant, "legacy").await;
    let new = seed_agent(db.database(), tenant, "successor").await;
    let mut tx = scope(db, tenant).await;
    sqlx::query(
        "UPDATE agents SET machine_id_hash = $2, created_at = now() - interval '1 minute' \
         WHERE id = $1",
    )
    .bind(old.as_uuid())
    .bind(vec![7_u8])
    .execute(&mut *tx)
    .await
    .expect("cannot mark the old record");
    sqlx::query("UPDATE agents SET machine_id_hash = $2 WHERE id = $1")
        .bind(new.as_uuid())
        .bind(vec![7_u8])
        .execute(&mut *tx)
        .await
        .expect("cannot mark the successor");
    tx.commit().await.expect("cannot commit the pair");
    (old, new)
}

/// Gives a seeded profile its declarations (FR-PF-09).
async fn declare(
    db: &TestDatabase,
    tenant: TenantId,
    profile: restichub_db::ProfileId,
    body: serde_json::Value,
) {
    let mut tx = scope(db, tenant).await;
    sqlx::query("UPDATE profiles SET parameters = $2::jsonb WHERE id = $1")
        .bind(profile.as_uuid())
        .bind(body.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot change the declarations");
    tx.commit().await.expect("cannot commit the declarations");
}

/// Puts the values of a host into the table (FR-PF-10).
async fn store_values(
    db: &TestDatabase,
    tenant: TenantId,
    agent: AgentId,
    text: Option<(&str, &str)>,
    secret: Option<(&str, restichub_db::SecretId)>,
) {
    let mut tx = scope(db, tenant).await;
    if let Some((name, value)) = text {
        sqlx::query(
            "INSERT INTO agent_parameter_values (tenant_id, agent_id, name, kind, value) \
             VALUES ($1, $2, $3, 'text', $4)",
        )
        .bind(tenant.as_uuid())
        .bind(agent.as_uuid())
        .bind(name)
        .bind(value)
        .execute(&mut *tx)
        .await
        .expect("cannot store the text value");
    }
    if let Some((name, id)) = secret {
        sqlx::query(
            "INSERT INTO agent_parameter_values (tenant_id, agent_id, name, kind, secret_id) \
             VALUES ($1, $2, $3, 'secret', $4)",
        )
        .bind(tenant.as_uuid())
        .bind(agent.as_uuid())
        .bind(name)
        .bind(id.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("cannot store the secret value");
    }
    tx.commit().await.expect("cannot commit the values");
}

async fn scope(db: &TestDatabase, tenant: TenantId) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    tx
}

/// FR-PF-11: a replace that would put two kinds of one name on the host is refused, and nothing
/// moves.
#[tokio::test]
async fn fr_pf_11_a_replace_refuses_two_kinds_of_one_name() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "replace").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let password = seed_secret(db.database(), tenant, "plan.password").await;
    let repository = seed_repository(db.database(), tenant, "repo", password).await;
    let (old, new) = duplicate_pair(&db, tenant).await;
    let text = seed_profile(db.database(), tenant, "text-kind").await;
    let secret = seed_profile(db.database(), tenant, "secret-kind").await;
    declare(
        &db,
        tenant,
        text,
        json!([{"name": "pg_dir", "kind": "text"}]),
    )
    .await;
    declare(
        &db,
        tenant,
        secret,
        json!([{"name": "pg_dir", "kind": "secret"}]),
    )
    .await;
    seed_plan(db.database(), tenant, "legacy", text, old, repository).await;
    seed_plan(db.database(), tenant, "successor", secret, new, repository).await;

    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/agents/{new}/replace"),
            Some(&cookie),
            Some(json!({"old_agent_id": old.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "old_agent_id");
    assert_eq!(problem["reason"], "parameter_kind_mismatch");
    assert_eq!(problem["parameters"], json!(["pg_dir"]));

    // The transaction rolled back, so the old record is still the active one.
    let (status, _, agents) =
        send(&app, request("GET", "/api/v1/agents", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{agents}");
    let status_of = |id: AgentId| {
        agents["items"]
            .as_array()
            .expect("a list")
            .iter()
            .find(|agent| agent["id"] == id.to_string())
            .map(|agent| agent["status"].clone())
            .unwrap_or_else(|| panic!("the agent {id} is absent: {agents}"))
    };
    assert_eq!(status_of(old), "active", "{agents}");
    assert_eq!(status_of(new), "active", "{agents}");

    db.cleanup().await;
}

/// FR-PF-12: the audit of a replace names the values that moved, and never their text.
#[tokio::test]
async fn fr_pf_12_a_replace_names_the_values_that_moved() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "replace").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let password = seed_secret(db.database(), tenant, "plan.password").await;
    let repository = seed_repository(db.database(), tenant, "repo", password).await;
    let (old, new) = duplicate_pair(&db, tenant).await;
    let profile = seed_profile(db.database(), tenant, "pg").await;
    declare(
        &db,
        tenant,
        profile,
        json!([
            {"name": "pg_dir", "kind": "text"},
            {"name": "pg_password", "kind": "secret"},
        ]),
    )
    .await;
    seed_plan(db.database(), tenant, "nightly", profile, old, repository).await;
    let hook = seed_secret(db.database(), tenant, "pg.password").await;
    let canary = "CANARY-moved-1";
    store_values(
        &db,
        tenant,
        old,
        Some(("pg_dir", canary)),
        Some(("pg_password", hook)),
    )
    .await;

    let (status, _, moved) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/agents/{new}/replace"),
            Some(&cookie),
            Some(json!({"old_agent_id": old.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["plans_moved"], 1, "{moved}");

    let diff = audit_diff(&app, &cookie, "agent_replace", &new.to_string()).await;
    let values = diff["new"]["parameter_values"]
        .as_array()
        .expect("the moved values");
    assert_eq!(values.len(), 2, "{diff}");
    assert_eq!(values[0]["name"], "pg_dir", "{diff}");
    assert_eq!(values[0]["kind"], "text", "{diff}");
    assert_eq!(values[0]["value"], "<set>", "{diff}");
    assert_eq!(values[1]["name"], "pg_password", "{diff}");
    assert_eq!(values[1]["kind"], "secret", "{diff}");
    assert_eq!(values[1]["reference"], hook.to_string(), "{diff}");
    assert!(
        !diff.to_string().contains(canary),
        "the audit holds no value"
    );

    db.cleanup().await;
}
