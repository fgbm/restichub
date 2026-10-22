//! The queue dispatcher: the job builder, the secrets, the lease and the tenant status (FR-PR-03,
//! FR-PR-05, FR-PR-06, FR-SC-03, FR-SC-05, AC-PR-03, AC-PR-04, AC-RP-03, AC-TN-03).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use axum::http::StatusCode;
use restichub_proto::agent::v1::repository_access::BackendCredentials;
use restichub_proto::agent::v1::{script_env, step};
use serde_json::json;

use queue_support::{Env, decode_job_id, test_queue};

/// Reads the audit records of one action. The owner login is scoped to the tenant.
async fn audit_rows(env: &Env, action: &str) -> Vec<String> {
    let mut tx = env.scoped().await;
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT coalesce(diff::text, '') || ' ' || coalesce(object_id::text, '') \
         FROM audit_log WHERE action = $1",
    )
    .bind(action)
    .fetch_all(&mut *tx)
    .await
    .expect("cannot read the audit log");
    tx.commit().await.expect("cannot end the read");
    rows
}

async fn delivered_count(env: &Env) -> i64 {
    audit_rows(env, "secret_delivered").await.len() as i64
}

#[tokio::test]
async fn ac_pr_03_the_job_carries_only_the_secrets_of_its_own_repository() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("pr03-host").await;
    let repository_a = env.create_repository(agent.id, "canary-secret-of-a").await;
    let repository_b = env.create_repository(agent.id, "canary-secret-of-b").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository_a).await;
    let other_profile = env.create_profile(json!({})).await;
    let _other_plan = env.create_plan(other_profile, agent.id, repository_b).await;

    let job_id = env.run_plan(plan).await;
    env.pass().await;

    let signed = agent.job().await;
    let job = env.verify(&signed, &agent).await;
    assert_eq!(job.job_id, job_id.to_string());
    assert_eq!(job.repositories.len(), 1, "one repository in the job");
    assert_eq!(job.repositories[0].repository_id, repository_a.to_string());
    assert_eq!(job.repositories[0].password, "canary-secret-of-a");
    assert!(
        !String::from_utf8_lossy(&signed.job).contains("canary-secret-of-b"),
        "the secret of another repository never reaches the job"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_rp_01_a_rest_job_carries_the_user_name_and_the_password() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("rest-host").await;
    let repository = env
        .create_rest_repository(agent.id, "backup", "rest-canary-password")
        .await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;

    env.run_plan(plan).await;
    env.pass().await;

    let signed = agent.job().await;
    let job = env.verify(&signed, &agent).await;
    let access = &job.repositories[0];
    assert_eq!(access.location, "rest:https://rest.example.net/main");
    match &access.backend_credentials {
        Some(BackendCredentials::Rest(credentials)) => {
            assert_eq!(credentials.username, "backup");
            assert_eq!(credentials.password, "rest-canary-password");
        }
        other => panic!("the job holds no rest credentials: {other:?}"),
    }
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_05_every_restic_step_carries_retry_lock() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("lock-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;

    env.run_plan(plan).await;
    env.pass().await;

    let signed = agent.job().await;
    let job = env.verify(&signed, &agent).await;
    let backup = job
        .steps
        .iter()
        .find_map(|step| match &step.kind {
            Some(step::Kind::Backup(backup)) => Some(backup),
            _ => None,
        })
        .expect("the job holds a backup step");
    assert!(
        backup.retry_lock,
        "the control plane always sends --retry-lock"
    );
    env.db.cleanup().await;
}

/// AC-AG-12, AC-AG-13, AC-AG-10: the job carries the stall timeout and the maximum duration of its
/// profile, and the backup step carries the transfer limits of the profile.
#[tokio::test]
async fn ac_ag_12_the_job_carries_the_limits_of_its_profile() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("limits-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env
        .create_profile(json!({
            "options": {
                "stall_timeout_minutes": 5,
                "max_duration_minutes": 7,
                "limit_upload_kib": 1000,
                "limit_download_kib": 500,
            }
        }))
        .await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job_id = env.run_plan(plan).await;
    env.pass().await;

    let signed = agent.job().await;
    let job = env.verify(&signed, &agent).await;
    assert_eq!(job.stall_timeout_seconds, 5 * 60);
    assert_eq!(job.timeout_seconds, 7 * 60);
    let backup = job
        .steps
        .iter()
        .find_map(|step| match &step.kind {
            Some(step::Kind::Backup(backup)) => Some(backup),
            _ => None,
        })
        .expect("the job holds a backup step");
    assert_eq!(backup.limit_upload_kib, 1000);
    assert_eq!(backup.limit_download_kib, 500);

    let detail = env.job(job_id).await;
    assert_eq!(detail["limits"]["stall_timeout_seconds"], 5 * 60);
    assert_eq!(detail["limits"]["max_duration_seconds"], 7 * 60);
    env.db.cleanup().await;
}

/// AC-AG-14, FR-AG-15: the steps of a job carry the low priority that the profile asks for.
#[tokio::test]
async fn ac_ag_14_the_job_carries_the_priority_of_its_profile() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("priority-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env
        .create_profile(json!({"options": {"priority": "low"}}))
        .await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    env.run_plan(plan).await;
    env.pass().await;

    let signed = agent.job().await;
    let job = env.verify(&signed, &agent).await;
    let low = restichub_proto::agent::v1::ProcessPriority::Low as i32;
    let backup = job
        .steps
        .iter()
        .find_map(|step| match &step.kind {
            Some(step::Kind::Backup(backup)) => Some(backup),
            _ => None,
        })
        .expect("the job holds a backup step");
    assert_eq!(backup.priority, low);
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_rp_03_the_delivery_record_commits_before_the_job_is_sent() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("rp03-host").await;
    let repository = env
        .create_repository(agent.id, "rp03-canary-password")
        .await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;

    env.run_plan(plan).await;
    assert_eq!(
        delivered_count(&env).await,
        0,
        "a queued job reads no secret"
    );
    env.pass().await;
    let signed = agent.job().await;
    let job_id = decode_job_id(&signed);

    // The agent holds the job, so the record is durable already.
    assert!(delivered_count(&env).await >= 1);
    let rows = audit_rows(&env, "secret_delivered").await;
    assert!(
        rows.iter().all(|row| !row.contains("rp03-canary-password")),
        "the record holds no secret value"
    );
    assert!(
        rows.iter().any(|row| row.contains(&job_id)),
        "the record names the job"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_pr_04_an_unacknowledged_job_is_delivered_again_after_the_lease() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("pr04-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job = env.run_plan(plan).await;

    env.pass().await;
    let first = decode_job_id(&agent.job().await);
    assert_eq!(first, job.to_string());
    assert_eq!(env.job_status(job).await, "sent");

    // Inside the lease the job does not go out again.
    env.clock.advance(30);
    env.pass().await;
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());

    // After the lease it goes out once more, with the same identifier.
    env.clock.advance(31);
    env.pass().await;
    let second = decode_job_id(&agent.job().await);
    assert_eq!(second, first, "the agent journal dedupes by job_id");

    agent.ack(&first).await;
    queue_support::eventually(|| async { env.job_status(job).await == "running" }).await;
    env.clock.advance(120);
    env.pass().await;
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_pr_04_a_job_that_is_never_acknowledged_fails_after_the_delivery_limit() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let mut queue = test_queue();
    queue.max_deliveries = 2;
    let env = Env::start(db, queue).await;
    let mut agent = env.connect_agent("pr04-limit").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job = env.run_plan(plan).await;

    for _ in 0..2 {
        env.pass().await;
        agent.job().await;
        env.clock.advance(61);
    }
    env.pass().await;
    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_message"], "not_acknowledged");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_03_a_suspended_tenant_dispatches_nothing_and_its_agents_stay_connected() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("tn03-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job = env.run_plan(plan).await;

    common::set_tenant_status(env.db.database(), env.tenant, "suspended").await;
    env.pass().await;
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());
    assert!(
        env.state.agent_registry().is_online(agent.id),
        "the agent stays connected"
    );

    common::set_tenant_status(env.db.database(), env.tenant, "active").await;
    env.pass().await;
    assert_eq!(decode_job_id(&agent.job().await), job.to_string());
    env.db.cleanup().await;
}

#[tokio::test]
async fn an_offline_agent_gets_no_send_and_no_secret_is_read() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let first = env.connect_agent("later-host").await;
    let repository = env.create_repository(first.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, first.id, repository).await;
    let job = env.run_plan(plan).await;
    let agent_id = first.id;
    drop(first);
    queue_support::eventually(|| async { !env.state.agent_registry().is_online(agent_id) }).await;

    env.pass().await;
    assert_eq!(
        env.job_status(job).await,
        "queued",
        "no online agent, no send"
    );
    assert_eq!(
        delivered_count(&env).await,
        0,
        "no secret is read for an offline agent"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn two_dispatcher_instances_never_send_the_same_job_twice() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("twice-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let mut expected = Vec::new();
    for _ in 0..8 {
        expected.push(env.run_fresh_plan(repository, agent.id).await.to_string());
    }

    let now = env.now();
    let (a, b, c) = tokio::join!(
        restichub_cp::queue::run_once(&env.state, now),
        restichub_cp::queue::run_once(&env.state, now),
        restichub_cp::queue::run_once(&env.state, now),
    );
    a.expect("the first instance");
    b.expect("the second instance");
    c.expect("the third instance");

    let mut delivered = agent.jobs_within(Duration::from_millis(1500)).await;
    delivered.sort();
    expected.sort();
    assert_eq!(delivered, expected, "each job goes out exactly once");
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_ag_09_an_agent_with_an_older_restic_gets_no_job_and_the_status_is_incompatible() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("compile-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job = env.run_plan(plan).await;

    // The agent downgrades restic below the supported minimum after the job was queued.
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE agents SET restic_version = '0.1.0' WHERE id = $1")
        .bind(agent.id)
        .execute(&mut *tx)
        .await
        .expect("cannot update the agent");
    tx.commit().await.expect("cannot commit the update");
    env.pass().await;

    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_code"], "incompatible");
    assert_eq!(body["result_message"], "incompatible");
    assert_eq!(
        body["result_params"],
        json!({
            "option": null,
            "required_version": "0.18.1",
            "installed_version": "0.1.0",
        })
    );
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());
    assert_eq!(
        delivered_count(&env).await,
        0,
        "a refused job reads no secret"
    );
    env.db.cleanup().await;
}

/// ADR-0047: a job keeps the repository that its plan named when the queue took it, so a plan that
/// moves to another repository does not write there.
#[tokio::test]
async fn adr_0047_a_queued_backup_fails_when_its_plan_moves_to_another_repository() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("repo-change-host").await;
    let first = env.create_repository(agent.id, "first-password").await;
    let second = env.create_repository(agent.id, "second-password").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, first).await;
    let job = env.run_plan(plan).await;

    // The plan moves to another repository after the job was queued.
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE plans SET repository_id = $2 WHERE id = $1")
        .bind(plan)
        .bind(second)
        .execute(&mut *tx)
        .await
        .expect("cannot move the plan");
    tx.commit().await.expect("cannot commit the move");
    env.pass().await;

    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_code"], "failed");
    assert_eq!(body["result_message"], "repository_changed");
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());
    assert_eq!(
        delivered_count(&env).await,
        0,
        "a refused job reads no secret"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn a_profile_that_no_longer_compiles_fails_the_job_with_a_typed_reason() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("compile-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env.create_profile(json!({})).await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job = env.run_plan(plan).await;

    // The profile loses its paths after the job was queued.
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE profiles SET paths_all = '{}' WHERE id = $1")
        .bind(profile)
        .execute(&mut *tx)
        .await
        .expect("cannot update the profile");
    tx.commit().await.expect("cannot commit the update");
    env.pass().await;

    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_code"], "config_error");
    assert_eq!(body["result_message"], "compile_failed");
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());
    env.db.cleanup().await;
}

