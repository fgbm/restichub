//! The safety rules of the Restore API: restore roots, concrete snapshot identifiers, the pairs
//! that restic refuses, and the limits of the browse jobs (FR-RS-03, FR-RS-04, ADR-0035).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;
mod restore_support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use queue_support::{Env, test_queue};
use restore_support::{
    audit_rows, initialized_repository, ls_uri, restore_body, restore_uri, snapshots_uri, work_dir,
};

async fn post(env: &Env, uri: &str, body: Value) -> (StatusCode, Value) {
    env.call(&env.operator, "POST", uri, Some(body)).await
}

fn page(path: &str) -> Value {
    json!({"snapshot_id": "0123abcd", "path": path, "offset": 0, "limit": 100})
}

/// FR-RS-03: the control plane refuses a target outside the roots that the agent reported, and a
/// target that is not a path of the agent operating system.
#[tokio::test]
async fn ac_rs_05_a_target_outside_the_agent_roots_is_refused_before_a_job_exists() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env
        .connect_agent_with_roots("roots-host", vec!["/srv/restore".to_owned()])
        .await;
    let repository =
        initialized_repository(&env, agent.id, "pw", &work_dir("roots").join("repo")).await;
    let uri = restore_uri(repository);
    let target = |path: &str| restore_body("0123abcd", std::path::Path::new(path), json!({}));

    for denied in ["/etc", "/", "/srv", "/srv/restore2", "/srv/restore/../etc"] {
        let (status, body) = post(&env, &uri, target(denied)).await;
        assert!(
            status == StatusCode::CONFLICT && body["code"] == "restore_target_denied"
                || status == StatusCode::BAD_REQUEST,
            "{denied}: {status} {body}"
        );
    }
    let (status, body) = post(&env, &uri, target("/etc")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "restore_target_denied");
    // A Windows path means nothing to a Linux agent.
    let (status, body) = post(&env, &uri, target("C:\\restore")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "target");

    let (status, body) = post(&env, &uri, target("/srv/restore/job-1")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let (status, _) = post(&env, &uri, target("/srv/restore")).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    env.db.cleanup().await;
}

/// FR-RS-03: the list of agents shows the restore roots that the last hello reported.
#[tokio::test]
async fn ac_rs_05_the_agent_list_shows_the_restore_roots() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env
        .connect_agent_with_roots(
            "roots-list-host",
            vec!["/var/lib/restichub-restore".to_owned(), "/srv".to_owned()],
        )
        .await;
    let (status, body) = env.call(&env.operator, "GET", "/api/v1/agents", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["id"] == agent.id.to_string())
        .expect("the agent is listed");
    assert_eq!(
        row["restore_roots"],
        json!(["/var/lib/restichub-restore", "/srv"])
    );
    env.db.cleanup().await;
}

