//! A refused delete names its reason (FR-RP-01, FR-SC-11).
//!
//! Each blocked delete returns a typed code. The code of a record that others reference carries
//! a bounded list of those records. The usage route returns the same answer before the delete.

#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use restichub_db::{PLATFORM_TENANT_ID, TenantId};
use serde_json::Value;

use common::{
    app_with_kek, bearer_request, create_tenant_with_key, request, seed_agent, seed_job, seed_plan,
    seed_profile, seed_repository, seed_secret, seed_session, send,
};

struct Fixture {
    db: restichub_db::test_util::TestDatabase,
    tenant: TenantId,
    app: axum::Router,
    cookie: String,
}

async fn fixture() -> Option<Fixture> {
    let db = common::test_db().await?;
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    Some(Fixture {
        db,
        tenant,
        app,
        cookie,
    })
}

impl Fixture {
    async fn call(&self, method: &str, path: &str) -> (StatusCode, Value) {
        let (status, _, body) =
            send(&self.app, request(method, path, Some(&self.cookie), None)).await;
        (status, body)
    }

    async fn repository(&self, name: &str) -> restichub_db::RepositoryId {
        let secret = seed_secret(self.db.database(), self.tenant, &format!("{name}-key")).await;
        seed_repository(self.db.database(), self.tenant, name, secret).await
    }
}

fn names(body: &Value) -> Vec<&str> {
    body["blockers"]["items"]
        .as_array()
        .expect("the blocker list")
        .iter()
        .map(|item| item["name"].as_str().expect("the name"))
        .collect()
}

/// A profile holds the repository. The body names the profiles and counts all of them.
#[tokio::test]
async fn a_repository_in_use_names_its_profiles() {
    let Some(f) = fixture().await else {
        return;
    };
    let repository = f.repository("main").await;
    seed_profile(f.db.database(), f.tenant, "zeta", repository).await;
    seed_profile(f.db.database(), f.tenant, "Alpha", repository).await;

    let (status, body) = f
        .call("DELETE", &format!("/api/v1/repositories/{repository}"))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "repository_in_use");
    assert_eq!(names(&body), ["Alpha", "zeta"]);
    assert_eq!(body["blockers"]["total"], 2);
    assert_eq!(body["blockers"]["items"][0]["kind"], "profile");

    let (status, usage) = f
        .call("GET", &format!("/api/v1/repositories/{repository}/usage"))
        .await;
    assert_eq!(status, StatusCode::OK, "{usage}");
    assert_eq!(usage["blocked_by"], "repository_in_use");
    assert_eq!(names(&usage), ["Alpha", "zeta"]);
    f.db.cleanup().await;
}

/// The list holds ten records at most. The total counts all of them.
#[tokio::test]
async fn the_blocker_list_is_bounded() {
    let Some(f) = fixture().await else {
        return;
    };
    let repository = f.repository("main").await;
    for index in 0..12 {
        seed_profile(
            f.db.database(),
            f.tenant,
            &format!("p{index:02}"),
            repository,
        )
        .await;
    }
    let (status, body) = f
        .call("DELETE", &format!("/api/v1/repositories/{repository}"))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(names(&body).len(), 10);
    assert_eq!(body["blockers"]["total"], 12);
    f.db.cleanup().await;
}

/// An active job holds the repository, and no profile does.
#[tokio::test]
async fn a_repository_with_an_active_job_has_its_own_code() {
    let Some(f) = fixture().await else {
        return;
    };
    let repository = f.repository("busy").await;
    let agent = seed_agent(f.db.database(), f.tenant, "host-1").await;
    seed_job(f.db.database(), f.tenant, agent, repository).await;

    let (status, body) = f
        .call("DELETE", &format!("/api/v1/repositories/{repository}"))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "repository_has_active_job");
    assert_eq!(body["blockers"]["total"], 1);
    assert_eq!(body["blockers"]["items"][0]["kind"], "job");

    let (status, usage) = f
        .call("GET", &format!("/api/v1/repositories/{repository}/usage"))
        .await;
    assert_eq!(status, StatusCode::OK, "{usage}");
    assert_eq!(usage["blocked_by"], "repository_has_active_job");
    f.db.cleanup().await;
}

