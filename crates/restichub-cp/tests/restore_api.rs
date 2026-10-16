//! The snapshot list, the tree pages, the roles and the refusals of the Restore API (FR-RS-01,
//! FR-RS-02, FR-RS-04, FR-PR-09, AC-RS-01, AC-RS-02, AC-TN-01).
//!
//! The restic double is the `fake-restic` example of the agent crate, behind the real job runner
//! of the agent. The example prints the recorded `restic snapshots --json` output of the agent
//! fixtures and generates the listing of a directory.

#![allow(clippy::expect_used)]

mod common;
mod queue_support;
mod restore_support;

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use restichub_db::JobKind;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use queue_support::{Env, decode_job_id, eventually, test_queue};
use restore_support::{
    RealAgent, fake_restic, finished_job, initialized_repository, ls_uri, restore_body,
    restore_uri, snapshots_uri, work_dir,
};

const SNAPSHOTS: &str =
    include_str!("../../restichub-agent/tests/fixtures/restic-0.18.1/snapshots.json");

fn time_of(value: &Value) -> OffsetDateTime {
    OffsetDateTime::parse(value.as_str().expect("a time"), &Rfc3339).expect("an RFC 3339 time")
}

/// The scripted repository of a test: a path with the control files of the fake next to it.
fn scripted_location(name: &str) -> std::path::PathBuf {
    let dir = work_dir(name);
    std::fs::write(dir.join("repo.restichub-fake"), "0 0").expect("cannot write the script");
    dir.join("repo")
}

async fn read(env: &Env, token: &str, uri: &str) -> (StatusCode, Value) {
    env.call(token, "GET", uri, None).await
}

