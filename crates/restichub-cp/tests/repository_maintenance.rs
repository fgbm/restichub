//! The repository maintenance: the agent rule, the retention isolation, the schedules and the role
//! rules (FR-RP-08, FR-SC-04, FR-PF-01, AC-RP-04, AC-SC-02, AC-TN-01, ADR-0034).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use std::time::Duration;

use axum::http::StatusCode;
use restichub_compiler::ValidatedJob;
use restichub_compiler::arg::build_command;
use restichub_db::{AgentId, JobKind};
use restichub_proto::agent::v1::{Job, Step, step};
use serde_json::{Value, json};
use uuid::Uuid;

use queue_support::{Env, FakeAgent, eventually, test_queue};

/// Creates a repository whose maintenance agent is `agent` and marks it initialized.
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

fn run_uri(repository: Uuid) -> String {
    format!("/api/v1/repositories/{repository}/maintenance/run")
}

fn maintenance_uri(repository: Uuid) -> String {
    format!("/api/v1/repositories/{repository}/maintenance")
}

async fn run(env: &Env, token: &str, repository: Uuid, task: &str) -> (StatusCode, Value) {
    env.call(
        token,
        "POST",
        &run_uri(repository),
        Some(json!({"task": task})),
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

async fn retention_job_count(env: &Env, repository: Uuid) -> usize {
    let (status, body) = env
        .call(
            &env.admin,
            "GET",
            &format!("/api/v1/jobs?repository_id={repository}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["items"]
        .as_array()
        .expect("the items")
        .iter()
        .filter(|job| job["kind"] == "forget" || job["kind"] == "check")
        .count()
}

async fn two_agents(env: &Env) -> (FakeAgent, FakeAgent) {
    let maintenance = env.connect_agent("maintenance-host").await;
    let backup = env.connect_agent("backup-host").await;
    (maintenance, backup)
}

#[tokio::test]
async fn fr_rp_08_the_retention_of_a_profile_removes_only_its_own_snapshots() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let (mut maintenance, mut backup) = two_agents(&env).await;
    let repository = initialized_repository(&env, maintenance.id).await;
    let profile_a = env
        .create_profile(
            repository,
            json!({"paths": {"all": ["/srv/a"]}, "retention": {"keep_last": 3}}),
        )
        .await;
    let profile_b = env
        .create_profile(
            repository,
            json!({"paths": {"all": ["/srv/b"]}, "retention": {"keep_daily": 7}}),
        )
        .await;
    env.create_plan(profile_a, maintenance.id).await;
    env.create_plan(profile_b, backup.id).await;

    let (status, queued) = run(&env, &env.operator, repository, "retention").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    env.pass().await;

    let signed = maintenance.job().await;
    let job = env.verify(&signed, &maintenance).await;
    assert!(
        backup.try_job(Duration::from_millis(300)).await.is_none(),
        "the backup host receives no maintenance step (AC-RP-04)"
    );
    let forgets = forget_steps(&job);
    assert_eq!(forgets.len(), 2, "one forget step for each profile");
    assert!(matches!(
        job.steps.last().and_then(|step| step.kind.as_ref()),
        Some(step::Kind::Prune(_))
    ));

    let tag_a = format!("--tag=restichub-profile-{profile_a}");
    let tag_b = format!("--tag=restichub-profile-{profile_b}");
    let mut seen_a = false;
    let mut seen_b = false;
    for forget in forgets {
        let args = argv(&job, forget);
        if args.contains(&tag_a) {
            seen_a = true;
            assert!(args.contains(&"--keep-last=3".to_owned()), "{args:?}");
            assert!(!args.contains(&tag_b), "{args:?}");
        } else {
            seen_b = true;
            assert!(args.contains(&tag_b), "{args:?}");
            assert!(args.contains(&"--keep-daily=7".to_owned()), "{args:?}");
        }
        assert!(
            !args
                .iter()
                .any(|arg| arg.starts_with("--host") || arg.starts_with("--path")),
            "the profile tag is the exact key: {args:?}"
        );
        assert!(
            args.iter().any(|arg| arg.starts_with("--group-by=")),
            "{args:?}"
        );
    }
    assert!(seen_a && seen_b, "each profile has its own forget step");
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_pf_01_a_backup_carries_the_profile_tag() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("tag-host").await;
    let repository = env.create_repository(agent.id, "pw").await;
    let profile = env
        .create_profile(repository, json!({"tags": ["daily"]}))
        .await;
    let plan = env.create_plan(profile, agent.id).await;
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
    assert_eq!(
        backup.tags,
        ["daily", &format!("restichub-profile-{profile}")]
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_rp_04_only_the_maintenance_agent_gets_forget_prune_and_check() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let (maintenance, mut backup) = two_agents(&env).await;
    let repository = initialized_repository(&env, maintenance.id).await;
    let profile = env
        .create_profile(repository, json!({"retention": {"keep_last": 1}}))
        .await;
    env.create_plan(profile, backup.id).await;

    for kind in [JobKind::Forget, JobKind::Prune, JobKind::Check] {
        let job = env.enqueue(kind, backup.id, repository, None).await;
        env.pass().await;
        eventually(|| async { env.job_status(job).await == "failed" }).await;
        let ended = env.job(job).await;
        assert_eq!(ended["result_message"], "not_maintenance_agent", "{ended}");
    }
    assert!(
        backup.try_job(Duration::from_millis(300)).await.is_none(),
        "the backup host receives no maintenance job"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_rp_08_a_repository_without_a_maintenance_agent_runs_no_maintenance() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let body = json!({
        "name": "no-agent",
        "backend": {"kind": "rest", "endpoint": "https://rest.example.net/none"},
        "password": "pw",
    });
    let (status, repository) = env
        .call(&env.admin, "POST", "/api/v1/repositories", Some(body))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{repository}");
    let repository = queue_support::id_of(&repository);

    let (status, body) = run(&env, &env.operator, repository, "check").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (_, before) = env
        .call(&env.admin, "GET", &maintenance_uri(repository), None)
        .await;
    assert!(before["maintenance_agent_id"].is_null(), "{before}");
    let first = before["retention"]["next_run_at"].clone();
    env.clock.advance(60 * 24 * 3600);
    restichub_cp::scheduler::run_once(env.db.database(), env.clock.as_ref())
        .await
        .expect("the scheduler pass");
    assert_eq!(retention_job_count(&env, repository).await, 0);
    let (_, after) = env
        .call(&env.admin, "GET", &maintenance_uri(repository), None)
        .await;
    assert_ne!(
        after["retention"]["next_run_at"], first,
        "the schedule advances"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_sc_02_two_instances_create_one_maintenance_job_for_a_fire_time() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("sched-host").await;
    let repository = initialized_repository(&env, agent.id).await;
    let profile = env
        .create_profile(repository, json!({"retention": {"keep_last": 2}}))
        .await;
    env.create_plan(profile, agent.id).await;
    let (status, view) = env
        .call(
            &env.admin,
            "PUT",
            &maintenance_uri(repository),
            Some(json!({
                "time_zone": "UTC",
                "retention": {"cron": "0 3 * * 0", "enabled": true},
                "check": {"cron": "0 4 1 * *", "enabled": false},
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    let next = time::OffsetDateTime::parse(
        view["retention"]["next_run_at"].as_str().expect("next run"),
        &time::format_description::well_known::Rfc3339,
    )
    .expect("RFC 3339");
    env.clock.advance((next - env.now()).whole_seconds() + 60);

    let first = restichub_cp::scheduler::run_once(env.db.database(), env.clock.as_ref());
    let second = restichub_cp::scheduler::run_once(env.db.database(), env.clock.as_ref());
    tokio::try_join!(first, second).expect("the two passes");
    assert_eq!(retention_job_count(&env, repository).await, 1);

    restichub_cp::scheduler::run_once(env.db.database(), env.clock.as_ref())
        .await
        .expect("a further pass");
    assert_eq!(retention_job_count(&env, repository).await, 1);
    env.db.cleanup().await;
}

#[tokio::test]
async fn fr_sc_04_a_repository_with_an_exclusive_job_waits_for_the_schedule() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("wait-host").await;
    let repository = initialized_repository(&env, agent.id).await;
    let profile = env
        .create_profile(repository, json!({"retention": {"keep_last": 2}}))
        .await;
    env.create_plan(profile, agent.id).await;
    let (status, queued) = run(&env, &env.operator, repository, "check").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
    let (status, again) = run(&env, &env.operator, repository, "retention").await;
    assert_eq!(status, StatusCode::CONFLICT, "{again}");

    let (_, view) = env
        .call(&env.admin, "GET", &maintenance_uri(repository), None)
        .await;
    let next = time::OffsetDateTime::parse(
        view["retention"]["next_run_at"].as_str().expect("next run"),
        &time::format_description::well_known::Rfc3339,
    )
    .expect("RFC 3339");
    env.clock.advance((next - env.now()).whole_seconds() + 60);
    restichub_cp::scheduler::run_once(env.db.database(), env.clock.as_ref())
        .await
        .expect("the scheduler pass");
    assert_eq!(
        retention_job_count(&env, repository).await,
        1,
        "the schedule waits behind the queued check"
    );
    env.db.cleanup().await;
}

#[tokio::test]
async fn ac_au_02_the_maintenance_role_rules() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("role-host").await;
    let repository = initialized_repository(&env, agent.id).await;
    let update = json!({
        "time_zone": "Europe/Berlin",
        "retention": {"cron": "0 2 * * 6", "enabled": true},
        "check": {"cron": "0 5 2 * *", "enabled": true},
    });

    for token in [&env.viewer, &env.operator] {
        let (status, _) = env
            .call(
                token,
                "PUT",
                &maintenance_uri(repository),
                Some(update.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let (status, view) = env
        .call(
            &env.admin,
            "PUT",
            &maintenance_uri(repository),
            Some(update),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["retention"]["time_zone"], "Europe/Berlin");
    let (status, _) = env
        .call(&env.viewer, "GET", &maintenance_uri(repository), None)
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = run(&env, &env.viewer, repository, "check").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, job) = run(&env, &env.operator, repository, "check").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    assert_eq!(job["kind"], "check");
    assert_eq!(job["agent_id"], agent.id.to_string().as_str());

    let bad = json!({
        "time_zone": "Mars/Base",
        "retention": {"cron": "0 2 * * 6", "enabled": true},
        "check": {"cron": "0 5 2 * *", "enabled": true},
    });
    let (status, body) = env
        .call(&env.admin, "PUT", &maintenance_uri(repository), Some(bad))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    env.db.cleanup().await;
}
