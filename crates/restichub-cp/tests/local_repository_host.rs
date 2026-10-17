//! A local repository is bound to its host: the path exists on the disk of that host only
//! (FR-RP-01, FR-RP-08, FR-RS-01, ADR-0035).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use axum::http::StatusCode;
use restichub_db::{AgentId, JobKind};
use serde_json::{Value, json};
use uuid::Uuid;

use queue_support::{Env, eventually, test_queue};

fn local_body(path: &str, host: Option<AgentId>) -> Value {
    let mut body = json!({
        "name": format!("local-{}", Uuid::now_v7()),
        "backend": {"kind": "local", "path": path},
        "password": "pw",
    });
    if let Some(host) = host {
        body["maintenance_agent_id"] = json!(host.to_string());
    }
    body
}

fn assert_field(body: &Value, field: &str, reason: &str) {
    assert_eq!(body["code"], "invalid_argument", "{body}");
    assert_eq!(body["field"], field, "{body}");
    assert_eq!(body["reason"], reason, "{body}");
}

async fn set_host(env: &Env, repository: Uuid, host: &str) -> (StatusCode, Value) {
    env.call(
        &env.admin,
        "PATCH",
        &format!("/api/v1/repositories/{repository}"),
        Some(json!({"maintenance_agent_id": host})),
    )
    .await
}

async fn mark_initialized(env: &Env, repository: Uuid) {
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE repositories SET initialized = true WHERE id = $1")
        .bind(repository)
        .execute(&mut *tx)
        .await
        .expect("cannot mark the repository initialized");
    tx.commit().await.expect("cannot commit the mark");
}

/// Moves a stored plan to another agent without the API, like a row saved before the host rule.
async fn move_plan(env: &Env, plan: Uuid, agent: AgentId) {
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE plans SET agent_id = $2 WHERE id = $1")
        .bind(plan)
        .bind(agent)
        .execute(&mut *tx)
        .await
        .expect("cannot move the plan");
    tx.commit().await.expect("cannot commit the move");
}

fn plan_body(profile: Uuid, agent: AgentId) -> Value {
    json!({
        "name": format!("plan-{}", Uuid::now_v7()),
        "profile_id": profile.to_string(),
        "agent_id": agent.to_string(),
        "cron": "0 2 1 1 *",
        "time_zone": "UTC",
        "missed_run_policy": "skip",
    })
}

