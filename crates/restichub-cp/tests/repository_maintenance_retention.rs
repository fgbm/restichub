//! The retention step of the maintenance job: the reserved tags, the profile filter, the isolation
//! key and the result of a `forget` that selects nothing (FR-RP-08, FR-PF-01, ADR-0034).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use axum::http::StatusCode;
use restichub_compiler::ValidatedJob;
use restichub_compiler::arg::build_command;
use restichub_db::AgentId;
use restichub_proto::agent::v1::{Job, Step, step};
use serde_json::{Value, json};
use uuid::Uuid;

use queue_support::{Env, decode_job_id, eventually, test_queue};

async fn initialized_repository(env: &Env, agent: AgentId) -> Uuid {
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

async fn run_retention(env: &Env, repository: Uuid) -> (StatusCode, Value) {
    env.call(
        &env.operator,
        "POST",
        &format!("/api/v1/repositories/{repository}/maintenance/run"),
        Some(json!({"task": "retention"})),
    )
    .await
}

fn forget_steps(job: &Job) -> Vec<&Step> {
    job.steps
        .iter()
        .filter(|step| matches!(step.kind, Some(step::Kind::Forget(_))))
        .collect()
}

fn argv(job: &Job, step: &Step) -> Vec<String> {
    let validated = ValidatedJob::new(job.clone()).expect("the job validates");
    build_command(&validated, step)
        .expect("the step builds")
        .args()
        .to_vec()
}

async fn create_profile_body(env: &Env, repository: Uuid, extra: Value) -> (StatusCode, Value) {
    let mut body = json!({
        "name": format!("profile-{}", Uuid::now_v7()),
        "repository_id": repository.to_string(),
        "paths": {"all": ["/srv"]},
    });
    for (key, value) in extra.as_object().expect("the extra fields") {
        body[key] = value.clone();
    }
    env.call(&env.admin, "POST", "/api/v1/profiles", Some(body))
        .await
}

#[tokio::test]
async fn fr_rp_08_a_reserved_tag_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("reserved-host").await;
    let repository = env.create_repository(agent.id, "pw").await;

    let cases = [
        ("tags", json!({"tags": ["restichub-profile-x"]})),
        ("tags", json!({"tags": ["ResticHub-x"]})),
        (
            "retention.keep_tags",
            json!({"retention": {"keep_last": 1, "keep_tags": ["restichub-profile-x"]}}),
        ),
        (
            "retention.filter.tags",
            json!({"retention": {"keep_last": 1, "filter": {"tags": ["restichub-profile-x"]}}}),
        ),
    ];
    for (field, extra) in cases {
        let (status, problem) = create_profile_body(&env, repository, extra).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");
        assert_eq!(problem["field"], field, "{problem}");
        assert_eq!(problem["reason"], "reserved", "{problem}");
    }
    let (status, problem) = create_profile_body(&env, repository, json!({"tags": ["daily"]})).await;
    assert_eq!(status, StatusCode::CREATED, "{problem}");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_the_profile_filter_narrows_the_forget_step() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut maintenance = env.connect_agent("host-a").await;
    let backup = env.connect_agent("host-b").await;
    let repository = initialized_repository(&env, maintenance.id).await;
    let profile = env
        .create_profile(
            repository,
            json!({"retention": {
                "keep_last": 2,
                "filter": {"hosts": ["host-a"], "tags": ["daily", "db"], "paths": ["/srv"]},
            }}),
        )
        .await;
    env.create_plan(profile, maintenance.id).await;
    env.create_plan(profile, backup.id).await;

    let (status, queued) = run_retention(&env, repository).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    env.pass().await;
    let signed = maintenance.job().await;
    let job = env.verify(&signed, &maintenance).await;

    let forgets = forget_steps(&job);
    assert_eq!(forgets.len(), 1, "one forget step for the profile");
    let args = argv(&job, forgets[0]);
    assert!(args.contains(&"--host=host-a".to_owned()), "{args:?}");
    assert!(!args.contains(&"--host=host-b".to_owned()), "{args:?}");
    assert!(
        args.contains(&format!("--tag=restichub-profile-{profile},daily,db")),
        "the profile tag and the filter tags form one AND list: {args:?}"
    );
    assert!(args.contains(&"--path=/srv".to_owned()), "{args:?}");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_a_plan_agent_outside_the_host_filter_gives_no_forget_step() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut maintenance = env.connect_agent("maintenance-host").await;
    let backup = env.connect_agent("host-b").await;
    let repository = initialized_repository(&env, maintenance.id).await;
    let profile = env
        .create_profile(
            repository,
            json!({"retention": {"keep_last": 2, "filter": {"hosts": ["host-a"]}}}),
        )
        .await;
    env.create_plan(profile, backup.id).await;
    let other = env
        .create_profile(repository, json!({"retention": {"keep_last": 1}}))
        .await;
    env.create_plan(other, backup.id).await;

    let (status, queued) = run_retention(&env, repository).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    env.pass().await;
    let signed = maintenance.job().await;
    let job = env.verify(&signed, &maintenance).await;

    let forgets = forget_steps(&job);
    assert_eq!(forgets.len(), 1, "only the unfiltered profile");
    assert!(
        argv(&job, forgets[0]).contains(&format!("--tag=restichub-profile-{other}")),
        "the filtered profile gives no step"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_the_forget_step_does_not_depend_on_the_agent_name() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut maintenance = env.connect_agent("maintenance-host").await;
    let first = env.connect_agent("host-one").await;
    let second = env.connect_agent("host-two").await;
    let repository = initialized_repository(&env, maintenance.id).await;
    let profile = env
        .create_profile(repository, json!({"retention": {"keep_last": 2}}))
        .await;
    env.create_plan(profile, first.id).await;
    env.create_plan(profile, second.id).await;

    let mut lines = Vec::new();
    for hostname in ["host-one", "renamed-host"] {
        sqlx::query("UPDATE agents SET hostname = $2 WHERE id = $1")
            .bind(first.id)
            .bind(hostname)
            .execute(&mut *env.scoped().await)
            .await
            .expect("cannot rename the agent");
        let (status, queued) = run_retention(&env, repository).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
        env.pass().await;
        let signed = maintenance.job().await;
        let job = env.verify(&signed, &maintenance).await;
        let forgets = forget_steps(&job);
        assert_eq!(
            forgets.len(),
            1,
            "one forget step for a profile on two hosts"
        );
        let args = argv(&job, forgets[0]);
        assert!(
            args.contains(&"--group-by=host,paths".to_owned()),
            "{args:?}"
        );
        assert!(
            !args.iter().any(|arg| arg.starts_with("--host")),
            "{args:?}"
        );
        lines.push(args);
        let id = Uuid::parse_str(&decode_job_id(&signed)).expect("job id");
        maintenance.start(&env, &id.to_string()).await;
        maintenance.finish(&env, &id.to_string()).await;
    }
    assert_eq!(lines[0], lines[1], "a rename does not change the argv");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_a_forget_that_selects_nothing_gives_a_warning() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut maintenance = env.connect_agent("maintenance-host").await;
    let repository = initialized_repository(&env, maintenance.id).await;
    let profile = env
        .create_profile(repository, json!({"retention": {"keep_last": 2}}))
        .await;
    env.create_plan(profile, maintenance.id).await;

    for (selected, expected) in [(false, "succeeded_with_warnings"), (true, "succeeded")] {
        let (status, queued) = run_retention(&env, repository).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
        env.pass().await;
        let signed = maintenance.job().await;
        let id = decode_job_id(&signed);
        let job = Uuid::parse_str(&id).expect("job id");
        maintenance.start(&env, &id).await;
        maintenance.forget_step(&id, selected).await;
        if !selected {
            eventually(|| async {
                let mut tx = env.scoped().await;
                sqlx::query_scalar::<_, bool>(
                    "SELECT empty_forget_reported FROM jobs WHERE id = $1",
                )
                .bind(job)
                .fetch_one(&mut *tx)
                .await
                .expect("cannot read the flag")
            })
            .await;
        }
        maintenance
            .result(
                &id,
                restichub_proto::agent::v1::JobStatus::Succeeded,
                restichub_proto::agent::v1::StepStatus::Success,
            )
            .await;
        eventually(|| async { env.job_status(job).await == "succeeded" }).await;
        let ended = env.job(job).await;
        assert_eq!(ended["result_code"], expected, "{ended}");
        if !selected {
            assert_eq!(
                ended["result_message"], "forget_selected_nothing",
                "{ended}"
            );
        }
    }
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_pf_01_the_check_step_reads_the_largest_subset() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut maintenance = env.connect_agent("maintenance-host").await;
    let repository = initialized_repository(&env, maintenance.id).await;
    for subset in ["5%", "1/2", "30%"] {
        env.create_profile(repository, json!({"check": {"read_data_subset": subset}}))
            .await;
    }

    let (status, queued) = env
        .call(
            &env.operator,
            "POST",
            &format!("/api/v1/repositories/{repository}/maintenance/run"),
            Some(json!({"task": "check"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    env.pass().await;
    let signed = maintenance.job().await;
    let job = env.verify(&signed, &maintenance).await;
    let check = job
        .steps
        .iter()
        .find(|step| matches!(step.kind, Some(step::Kind::Check(_))))
        .expect("the check step");
    assert!(
        argv(&job, check).contains(&"--read-data-subset=1/2".to_owned()),
        "{:?}",
        argv(&job, check)
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_only_a_plan_of_an_active_agent_counts_for_the_retention() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let maintenance = env.connect_agent("maintenance-host").await;
    let backup = env.connect_agent("backup-host").await;
    let repository = initialized_repository(&env, maintenance.id).await;
    let profile = env
        .create_profile(repository, json!({"retention": {"keep_last": 2}}))
        .await;
    env.create_plan(profile, backup.id).await;
    let (status, revoked) = env
        .call(
            &env.admin,
            "POST",
            &format!("/api/v1/agents/{}/revoke", backup.id),
            None,
        )
        .await;
    assert!(status.is_success(), "{revoked}");

    let (status, body) = run_retention(&env, repository).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    env.db.cleanup().await;
}
