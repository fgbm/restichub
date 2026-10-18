//! The retention step and the agents that do not know every field: the tag filter must reach the
//! agent in a form that it reads (FR-RP-08, FR-PR-09, ADR-0034).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use axum::http::StatusCode;
use restichub_compiler::ValidatedJob;
use restichub_compiler::arg::build_command;
use restichub_db::AgentId;
use restichub_proto::agent::v1::{ForgetStep, Job, step};
use serde_json::{Value, json};
use uuid::Uuid;

use queue_support::{Env, test_queue};

async fn initialized_repository(env: &Env, agent: AgentId) -> Uuid {
    // A remote backend: the tests run backups on hosts other than the maintenance agent.
    let repository = env.create_rest_repository(agent, "user", "pw").await;
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE repositories SET initialized = true WHERE id = $1")
        .bind(repository)
        .execute(&mut *tx)
        .await
        .expect("cannot mark the repository initialized");
    tx.commit().await.expect("cannot commit the mark");
    repository
}

async fn run_retention(env: &Env, repository: Uuid) -> Uuid {
    let (status, queued) = env
        .call(
            &env.operator,
            "POST",
            &format!("/api/v1/repositories/{repository}/maintenance/run"),
            Some(json!({"task": "retention"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    Uuid::parse_str(queued["id"].as_str().expect("the job id")).expect("the job id is a UUID")
}

fn forget(job: &Job) -> &ForgetStep {
    job.steps
        .iter()
        .find_map(|step| match &step.kind {
            Some(step::Kind::Forget(forget)) => Some(forget),
            _ => None,
        })
        .expect("a forget step")
}

fn forget_args(job: &Job) -> Vec<String> {
    let validated = ValidatedJob::new(job.clone()).expect("the job validates");
    let step = job
        .steps
        .iter()
        .find(|step| matches!(step.kind, Some(step::Kind::Forget(_))))
        .expect("a forget step");
    build_command(&validated, step)
        .expect("the step builds")
        .args()
        .to_vec()
}

#[tokio::test]
async fn fr_rp_08_an_agent_without_the_capability_gets_the_profile_tag_in_tags() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent_with("old-host", Vec::new()).await;
    let repository = initialized_repository(&env, agent.id).await;
    let profile = env
        .create_profile(json!({"retention": {"keep_last": 2}}))
        .await;
    env.create_plan(profile, agent.id, repository).await;

    run_retention(&env, repository).await;
    env.pass().await;
    let signed = agent.job().await;
    let job = env.verify(&signed, &agent).await;

    let filter = forget(&job).filter.as_ref().expect("a filter");
    assert_eq!(filter.tags, [format!("restichub-profile-{profile}")]);
    assert!(filter.all_tags.is_empty(), "an old agent drops all_tags");
    assert!(
        forget_args(&job).contains(&format!("--tag=restichub-profile-{profile}")),
        "{:?}",
        forget_args(&job)
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_an_agent_without_the_capability_never_gets_all_tags() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent_with("old-host", Vec::new()).await;
    let repository = initialized_repository(&env, agent.id).await;
    let profile = env
        .create_profile(json!({"retention": {"keep_last": 2, "filter": {"tags": ["daily"]}}}))
        .await;
    env.create_plan(profile, agent.id, repository).await;

    let job = run_retention(&env, repository).await;
    env.pass().await;

    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_code"], "config_error");
    assert_eq!(body["result_message"], "agent_capability_missing");
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_pr_09_the_hello_stores_the_capabilities() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("new-host").await;
    assert_eq!(
        env.stored_capabilities(agent.id).await,
        [
            "forget-all-tags".to_owned(),
            "restore-v1".to_owned(),
            "job-limits-v1".to_owned(),
            "scripts-v1".to_owned(),
            "script-masked-value".to_owned(),
            "log-ack-v1".to_owned()
        ]
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_pr_09_a_hello_with_junk_stores_only_the_known_capabilities() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut names: Vec<String> = (0..10_000).map(|index| format!("junk-{index}")).collect();
    names.push("forget-all-tags".to_owned());
    names.push("forget-all-tags".to_owned());
    let agent = env.connect_agent_with("junk-host", names).await;
    assert_eq!(
        env.stored_capabilities(agent.id).await,
        ["forget-all-tags".to_owned()]
    );
    env.db.cleanup().await;
}

/// The agent row held the capability from an earlier connection. The agent came back from an older
/// package with the same identity, so the new connection announces nothing.
#[tokio::test]
async fn fr_pr_09_a_downgraded_agent_never_gets_all_tags_in_a_first_send() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let capable = env.connect_agent("down-host").await;
    let mut agent = env.reconnect_agent(&capable, Vec::new()).await;
    let repository = initialized_repository(&env, agent.id).await;
    let profile = env
        .create_profile(json!({"retention": {"keep_last": 2, "filter": {"tags": ["daily"]}}}))
        .await;
    env.create_plan(profile, agent.id, repository).await;

    let job = run_retention(&env, repository).await;
    env.pass().await;

    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_message"], "agent_capability_missing");
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_pr_09_a_redelivery_to_a_downgraded_agent_fails_and_is_not_sent() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut capable = env.connect_agent("redeliver-host").await;
    let repository = initialized_repository(&env, capable.id).await;
    let profile = env
        .create_profile(json!({"retention": {"keep_last": 2, "filter": {"tags": ["daily"]}}}))
        .await;
    env.create_plan(profile, capable.id, repository).await;

    let job = run_retention(&env, repository).await;
    env.pass().await;
    let signed = capable.job().await;
    let sent = env.verify(&signed, &capable).await;
    assert_eq!(
        forget(&sent)
            .filter
            .as_ref()
            .expect("a filter")
            .all_tags
            .len(),
        2
    );

    let mut downgraded = env.reconnect_agent(&capable, Vec::new()).await;
    env.clock.advance(61);
    env.pass().await;

    let body = env.job(job).await;
    assert_eq!(body["status"], "failed");
    assert_eq!(body["result_message"], "agent_capability_missing");
    assert!(
        downgraded
            .try_job(Duration::from_millis(300))
            .await
            .is_none()
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_pr_09_a_reconnected_capable_agent_still_gets_all_tags() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let first = env.connect_agent_with("up-host", Vec::new()).await;
    let mut agent = env
        .reconnect_agent(&first, vec!["forget-all-tags".to_owned()])
        .await;
    let repository = initialized_repository(&env, agent.id).await;
    let profile = env
        .create_profile(json!({"retention": {"keep_last": 2, "filter": {"tags": ["daily"]}}}))
        .await;
    env.create_plan(profile, agent.id, repository).await;

    run_retention(&env, repository).await;
    env.pass().await;
    let signed = agent.job().await;
    let job = env.verify(&signed, &agent).await;
    assert_eq!(
        forget(&job).filter.as_ref().expect("a filter").all_tags,
        [format!("restichub-profile-{profile}"), "daily".to_owned()]
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_the_grouping_of_a_shared_profile_always_holds_the_host() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut maintenance = env.connect_agent("host-a").await;
    let other = env.connect_agent("host-b").await;
    let repository = initialized_repository(&env, maintenance.id).await;
    let profile = env
        .create_profile(json!({"retention": {"keep_last": 2, "group_by": ["paths"]}}))
        .await;
    env.create_plan(profile, maintenance.id, repository).await;
    env.create_plan(profile, other.id, repository).await;

    run_retention(&env, repository).await;
    env.pass().await;
    let signed = maintenance.job().await;
    let job = env.verify(&signed, &maintenance).await;

    assert!(
        forget_args(&job).contains(&"--group-by=host,paths".to_owned()),
        "{:?}",
        forget_args(&job)
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_a_template_in_the_host_filter_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let (status, problem): (StatusCode, Value) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/profiles",
            Some(json!({
                "name": "templated",
                "paths": {"all": ["/srv"]},
                "retention": {"keep_last": 1, "filter": {"hosts": ["{{hostname}}"]}},
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
    assert_eq!(problem["field"], "retention.filter.hosts", "{problem}");
    env.db.cleanup().await;
}

async fn wait_offline(env: &Env, agent: AgentId) {
    queue_support::eventually(|| async {
        env.state.agent_registry().capabilities(agent).is_none()
    })
    .await;
}

#[tokio::test]
async fn fr_pr_09_an_agent_that_left_after_the_snapshot_keeps_its_queued_job() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("snapshot-host").await;
    let agent_id = agent.id;
    let repository = initialized_repository(&env, agent_id).await;
    let profile = env
        .create_profile(json!({"retention": {"keep_last": 2, "filter": {"tags": ["daily"]}}}))
        .await;
    env.create_plan(profile, agent_id, repository).await;
    let job = run_retention(&env, repository).await;

    let online = env.state.agent_registry().online_agents();
    assert_eq!(online, vec![agent_id]);
    drop(agent);
    wait_offline(&env, agent_id).await;
    restichub_cp::queue::run_with_online(&env.state, env.now(), &online)
        .await
        .expect("the dispatcher pass");

    let body = env.job(job).await;
    assert_eq!(body["status"], "queued", "{body}");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_pr_09_an_agent_that_left_after_the_snapshot_keeps_the_lease_of_a_redelivery() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("redeliver-offline-host").await;
    let agent_id = agent.id;
    let repository = initialized_repository(&env, agent_id).await;
    let profile = env
        .create_profile(json!({"retention": {"keep_last": 2, "filter": {"tags": ["daily"]}}}))
        .await;
    env.create_plan(profile, agent_id, repository).await;
    let job = run_retention(&env, repository).await;
    env.pass().await;
    let _ = agent.job().await;

    let online = env.state.agent_registry().online_agents();
    drop(agent);
    wait_offline(&env, agent_id).await;
    env.clock.advance(61);
    restichub_cp::queue::run_with_online(&env.state, env.now(), &online)
        .await
        .expect("the dispatcher pass");

    let body = env.job(job).await;
    assert_eq!(body["status"], "sent", "{body}");
    env.db.cleanup().await;
}
