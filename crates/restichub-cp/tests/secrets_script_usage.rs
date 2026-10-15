//! The usage of a hook secret by the scripts of profiles and job templates (FR-RP-05, FR-HK-09).

#![allow(clippy::expect_used)]

mod common;
mod queue_support;

use axum::http::StatusCode;
use serde_json::{Value, json};

use queue_support::{Env, test_queue};

fn script(body: &str, env: Value) -> Value {
    json!({
        "name": "dump",
        "interpreter": "bash",
        "phase": "before",
        "body": body,
        "timeout_seconds": 60,
        "failure_policy": "abort_backup",
        "env": env,
    })
}

async fn create_secret(env: &Env, name: &str) -> String {
    let (status, body) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/secrets",
            Some(json!({"name": name, "purpose": "hook_secret", "value": "s3cret-value"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().expect("the secret id").to_owned()
}

async fn delete(env: &Env, id: &str) -> StatusCode {
    env.call(&env.admin, "DELETE", &format!("/api/v1/secrets/{id}"), None)
        .await
        .0
}

fn secret_ref(value: &str) -> Value {
    json!([{"name": "PGPASSWORD", "kind": "secret_ref", "value": value}])
}

/// FR-RP-05: a profile script that names a secret in upper case, with the body naming another one,
/// blocks the delete of the first secret only.
#[tokio::test]
async fn fr_rp_05_a_profile_script_blocks_the_delete_of_its_secret_only() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let agent = env.connect_agent_trusting("host-a", true, vec![]).await;
    let repository = env.create_repository(agent.id, "pw").await;
    let used = create_secret(&env, "used").await;
    let mentioned = create_secret(&env, "mentioned").await;
    let profile = env
        .create_profile(
            repository,
            json!({"scripts": [script(
                &format!("echo {mentioned}"),
                secret_ref(&used.to_uppercase()),
            )]}),
        )
        .await;

    let (status, saved) = env
        .call(
            &env.admin,
            "GET",
            &format!("/api/v1/profiles/{profile}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        saved["scripts"][0]["env"][0]["value"], used,
        "the reference must be saved in the canonical form"
    );

    let (status, list) = env.call(&env.admin, "GET", "/api/v1/secrets", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let entry = list["items"]
        .as_array()
        .expect("the items")
        .iter()
        .find(|item| item["id"] == used)
        .expect("the used secret");
    assert_eq!(entry["used_by"][0]["kind"], "profile");

    assert_eq!(delete(&env, &used).await, StatusCode::CONFLICT);
    assert_eq!(delete(&env, &mentioned).await, StatusCode::NO_CONTENT);
    env.db.cleanup().await;
}

/// FR-RP-05: a job template counts as a user of a secret, and it keeps only a secret that exists.
#[tokio::test]
async fn fr_rp_05_a_job_template_blocks_the_delete_of_its_secret() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let secret = create_secret(&env, "templated").await;
    let (status, created) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/job-templates",
            Some(json!({
                "name": "database",
                "description": "d",
                "scripts": [script("echo", secret_ref(&secret))],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(delete(&env, &secret).await, StatusCode::CONFLICT);

    let (status, refused) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/job-templates",
            Some(json!({
                "name": "dangling",
                "description": "d",
                "scripts": [script("echo", secret_ref(&uuid::Uuid::new_v4().to_string()))],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");

    let id = created["id"].as_str().expect("the template id");
    let (status, _) = env
        .call(
            &env.admin,
            "DELETE",
            &format!("/api/v1/job-templates/{id}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(delete(&env, &secret).await, StatusCode::NO_CONTENT);
    env.db.cleanup().await;
}

/// FR-RP-05: the usage list shows a channel only to a role that may read channels.
#[tokio::test]
async fn fr_rp_05_a_viewer_sees_no_channel_in_the_usage() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = Env::start(db, test_queue()).await;
    let (status, channel) = env
        .call(
            &env.admin,
            "POST",
            "/api/v1/notification-channels",
            Some(json!({
                "name": "hook",
                "kind": "webhook",
                "url": "https://hooks.example.com/in/token123",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{channel}");

    let kinds = |list: &Value| -> Vec<String> {
        list["items"]
            .as_array()
            .expect("the items")
            .iter()
            .flat_map(|item| item["used_by"].as_array().cloned().unwrap_or_default())
            .filter_map(|usage| usage["kind"].as_str().map(str::to_owned))
            .collect()
    };
    let (_, admin_list) = env.call(&env.admin, "GET", "/api/v1/secrets", None).await;
    assert!(
        kinds(&admin_list).contains(&"channel".to_owned()),
        "{admin_list}"
    );
    let (status, viewer_list) = env.call(&env.viewer, "GET", "/api/v1/secrets", None).await;
    assert_eq!(status, StatusCode::OK, "{viewer_list}");
    assert!(
        !kinds(&viewer_list).contains(&"channel".to_owned()),
        "{viewer_list}"
    );
    env.db.cleanup().await;
}