/// The usage of a free repository is empty, and the delete succeeds.
#[tokio::test]
async fn a_free_repository_reports_no_blocker() {
    let Some(f) = fixture().await else {
        return;
    };
    let repository = f.repository("free").await;
    let (status, usage) = f
        .call("GET", &format!("/api/v1/repositories/{repository}/usage"))
        .await;
    assert_eq!(status, StatusCode::OK, "{usage}");
    assert!(usage["blocked_by"].is_null());
    assert_eq!(usage["blockers"]["total"], 0);
    let (status, _) = f
        .call("DELETE", &format!("/api/v1/repositories/{repository}"))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = f
        .call("GET", &format!("/api/v1/repositories/{repository}/usage"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    f.db.cleanup().await;
}

/// Plans hold a foreign key to the profile (FR-SC-11).
#[tokio::test]
async fn a_profile_in_use_names_its_plans() {
    let Some(f) = fixture().await else {
        return;
    };
    let repository = f.repository("main").await;
    let profile = seed_profile(f.db.database(), f.tenant, "daily", repository).await;
    let agent = seed_agent(f.db.database(), f.tenant, "host-1").await;
    seed_plan(f.db.database(), f.tenant, "nightly", profile, agent).await;

    let (status, body) = f
        .call("DELETE", &format!("/api/v1/profiles/{profile}"))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "profile_in_use");
    assert_eq!(names(&body), ["nightly"]);
    assert_eq!(body["blockers"]["items"][0]["kind"], "plan");

    let (status, usage) = f
        .call("GET", &format!("/api/v1/profiles/{profile}/usage"))
        .await;
    assert_eq!(status, StatusCode::OK, "{usage}");
    assert_eq!(usage["blocked_by"], "profile_in_use");
    f.db.cleanup().await;
}

/// The foreign key of a repository names the secret.
#[tokio::test]
async fn a_secret_in_use_names_its_objects() {
    let Some(f) = fixture().await else {
        return;
    };
    let secret = seed_secret(f.db.database(), f.tenant, "shared").await;
    seed_repository(f.db.database(), f.tenant, "uses-secret", secret).await;

    let (status, body) = f.call("DELETE", &format!("/api/v1/secrets/{secret}")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "secret_in_use");
    assert_eq!(names(&body), ["uses-secret"]);
    assert_eq!(body["blockers"]["items"][0]["kind"], "repository");

    f.db.cleanup().await;
}

/// A reader sees no channel or job template through a usage route (FR-SC-11).
#[tokio::test]
async fn a_viewer_sees_no_channel_or_template_in_a_usage_route() {
    let Some(f) = fixture().await else {
        return;
    };
    let secret = seed_secret(f.db.database(), f.tenant, "shared").await;
    let repository = seed_repository(f.db.database(), f.tenant, "main", secret).await;
    let profile = seed_profile(f.db.database(), f.tenant, "daily", repository).await;
    let viewer = seed_session(f.db.database(), f.tenant, "viewer", "viewer").await;

    for path in [
        format!("/api/v1/secrets/{secret}/usage"),
        format!("/api/v1/repositories/{repository}/usage"),
        format!("/api/v1/profiles/{profile}/usage"),
    ] {
        let (status, _, body) = send(&f.app, request("GET", &path, Some(&viewer), None)).await;
        let kinds: Vec<&str> = body["blockers"]["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item["kind"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            !kinds
                .iter()
                .any(|kind| matches!(*kind, "channel" | "job_template")),
            "{path}: {status} {body}"
        );
        if path.contains("/secrets/") {
            assert!(status.is_client_error(), "{path}: {status} {body}");
        }
    }
    f.db.cleanup().await;
}

/// The caller cannot remove its own access. The body holds no list.
#[tokio::test]
async fn an_admin_cannot_remove_its_own_access() {
    let Some(f) = fixture().await else {
        return;
    };
    let users = common::list_users(f.db.database(), f.tenant).await;
    let me = users.first().map(|user| user.id).expect("the admin");
    let (status, body) = f.call("DELETE", &format!("/api/v1/users/{me}")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "own_account_protected");
    assert!(body.get("blockers").is_none());
    f.db.cleanup().await;
}

/// The last Admin stays, also when another actor tries the delete.
#[tokio::test]
async fn the_last_admin_has_its_own_code() {
    let Some(f) = fixture().await else {
        return;
    };
    let users = common::list_users(f.db.database(), f.tenant).await;
    let admin = users.first().map(|user| user.id).expect("the admin");
    let token = common::seed_admin_token_actor(f.db.database(), f.tenant).await;
    let (status, _, body) = send(
        &f.app,
        bearer_request("DELETE", &format!("/api/v1/users/{admin}"), &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "last_admin");
    f.db.cleanup().await;
}

/// The platform organization cannot be suspended or deleted.
#[tokio::test]
async fn the_platform_organization_is_protected() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let id = PLATFORM_TENANT_ID;
    for (method, path) in [
        ("DELETE", format!("/api/v1/platform/tenants/{id}")),
        ("POST", format!("/api/v1/platform/tenants/{id}/suspend")),
    ] {
        let (status, _, body) = send(&app, request(method, &path, Some(&cookie), None)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{method} {path} {body}");
        assert_eq!(body["code"], "platform_tenant_protected", "{method} {path}");
    }
    db.cleanup().await;
}