/// FR-PF-11: the job builder checks the values again and ends the job with `config_error`.
#[tokio::test]
async fn fr_pf_11_a_job_without_a_required_parameter_ends_with_config_error() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("parameter-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env
        .create_profile(json!({
            "paths": {"all": ["/data/{{param.pg_dir}}"]},
            "parameters": [{"name": "pg_dir", "kind": "text", "default_value": "/srv/pg"}],
        }))
        .await;
    let plan = env.create_plan(profile, agent.id, repository).await;
    let job = env.run_plan(plan).await;

    // The value goes away after the job was queued: the plan keeps nothing to back up.
    let mut tx = env.scoped().await;
    sqlx::query(
        "UPDATE profiles SET parameters = '[{\"name\": \"pg_dir\", \"kind\": \"text\", \
         \"required\": true}]'::jsonb WHERE id = $1",
    )
    .bind(profile)
    .execute(&mut *tx)
    .await
    .expect("cannot update the profile");
    tx.commit().await.expect("cannot commit the update");
    env.pass().await;

    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_code"], "config_error");
    assert_eq!(body["result_message"], "parameter_missing");
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());
    env.db.cleanup().await;
}

/// The value of one variable of the first script step of a job.
fn script_value(job: &restichub_proto::agent::v1::Job, name: &str) -> script_env::Value {
    let Some(step::Kind::Script(script)) = &job.steps[0].kind else {
        panic!("the first step is a script");
    };
    script
        .env
        .iter()
        .find(|variable| variable.name == name)
        .and_then(|variable| variable.value.clone())
        .expect("the variable")
}