/// FR-RS-03, FR-RS-04: an unsafe request is a 400 before any job exists.
#[tokio::test]
async fn fr_rs_04_dry_run_with_verify_and_a_moving_snapshot_are_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("unsafe-host").await;
    let repository =
        initialized_repository(&env, agent.id, "pw", &work_dir("unsafe").join("repo")).await;
    let target = work_dir("unsafe-target");

    let (status, body) = post(
        &env,
        &restore_uri(repository),
        restore_body(
            "0123abcd",
            &target,
            json!({"dry_run": true, "verify": true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "verify");
    let (status, body) = post(
        &env,
        &restore_uri(repository),
        restore_body("latest", &target, json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "snapshot_id");
    let (status, body) = post(&env, &ls_uri(repository), json!({"snapshot_id": "latest"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "snapshot_id");
    let (status, body) = post(
        &env,
        &restore_uri(repository),
        restore_body("0123abcd", &target, json!({"overwrite": "default"})),
    )
    .await;
    assert!(status.is_client_error(), "{status} {body}");
    env.db.cleanup().await;
}

/// ADR-0035: an identical request that still waits gets the job that waits, and a request for
/// another page gets a new job.
#[tokio::test]
async fn adr_0035_an_identical_pending_browse_request_reuses_the_job() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("reuse-host").await;
    let repository =
        initialized_repository(&env, agent.id, "pw", &work_dir("reuse").join("repo")).await;

    let (status, first) = post(&env, &ls_uri(repository), page("/data")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{first}");
    let (status, second) = post(&env, &ls_uri(repository), page("/data")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{second}");
    assert_eq!(first["job_id"], second["job_id"], "the job is reused");
    let (_, other) = post(&env, &ls_uri(repository), page("/other")).await;
    assert_ne!(first["job_id"], other["job_id"]);

    let (_, a) = post(&env, &snapshots_uri(repository), json!({"hosts": ["a"]})).await;
    let (_, b) = post(&env, &snapshots_uri(repository), json!({"hosts": ["a"]})).await;
    let (_, c) = post(&env, &snapshots_uri(repository), json!({"hosts": ["b"]})).await;
    assert_eq!(a["job_id"], b["job_id"]);
    assert_ne!(a["job_id"], c["job_id"]);

    let mut tx = env.scoped().await;
    let jobs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind IN ('ls', 'snapshots')")
            .fetch_one(&mut *tx)
            .await
            .expect("cannot count the jobs");
    tx.commit().await.expect("cannot end the read");
    assert_eq!(jobs, 4, "four requests differ");
    env.db.cleanup().await;
}

/// ADR-0035: one actor has a request limit, and one agent holds only a few waiting browse jobs.
#[tokio::test]
async fn adr_0035_browse_requests_have_an_actor_limit_and_an_agent_cap() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start_with_browse_rate(db, test_queue(), Some(12)).await;
    let agent = env.connect_agent("limit-host").await;
    let repository =
        initialized_repository(&env, agent.id, "pw", &work_dir("limit").join("repo")).await;

    // The agent holds eight waiting browse jobs; the ninth distinct request waits for room.
    for index in 0..8 {
        let (status, body) = post(&env, &ls_uri(repository), page(&format!("/d{index}"))).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{index}: {body}");
    }
    let (status, body) = post(&env, &ls_uri(repository), page("/d8")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["code"], "rate_limited");
    // A request for a job that waits already costs nothing more on the agent.
    let (status, _) = post(&env, &ls_uri(repository), page("/d0")).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // The actor bucket (12 requests in a minute) is now spent.
    let (status, body) = post(&env, &ls_uri(repository), page("/d9")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    let (status, _) = env
        .call(&env.admin, "POST", &ls_uri(repository), Some(page("/d0")))
        .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "another actor has its own bucket"
    );
    env.db.cleanup().await;
}

/// ADR-0035: parallel requests cannot pass the cap of waiting browse jobs of one agent.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adr_0035_parallel_browse_requests_respect_the_agent_cap() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = std::sync::Arc::new(Env::start_with_browse_rate(db, test_queue(), Some(1000)).await);
    let agent = env.connect_agent("parallel-host").await;
    let repository =
        initialized_repository(&env, agent.id, "pw", &work_dir("parallel").join("repo")).await;
    let second =
        initialized_repository(&env, agent.id, "pw", &work_dir("parallel").join("repo2")).await;

    let mut set = tokio::task::JoinSet::new();
    for index in 0..16 {
        let env = std::sync::Arc::clone(&env);
        set.spawn(async move {
            // Two repositories of one agent: the lock of the repository row alone cannot serialize them.
            let repository = if index % 2 == 0 { repository } else { second };
            post(&env, &ls_uri(repository), page(&format!("/p{index}")))
                .await
                .0
        });
    }
    let mut accepted = 0;
    while let Some(status) = set.join_next().await {
        if status.expect("the request task must finish") == StatusCode::ACCEPTED {
            accepted += 1;
        }
    }
    assert_eq!(accepted, 8, "the cap is 8 waiting browse jobs");

    let mut tx = env.scoped().await;
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind = 'ls'")
        .fetch_one(&mut *tx)
        .await
        .expect("cannot count the jobs");
    tx.commit().await.expect("cannot end the read");
    assert!(jobs <= 8, "{jobs} browse jobs wait");
    env.db.cleanup().await;
}

/// ADR-0035: a snapshot list or a page that starts a job writes `browse_start` with the agent that
/// gets the repository keys.
#[tokio::test]
async fn adr_0035_a_browse_job_writes_an_audit_record_with_the_target_agent() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent("audit-host").await;
    let repository =
        initialized_repository(&env, agent.id, "pw", &work_dir("audit").join("repo")).await;

    let (status, _) = post(&env, &snapshots_uri(repository), json!({})).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, _) = post(&env, &ls_uri(repository), page("/")).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    // The reused job leaves two records, not three.
    let (status, _) = post(&env, &ls_uri(repository), page("/")).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let rows = audit_rows(&env, "browse_start").await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    for row in &rows {
        assert!(row.contains(&agent.id.to_string()), "{row}");
        assert!(row.contains(&repository.to_string()), "{row}");
    }
    assert!(Uuid::parse_str(&repository.to_string()).is_ok());
    env.db.cleanup().await;
}