#[tokio::test]
async fn fr_rp_08_a_local_repository_needs_its_host_at_create() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let host = env.enroll_agent("lo-create").await;
    let host = AgentId::from_uuid(Uuid::parse_str(&host.agent_id).expect("an agent id"));

    let (status, body) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/repositories",
            Some(local_body("/srv/backup/lo", None)),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_field(&body, "maintenance_agent_id", "empty");

    let (status, body) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/repositories",
            Some(local_body("/srv/backup/lo", Some(host))),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // A remote backend still accepts no host: any host reaches it.
    let (status, body) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/repositories",
            Some(json!({
                "name": "remote-without-host",
                "backend": {"kind": "rest", "endpoint": "https://rest.example.net/main"},
                "password": "pw",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_the_host_of_a_local_repository_is_locked_after_init() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let first = env.connect_agent("lo-first").await;
    let second = env.connect_agent("lo-second").await;
    let repository = env.create_repository(first.id, "pw").await;

    // Before the init the host may change, but a local repository never loses it.
    let (status, body) = set_host(&env, repository, "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_field(&body, "maintenance_agent_id", "empty");
    let (status, body) = set_host(&env, repository, &second.id.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["maintenance_agent_id"], second.id.to_string());

    mark_initialized(&env, repository).await;
    for value in [first.id.to_string(), String::new()] {
        let (status, body) = set_host(&env, repository, &value).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "repository_host_locked", "{body}");
        assert_eq!(body["field"], "maintenance_agent_id", "{body}");
    }
    // The same host with a rename passes.
    let (status, body) = env
        .call(
            &env.admin,
            "PATCH",
            &format!("/api/v1/repositories/{repository}"),
            Some(json!({"name": "renamed", "maintenance_agent_id": second.id.to_string()})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The host of an initialized remote repository changes as usual.
    let remote = env.create_rest_repository(first.id, "user", "secret").await;
    mark_initialized(&env, remote).await;
    let (status, body) = set_host(&env, remote, &second.id.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_a_host_change_keeps_the_plans_on_the_host() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let first = env.connect_agent("lo-plan-first").await;
    let second = env.connect_agent("lo-plan-second").await;
    let repository = env.create_repository(first.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, first.id).await;

    let (status, body) = set_host(&env, repository, &second.id.to_string()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_field(&body, "maintenance_agent_id", "plan_on_other_host");

    let (status, body) = env
        .call(&env.admin, "DELETE", &format!("/api/v1/plans/{plan}"), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (status, body) = set_host(&env, repository, &second.id.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_a_plan_of_a_local_repository_runs_on_its_host() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let host = env.connect_agent("lo-plan-host").await;
    let other = env.connect_agent("lo-plan-other").await;
    let repository = env.create_repository(host.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;

    let (status, body) = env
        .call(
            &env.operator,
            "POST",
            "/api/v1/plans",
            Some(plan_body(profile, other.id)),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_field(&body, "agent_id", "not_repository_host");

    let plan = env.create_plan(profile, host.id).await;
    let (status, body) = env
        .call(
            &env.operator,
            "PATCH",
            &format!("/api/v1/plans/{plan}"),
            Some(plan_body(profile, other.id)),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_field(&body, "agent_id", "not_repository_host");

    // A remote repository takes a plan on any host.
    let remote = env.create_rest_repository(host.id, "user", "secret").await;
    let remote_profile = env.create_profile(remote, json!({})).await;
    env.create_plan(remote_profile, other.id).await;

    // A profile with a plan on another host cannot move to the local repository.
    let (status, body) = env
        .call(
            &env.operator,
            "PATCH",
            &format!("/api/v1/profiles/{remote_profile}"),
            Some(json!({"repository_id": repository.to_string(), "paths": {"all": ["/srv"]}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_field(&body, "repository_id", "plan_on_other_host");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_the_dispatcher_refuses_a_job_on_another_host() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let host = env.connect_agent("lo-dispatch-host").await;
    let mut other = env.connect_agent("lo-dispatch-other").await;
    let repository = env.create_repository(host.id, "pw").await;
    mark_initialized(&env, repository).await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, host.id).await;
    move_plan(&env, plan, other.id).await;

    let (status, body) = env
        .call(
            &env.operator,
            "POST",
            &format!("/api/v1/plans/{plan}/run"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_field(&body, "agent_id", "not_repository_host");

    for (kind, plan) in [
        (JobKind::Backup, Some(plan)),
        (JobKind::Unlock, None),
        (JobKind::Check, None),
    ] {
        let job = env.enqueue(kind, other.id, repository, plan).await;
        env.pass().await;
        eventually(|| async { env.job_status(job).await == "failed" }).await;
        let ended = env.job(job).await;
        assert_eq!(ended["result_message"], "not_repository_host", "{ended}");
    }
    assert!(
        other
            .try_job(std::time::Duration::from_millis(300))
            .await
            .is_none(),
        "the other host receives no job of the local repository"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rs_01_a_local_repository_restores_and_lists_on_its_host_only() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let host = env.connect_agent("lo-restore-host").await;
    let other = env.connect_agent("lo-restore-other").await;
    let repository = env.create_repository(host.id, "pw").await;
    mark_initialized(&env, repository).await;

    let agent = json!(other.id.to_string());
    for (path, body) in [
        (
            "restore",
            json!({
                "snapshot_id": "0123abcd",
                "target": "/var/lib/restichub-restore/x",
                "overwrite": "always",
                "agent_id": agent,
            }),
        ),
        ("snapshots", json!({"agent_id": agent})),
        ("ls", json!({"snapshot_id": "0123abcd", "agent_id": agent})),
        ("test-access", json!({"agent_id": agent})),
    ] {
        let (status, body) = env
            .call(
                &env.admin,
                "POST",
                &format!("/api/v1/repositories/{repository}/{path}"),
                Some(body),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
        assert_field(&body, "agent_id", "not_repository_host");
    }
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_the_start_check_reports_old_rows_and_keeps_them() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let host = env.connect_agent("lo-report-host").await;
    let other = env.connect_agent("lo-report-other").await;
    let repository = env.create_repository(host.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, host.id).await;
    move_plan(&env, plan, other.id).await;
    let clean = env.create_repository_at(host.id, "pw", "/srv/clean").await;
    let clean_profile = env.create_profile(clean, json!({})).await;
    env.create_plan(clean_profile, host.id).await;
    let hostless = env
        .create_repository_at(host.id, "pw", "/srv/hostless")
        .await;
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE repositories SET maintenance_agent_id = NULL WHERE id = $1")
        .bind(hostless)
        .execute(&mut *tx)
        .await
        .expect("cannot clear the host");
    tx.commit().await.expect("cannot commit the clear");

    let database = env.db.database();
    restichub_cp::startup::report_local_repository_hosts(database)
        .await
        .expect("the check runs");
    let mut tx = database
        .begin_tenant(env.tenant)
        .await
        .expect("cannot begin the transaction");
    let problems = restichub_db::plans::local_host_problems(&mut tx)
        .await
        .expect("cannot list the problems");
    tx.commit().await.expect("cannot end the read");
    let mut found: Vec<(Uuid, Option<Uuid>, Vec<Uuid>)> = problems
        .iter()
        .map(|problem| {
            (
                problem.repository_id.as_uuid(),
                problem.host.map(|host| host.as_uuid()),
                problem
                    .plans_off_host
                    .iter()
                    .map(|plan| plan.as_uuid())
                    .collect(),
            )
        })
        .collect();
    found.sort();
    let mut expected = vec![
        (repository, Some(host.id.as_uuid()), vec![plan]),
        (hostless, None, vec![]),
    ];
    expected.sort();
    assert_eq!(found, expected);

    // The rows stay, and a repository without a host gets one even with plans elsewhere.
    let (status, body) = set_host(&env, hostless, &other.id.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = env
        .call(&env.admin, "GET", &format!("/api/v1/plans/{plan}"), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    env.db.cleanup().await;
}

/// AC-TN-01: the host rule reads the rows of the caller tenant only.
#[tokio::test]
async fn ac_tn_01_the_host_rule_stays_in_the_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let host = env.connect_agent("lo-tenant-host").await;
    let other = env.connect_agent("lo-tenant-other").await;
    let repository = env.create_repository(host.id, "pw").await;
    let profile = env.create_profile(repository, json!({})).await;
    let plan = env.create_plan(profile, host.id).await;
    move_plan(&env, plan, other.id).await;

    let database = env.db.database();
    let foreign = common::create_tenant_with_key(database, "lo-foreign").await;
    let mut tx = database
        .begin_tenant(foreign)
        .await
        .expect("cannot begin the transaction");
    let problems = restichub_db::plans::local_host_problems(&mut tx)
        .await
        .expect("cannot list the problems");
    let off_host = restichub_db::plans::has_plan_off_host(
        &mut tx,
        restichub_db::RepositoryId::from_uuid(repository),
        host.id,
    )
    .await
    .expect("cannot read the plans");
    tx.commit().await.expect("cannot end the read");
    assert!(problems.is_empty(), "{problems:?}");
    assert!(!off_host, "another tenant does not see the plans");
    env.db.cleanup().await;
}