/// FR-PF-12: a text parameter reaches an old agent as a secret value, because that agent rejects
/// `masked_value` and masks a secret value the same way (ADR-0047).
#[tokio::test]
async fn fr_pf_12_an_old_agent_receives_a_parameter_value_as_a_secret_value() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let interpreters = vec![
        restichub_proto::agent::v1::ScriptInterpreter::Bash,
        restichub_proto::agent::v1::ScriptInterpreter::Sh,
    ];
    let agent = env
        .connect_agent_trusting("masked-host", true, interpreters.clone())
        .await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env
        .create_profile(json!({
            "paths": {"all": ["/data/{{param.pg_dir}}"]},
            "parameters": [{"name": "pg_dir", "kind": "text", "required": true}],
            "scripts": [{
                "name": "dump",
                "interpreter": "bash",
                "phase": "before",
                "body": "echo dump",
                "timeout_seconds": 60,
                "failure_policy": "abort_backup",
                "env": [{"name": "PGDIR", "kind": "parameter", "value": "pg_dir"}],
            }],
        }))
        .await;
    let (status, body) = env
        .call(
            &env.admin,
            "PUT",
            &format!("/api/v1/agents/{}/parameters", agent.id),
            Some(json!({"parameters": [{"name": "pg_dir", "kind": "text", "value": "/srv/pg"}]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let plan = env.create_plan(profile, agent.id, repository).await;

    // The agent that announces `script-masked-value` keeps the marking of the text value.
    let mut current = agent;
    env.run_plan(plan).await;
    env.pass().await;
    let signed = current.job().await;
    let job = env.verify(&signed, &current).await;
    assert_eq!(
        script_value(&job, "PGDIR"),
        script_env::Value::MaskedValue("/srv/pg".to_owned())
    );

    // The agent comes back from a package that knows no `masked_value`.
    let old: Vec<String> = restichub_proto::AgentCapability::ALL
        .iter()
        .filter(|capability| **capability != restichub_proto::AgentCapability::ScriptMaskedValue)
        .map(|capability| capability.as_str().to_owned())
        .collect();
    let mut current = env
        .reconnect_agent_reporting(
            &current,
            old,
            queue_support::scripts_report(true, interpreters),
        )
        .await;
    env.run_plan(plan).await;
    env.pass().await;
    let signed = current.job().await;
    let job = env.verify(&signed, &current).await;
    assert_eq!(
        script_value(&job, "PGDIR"),
        script_env::Value::SecretValue("/srv/pg".to_owned())
    );
    env.db.cleanup().await;
}