/// AC-RS-01: the list of the control plane agrees with `restic snapshots --json`, and the filters
/// reach restic as options.
#[tokio::test]
async fn ac_rs_01_the_snapshot_list_agrees_with_restic_snapshots_json() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("rs01-host").await;
    let location = scripted_location("rs01");
    std::fs::write(
        format!("{}.restichub-out-snapshots", location.display()),
        SNAPSHOTS,
    )
    .expect("cannot script the snapshots");
    let repository = initialized_repository(&env, agent.id, "pw", &location).await;
    let mut real = RealAgent::new("rs01-agent", &fake_restic());

    let (status, started) = env
        .call(
            &env.operator,
            "POST",
            &snapshots_uri(repository),
            Some(json!({"hosts": ["m00nd3v"], "tags": ["daily"], "paths": ["/srv"]})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    assert_eq!(started["state"], "pending");
    let job = started["job_id"].as_str().expect("a job id").to_owned();
    env.pass().await;
    assert_eq!(real.run_next(&mut agent).await, job);

    let uri = format!("{}?job={job}", snapshots_uri(repository));
    eventually(|| async { read(&env, &env.operator, &uri).await.1["state"] == "done" }).await;
    let (status, view) = read(&env, &env.operator, &uri).await;
    assert_eq!(status, StatusCode::OK, "{view}");

    let expected: Vec<Value> = serde_json::from_str(SNAPSHOTS).expect("the fixture is JSON");
    let listed = view["snapshots"].as_array().expect("snapshots");
    assert_eq!(listed.len(), expected.len());
    for (item, restic) in listed.iter().zip(&expected) {
        assert_eq!(item["id"], restic["id"]);
        assert_eq!(item["short_id"], restic["short_id"]);
        assert_eq!(item["hostname"], restic["hostname"]);
        assert_eq!(item["username"], restic["username"]);
        assert_eq!(item["paths"], restic["paths"]);
        assert_eq!(item["program_version"], restic["program_version"]);
        assert_eq!(
            item["tags"].as_array().cloned().unwrap_or_default(),
            restic["tags"].as_array().cloned().unwrap_or_default()
        );
        assert_eq!(time_of(&item["time"]), time_of(&restic["time"]));
    }
    assert_eq!(view["hosts"], json!(["m00nd3v"]));
    assert_eq!(view["tags"], json!(["daily"]));
    assert_eq!(view["truncated"], false);

    let argv = std::fs::read_to_string(format!("{}.restichub-argv", location.display()))
        .expect("the recorded arguments");
    assert_eq!(
        argv.trim(),
        "snapshots --json --host=m00nd3v --tag=daily --path=/srv"
    );

    // The date window is applied when the list is served.
    let times: Vec<OffsetDateTime> = expected.iter().map(|item| time_of(&item["time"])).collect();
    let newest = times.iter().max().expect("a snapshot");
    let from = (*newest - time::Duration::seconds(1))
        .format(&Rfc3339)
        .expect("a time");
    let (_, recent) = read(
        &env,
        &env.operator,
        &format!("{uri}&from={}", from.replace('+', "%2B")),
    )
    .await;
    let recent = recent["snapshots"].as_array().expect("snapshots").len();
    assert!(recent >= 1 && recent < expected.len(), "{recent}");

    // Without a job the call returns the stored list with its age.
    env.clock.advance(90);
    let (_, cached) = read(&env, &env.viewer, &snapshots_uri(repository)).await;
    assert_eq!(cached["state"], "done");
    assert_eq!(cached["job_id"], job.as_str());
    assert!(cached["age_seconds"].as_i64().expect("an age") >= 90);
    assert_eq!(
        cached["snapshots"].as_array().expect("snapshots").len(),
        expected.len()
    );
    env.db.cleanup().await;
}

/// AC-RS-02: the first page comes in 5 s or less, the next page loads on request, and a repeated
/// request comes from the stored copy.
#[tokio::test]
async fn ac_rs_02_a_page_comes_fast_and_a_repeat_starts_no_job() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("rs02-host").await;
    let location = scripted_location("rs02");
    std::fs::write(format!("{}.restichub-ls-count", location.display()), "3000")
        .expect("cannot script the count");
    let repository = initialized_repository(&env, agent.id, "pw", &location).await;
    let mut real = RealAgent::new("rs02-agent", &fake_restic());
    let page = |offset: u64| json!({"snapshot_id": "0123abcd", "path": "/data", "offset": offset, "limit": 1000});

    let started = Instant::now();
    let (status, pending) = env
        .call(&env.operator, "POST", &ls_uri(repository), Some(page(0)))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{pending}");
    let job = pending["job_id"].as_str().expect("a job id").to_owned();
    env.pass().await;
    assert_eq!(real.run_next(&mut agent).await, job);
    let uri = format!("{}?job={job}", ls_uri(repository));
    eventually(|| async { read(&env, &env.operator, &uri).await.1["state"] == "done" }).await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let (_, first) = read(&env, &env.operator, &uri).await;
    let nodes = first["nodes"].as_array().expect("nodes");
    assert_eq!(nodes.len(), 1000);
    assert_eq!(nodes[0]["name"], "file-0000000");
    assert_eq!(nodes[0]["kind"], "file");
    assert_eq!(first["last"], false);
    assert_eq!(first["next_offset"], 1000);
    assert_eq!(first["cached"], false);

    // The same request again comes from the stored copy.
    let (status, again) = env
        .call(&env.operator, "POST", &ls_uri(repository), Some(page(0)))
        .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["cached"], true);
    assert_eq!(again["nodes"].as_array().expect("nodes").len(), 1000);
    assert!(
        agent.try_job(Duration::from_millis(400)).await.is_none(),
        "no job runs"
    );

    // The next offset is a page that no job listed before, so a job runs.
    let (status, next) = env
        .call(&env.operator, "POST", &ls_uri(repository), Some(page(1000)))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{next}");
    env.pass().await;
    real.run_next(&mut agent).await;
    let next_uri = format!(
        "{}?job={}",
        ls_uri(repository),
        next["job_id"].as_str().expect("a job")
    );
    eventually(|| async { read(&env, &env.operator, &next_uri).await.1["state"] == "done" }).await;
    let (_, second) = read(&env, &env.operator, &next_uri).await;
    assert_eq!(second["nodes"][0]["name"], "file-0001000");
    assert_eq!(second["next_offset"], 2000);
    env.db.cleanup().await;
}

