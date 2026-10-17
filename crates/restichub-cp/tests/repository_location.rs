//! A tenant holds one repository for each location (FR-RP-11, AC-RP-10, AC-RP-11).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use queue_support::{Env, id_of, test_queue};

fn s3(name: &str, endpoint: &str, bucket: &str, prefix: Option<&str>) -> Value {
    let mut backend = json!({"kind": "s3", "endpoint": endpoint, "bucket": bucket});
    if let Some(prefix) = prefix {
        backend["prefix"] = json!(prefix);
    }
    json!({
        "name": name,
        "backend": backend,
        "password": "pw",
        "s3_access_key": "key",
        "s3_secret_key": "secret",
    })
}

fn rest(name: &str, endpoint: &str) -> Value {
    json!({
        "name": name,
        "backend": {"kind": "rest", "endpoint": endpoint},
        "password": "pw",
    })
}

fn local(name: &str, path: &str, host: &str) -> Value {
    json!({
        "name": name,
        "backend": {"kind": "local", "path": path},
        "password": "pw",
        "maintenance_agent_id": host,
    })
}

async fn create(env: &Env, body: Value) -> (StatusCode, Value) {
    env.call(&env.admin, "POST", "/api/v1/repositories", Some(body))
        .await
}

async fn created(env: &Env, body: Value) -> Uuid {
    let (status, body) = create(env, body).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    id_of(&body)
}

fn assert_taken(response: (StatusCode, Value), field: &str, existing: &str) {
    let (status, body) = response;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "repository_location_taken", "{body}");
    assert_eq!(body["field"], field, "{body}");
    assert_eq!(body["existing_name"], existing, "{body}");
}

#[tokio::test]
async fn ac_rp_10_an_s3_location_is_taken_in_every_spelling() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    created(
        &env,
        s3("s3-first", "https://s3.example.net", "data", Some("main")),
    )
    .await;

    for endpoint in [
        "https://s3.example.net",
        "https://S3.Example.NET",
        "https://s3.example.net:443",
    ] {
        assert_taken(
            create(&env, s3("s3-copy", endpoint, "data", Some("main"))).await,
            "backend.bucket",
            "s3-first",
        );
    }

    for (endpoint, bucket, prefix) in [
        ("https://s3.example.net", "data", Some("other")),
        ("https://s3.example.net", "data", None),
        ("https://s3.example.net", "other", Some("main")),
        ("https://s3.example.net:9000", "data", Some("main")),
        ("https://s3.other.net", "data", Some("main")),
    ] {
        created(
            &env,
            s3(&format!("s3-{}", Uuid::now_v7()), endpoint, bucket, prefix),
        )
        .await;
    }
}

#[tokio::test]
async fn ac_rp_10_a_rest_location_is_taken_in_every_spelling() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    created(&env, rest("rest-first", "https://rest.example.net/main")).await;

    for endpoint in [
        "https://rest.example.net/main",
        "https://REST.example.net/main/",
        "http://rest.example.net/main",
    ] {
        let response = create(&env, rest("rest-copy", endpoint)).await;
        if endpoint.starts_with("http:") {
            assert_eq!(response.0, StatusCode::CREATED, "{}", response.1);
        } else {
            assert_taken(response, "backend.endpoint", "rest-first");
        }
    }
    created(&env, rest("rest-other", "https://rest.example.net/main2")).await;
}

#[tokio::test]
async fn ac_rp_10_a_local_location_is_the_host_and_the_path() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let host = env.enroll_agent("loc-host").await.agent_id;
    let other = env.enroll_agent("loc-other").await.agent_id;
    created(&env, local("local-first", "/srv/backup/main", &host)).await;

    for path in ["/srv/backup/main", "/srv/./backup/main"] {
        assert_taken(
            create(&env, local("local-copy", path, &host)).await,
            "backend.path",
            "local-first",
        );
    }
    created(&env, local("local-other-path", "/srv/backup/other", &host)).await;
    created(&env, local("local-other-host", "/srv/backup/main", &other)).await;
}

#[tokio::test]
async fn ac_rp_10_an_update_to_a_taken_location_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    created(&env, rest("update-first", "https://rest.example.net/a")).await;
    let second = created(&env, rest("update-second", "https://rest.example.net/b")).await;
    let path = format!("/api/v1/repositories/{second}");

    assert_taken(
        env.call(
            &env.admin,
            "PATCH",
            &path,
            Some(json!({"backend": {"kind": "rest", "endpoint": "https://REST.example.net/a/"}})),
        )
        .await,
        "backend.endpoint",
        "update-first",
    );

    let (status, body) = env
        .call(
            &env.admin,
            "PATCH",
            &path,
            Some(json!({"backend": {"kind": "rest", "endpoint": "https://rest.example.net/c"}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = env
        .call(&env.admin, "PATCH", &path, Some(json!({"name": "renamed"})))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn ac_rp_11_the_list_marks_a_duplicate_and_the_start_check_changes_no_row() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let first = created(&env, rest("dup-first", "https://rest.example.net/x")).await;
    let second = created(&env, rest("dup-second", "https://rest.example.net/y")).await;
    created(&env, rest("dup-alone", "https://rest.example.net/z")).await;
    // A row from before the rule: the API refuses to build it, so the test writes it.
    let mut tx = env.scoped().await;
    sqlx::query("UPDATE repositories SET rest_endpoint = $2 WHERE id = $1")
        .bind(second)
        .bind("https://rest.example.net/x")
        .execute(&mut *tx)
        .await
        .expect("cannot write the duplicate");
    tx.commit().await.expect("cannot commit the duplicate");

    restichub_cp::startup::report_shared_locations(env.db.database())
        .await
        .expect("the check runs");

    let (status, body) = env
        .call(&env.admin, "GET", "/api/v1/repositories", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().expect("the items");
    assert_eq!(items.len(), 3, "{body}");
    for item in items {
        let expected = match item["name"].as_str().expect("a name") {
            "dup-first" => json!(["dup-second"]),
            "dup-second" => json!(["dup-first"]),
            _ => json!([]),
        };
        assert_eq!(item["shares_location_with"], expected, "{item}");
    }
    assert_eq!(
        id_of(
            &items
                .iter()
                .find(|item| item["name"] == "dup-first")
                .expect("first")
                .clone()
        ),
        first
    );

    // A duplicate that exists already can be renamed: only a location change is checked.
    let (status, body) = env
        .call(
            &env.admin,
            "PATCH",
            &format!("/api/v1/repositories/{second}"),
            Some(json!({"name": "dup-renamed"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}