/// The browse jobs stay out of the job list. A restore job shows there.
#[tokio::test]
async fn fr_rs_01_the_job_list_hides_the_browse_jobs() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env.connect_agent("hide-host").await;
    let location = scripted_location("hide");
    let repository = initialized_repository(&env, agent.id, "pw", &location).await;
    let mut real = RealAgent::new("hide-agent", &fake_restic());

    let (status, _) = env
        .call(
            &env.operator,
            "POST",
            &snapshots_uri(repository),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    env.pass().await;
    let browse = real.run_next(&mut agent).await;
    let target = work_dir("hide-target");
    let (status, restore) = env
        .call(
            &env.operator,
            "POST",
            &restore_uri(repository),
            Some(restore_body("0123abcd", &target, json!({}))),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{restore}");
    assert_eq!(restore["kind"], "restore");
    env.pass().await;
    real.run_next(&mut agent).await;

    let (_, list) = env.call(&env.admin, "GET", "/api/v1/jobs", None).await;
    let kinds: Vec<&str> = list["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|job| job["kind"].as_str().expect("a kind"))
        .collect();
    assert_eq!(kinds, ["restore"], "{list}");
    // A browse job is still readable by its identifier.
    let (status, _) = env
        .call(&env.admin, "GET", &format!("/api/v1/jobs/{browse}"), None)
        .await;
    assert_eq!(status, StatusCode::OK);

    // The detail of a restore job carries the parameters of the request; a browse job has none.
    let restore_id = restore["id"].as_str().expect("a job id");
    let (_, detail) = env
        .call(&env.admin, "GET", &format!("/api/v1/jobs/{restore_id}"), None)
        .await;
    assert_eq!(detail["restore"]["snapshot_id"], "0123abcd", "{detail}");
    assert_eq!(detail["restore"]["target"], target.to_str().expect("UTF-8"));
    assert_eq!(detail["restore"]["overwrite"], "always");
    assert_eq!(detail["restore"]["includes"], json!([]));
    assert_eq!(detail["restore"]["dry_run"], false);
    let (_, detail) = env
        .call(&env.admin, "GET", &format!("/api/v1/jobs/{browse}"), None)
        .await;
    assert!(detail["restore"].is_null(), "{detail}");
    env.db.cleanup().await;
}

/// FR-PR-09: an agent without `restore-v1` gets no `ls` and no `restore` job, and a snapshot list
/// still works.
#[tokio::test]
async fn fr_pr_09_an_agent_without_restore_v1_gets_no_ls_and_no_restore() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let mut agent = env
        .connect_agent_with("old-host", vec!["forget-all-tags".to_owned()])
        .await;
    let location = scripted_location("old");
    let repository = initialized_repository(&env, agent.id, "pw", &location).await;
    let target = work_dir("old-target");

    let (status, body) = env
        .call(
            &env.operator,
            "POST",
            &restore_uri(repository),
            Some(restore_body("0123abcd", &target, json!({}))),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "agent_capability_missing");
    let (status, body) = env
        .call(
            &env.operator,
            "POST",
            &ls_uri(repository),
            Some(json!({"snapshot_id": "0123abcd"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "agent_capability_missing");
    assert!(agent.try_job(Duration::from_millis(300)).await.is_none());
    let (status, body) = env
        .call(
            &env.operator,
            "POST",
            &snapshots_uri(repository),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");

    // A restore row that the API did not check, for an agent that lost the capability since: the
    // builder refuses it before any secret leaves the control plane.
    let job = env
        .enqueue(JobKind::Restore, agent.id, repository, None)
        .await;
    {
        use restichub_db::restore::{JobRequest, OverwriteMode, RestoreRequest};
        let mut tx = env
            .db
            .database()
            .begin_tenant(env.tenant)
            .await
            .expect("a transaction");
        restichub_db::restore::insert_request(
            &mut tx,
            restichub_db::JobId::from_uuid(job),
            &JobRequest::Restore(RestoreRequest {
                snapshot_id: "0123abcd".to_owned(),
                includes: Vec::new(),
                excludes: Vec::new(),
                target: target.to_string_lossy().into_owned(),
                overwrite: OverwriteMode::Always,
                dry_run: false,
                verify: false,
            }),
        )
        .await
        .expect("cannot store the request");
        tx.commit().await.expect("cannot commit");
    }
    env.pass().await;
    let ended = finished_job(&env, &job.to_string()).await;
    assert_eq!(ended["status"], "failed", "{ended}");
    assert_eq!(ended["result_message"], "agent_capability_missing");
    let delivered = restore_support::audit_rows(&env, "secret_delivered").await;
    assert!(
        delivered.iter().all(|row| !row.contains(&job.to_string())),
        "the refused restore delivered no secret"
    );
    env.db.cleanup().await;
}

/// The API refuses an offline agent, a bad field, both pattern lists and unknown fields.
#[tokio::test]
async fn fr_rs_04_the_restore_api_refuses_a_bad_request() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("valid-host").await;
    let location = scripted_location("valid");
    let repository = initialized_repository(&env, agent.id, "pw", &location).await;
    let target = work_dir("valid-target");
    let post = |body: Value| {
        let env = &env;
        async move {
            env.call(&env.operator, "POST", &restore_uri(repository), Some(body))
                .await
        }
    };

    for (body, field) in [
        (
            restore_body("0123abcd", std::path::Path::new("relative/dir"), json!({})),
            "target",
        ),
        (restore_body("--dry-run", &target, json!({})), "snapshot_id"),
        (restore_body("ABCDEF12", &target, json!({})), "snapshot_id"),
        (
            restore_body(
                "0123abcd",
                &target,
                json!({"includes": ["/a"], "excludes": ["*.log"]}),
            ),
            "excludes",
        ),
        (
            restore_body("0123abcd", &target, json!({"includes": [""]})),
            "includes",
        ),
    ] {
        let (status, error) = post(body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {error}");
        assert_eq!(error["field"], field, "{body}");
    }
    let (status, _) = post(restore_body(
        "0123abcd",
        &target,
        json!({"overwrite": "sometimes"}),
    ))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = post(restore_body("0123abcd", &target, json!({"shell": "x"}))).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "unknown fields are refused"
    );
    let (status, error) = env
        .call(
            &env.operator,
            "POST",
            &ls_uri(repository),
            Some(json!({"snapshot_id": "0123abcd", "limit": 5001})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");

    // An enrolled agent without a stream is offline for the control plane.
    let sleeper = env.enroll_agent("never-connected").await;
    let sleeper_id =
        restichub_db::AgentId::from_uuid(Uuid::parse_str(&sleeper.agent_id).expect("an agent id"));
    let offline = initialized_repository(&env, sleeper_id, "pw", &scripted_location("off")).await;
    let (status, error) = env
        .call(
            &env.operator,
            "POST",
            &restore_uri(offline),
            Some(restore_body("0123abcd", &target, json!({}))),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert_eq!(error["code"], "agent_offline");

    // A repository that is not initialized gives HTTP 409.
    let fresh = env
        .create_repository_at(agent.id, "pw", location.to_str().expect("a path"))
        .await;
    let (status, _) = env
        .call(
            &env.operator,
            "POST",
            &restore_uri(fresh),
            Some(restore_body("0123abcd", &target, json!({}))),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // A local repository accepts its maintenance agent only.
    let other = env.connect_agent("other-host").await;
    let (status, error) = env
        .call(
            &env.operator,
            "POST",
            &restore_uri(repository),
            Some(restore_body(
                "0123abcd",
                &target,
                json!({"agent_id": other.id.to_string()}),
            )),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    assert_eq!(error["field"], "agent_id");
    env.db.cleanup().await;
}

/// The roles (requirements, section 2): a Viewer reads the snapshot list and starts nothing.
#[tokio::test]
async fn ac_au_02_the_roles_of_the_restore_endpoints() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("roles-host").await;
    let location = scripted_location("roles");
    let repository = initialized_repository(&env, agent.id, "pw", &location).await;
    let target = work_dir("roles-target");
    let restore = Some(restore_body("0123abcd", &target, json!({"dry_run": true})));
    let ls = Some(json!({"snapshot_id": "0123abcd"}));

    for token in [&env.viewer] {
        for (method, uri, body) in [
            ("POST", snapshots_uri(repository), Some(json!({}))),
            ("POST", ls_uri(repository), ls.clone()),
            ("POST", restore_uri(repository), restore.clone()),
            (
                "GET",
                format!("{}?job={}", ls_uri(repository), Uuid::now_v7()),
                None,
            ),
        ] {
            let (status, body) = env.call(token, method, &uri, body).await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "viewer {method} {uri}: {body}"
            );
        }
    }
    let (status, _) = env
        .call(&env.viewer, "GET", &snapshots_uri(repository), None)
        .await;
    assert_eq!(status, StatusCode::OK, "a Viewer reads the stored list");
    let (status, _) = env
        .call(
            "bad-token",
            "POST",
            &restore_uri(repository),
            restore.clone(),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = env
        .call(&env.operator, "POST", &restore_uri(repository), restore)
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let (status, _) = env.call(&env.admin, "POST", &ls_uri(repository), ls).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let _ = decode_job_id;
    env.db.cleanup().await;
}
