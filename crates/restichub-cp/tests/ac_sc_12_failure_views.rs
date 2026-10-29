//! AC-SC-12: the API shows the failure state of a plan and of a maintenance schedule, and only an
//! edit of the schedule fields clears it (FR-SC-12).

#![allow(clippy::expect_used)]

mod common;
mod schedule_failure_support;

use axum::Router;
use restichub_db::TenantId;
use restichub_db::test_util::TestDatabase;
use serde_json::{Value, json};

use common::{
    app_with_kek, create_tenant_with_key, request, seed_agent, seed_profile, seed_repository,
    seed_secret, seed_session, send,
};
use schedule_failure_support::owner_exec;

struct Fixture {
    db: TestDatabase,
    tenant: TenantId,
    app: Router,
    cookie: String,
    profile: String,
    agent: String,
    repository: String,
}

async fn fixture(name: &str) -> Option<Fixture> {
    let db = common::test_db().await?;
    let tenant = create_tenant_with_key(db.database(), name).await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let secret = seed_secret(db.database(), tenant, "views.password").await;
    let repository = seed_repository(db.database(), tenant, "views-repo", secret)
        .await
        .to_string();
    let profile = seed_profile(db.database(), tenant, "views-profile")
        .await
        .to_string();
    let agent = seed_agent(db.database(), tenant, "views-host")
        .await
        .to_string();
    Some(Fixture {
        db,
        tenant,
        app,
        cookie,
        profile,
        agent,
        repository,
    })
}

impl Fixture {
    fn plan_body(&self, name: &str, cron: &str) -> Value {
        json!({
            "name": name,
            "profile_id": self.profile,
            "agent_id": self.agent,
            "repository_id": self.repository,
            "cron": cron,
            "time_zone": "UTC",
            "missed_run_policy": "run_all",
        })
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> Value {
        let (status, _, response) =
            send(&self.app, request(method, uri, Some(&self.cookie), body)).await;
        assert!(status.is_success(), "{method} {uri}: {status} {response}");
        response
    }
}

/// AC-SC-12: the plan view carries the reason, and the plan counts as at risk on the plan view and
/// on the dashboard while the schedule is failed. The plan stays enabled.
#[tokio::test]
async fn ac_sc_12_the_views_show_a_failed_plan_as_at_risk() {
    let Some(fx) = fixture("failure-views").await else {
        return;
    };
    let created = fx
        .call(
            "POST",
            "/api/v1/plans",
            Some(fx.plan_body("nightly", "0 2 * * *")),
        )
        .await;
    let plan_id = created["plan"]["id"].as_str().expect("id").to_owned();
    assert!(created["plan"]["schedule_failure"].is_null());
    assert_eq!(created["plan"]["rpo_state"], "ok");

    owner_exec(
        &fx.db,
        fx.tenant,
        "UPDATE plans SET schedule_failure = 'invalid_cron', failed_since = now(), \
         next_run_at = NULL WHERE id = $1",
        &[plan_id.parse().expect("uuid")],
    )
    .await;

    let plan = fx
        .call("GET", &format!("/api/v1/plans/{plan_id}"), None)
        .await;
    let failure = &plan["schedule_failure"];
    assert_eq!(failure["reason"], "invalid_cron", "{plan}");
    assert!(failure["since"].is_string());
    assert!(failure["retry_after"].is_null(), "only an edit helps");
    assert_eq!(plan["enabled"], true);
    assert_eq!(plan["rpo_state"], "at_risk");

    let dashboard = fx.call("GET", "/api/v1/dashboard", None).await;
    assert_eq!(dashboard["plans"]["at_risk"], 1, "{dashboard}");
    assert_eq!(dashboard["plans"]["items"][0]["state"], "at_risk");
    fx.db.cleanup().await;
}

/// AC-SC-12: a rename keeps the failure, an edit of the cron expression clears it.
#[tokio::test]
async fn ac_sc_12_only_an_edit_of_the_schedule_clears_the_failure_of_a_plan() {
    let Some(fx) = fixture("failure-clear").await else {
        return;
    };
    let created = fx
        .call(
            "POST",
            "/api/v1/plans",
            Some(fx.plan_body("nightly", "0 2 * * *")),
        )
        .await;
    let plan_id = created["plan"]["id"].as_str().expect("id").to_owned();
    owner_exec(
        &fx.db,
        fx.tenant,
        "UPDATE plans SET schedule_failure = 'database_error', failed_since = now(), \
         retry_after = now() + interval '1 minute', failure_attempts = 2 WHERE id = $1",
        &[plan_id.parse().expect("uuid")],
    )
    .await;
    let uri = format!("/api/v1/plans/{plan_id}");

    let renamed = fx
        .call("PATCH", &uri, Some(fx.plan_body("renamed", "0 2 * * *")))
        .await;
    assert_eq!(renamed["plan"]["name"], "renamed");
    let failure = &renamed["plan"]["schedule_failure"];
    assert_eq!(failure["reason"], "database_error", "{renamed}");
    assert!(failure["retry_after"].is_string());

    let edited = fx
        .call("PATCH", &uri, Some(fx.plan_body("renamed", "0 3 * * *")))
        .await;
    assert!(edited["plan"]["schedule_failure"].is_null(), "{edited}");
    assert!(edited["plan"]["next_run_at"].is_string());
    assert_eq!(edited["plan"]["rpo_state"], "ok");
    fx.db.cleanup().await;
}

fn maintenance_body(retention: &str, check: &str) -> Value {
    json!({
        "time_zone": "UTC",
        "retention": {"cron": retention, "enabled": true},
        "check": {"cron": check, "enabled": true},
    })
}

/// AC-SC-12: the repository maintenance shows the failure of a schedule, a save without a change
/// keeps it, and a changed cron expression clears it.
#[tokio::test]
async fn ac_sc_12_the_maintenance_view_shows_and_clears_the_failure() {
    let Some(fx) = fixture("failure-maintenance").await else {
        return;
    };
    let uri = format!("/api/v1/repositories/{}/maintenance", fx.repository);
    fx.call(
        "PUT",
        &uri,
        Some(maintenance_body("0 3 * * 0", "0 4 1 * *")),
    )
    .await;
    let repository: uuid::Uuid = fx.repository.parse().expect("uuid");
    owner_exec(
        &fx.db,
        fx.tenant,
        "UPDATE maintenance_schedules SET schedule_failure = 'unknown_time_zone', \
         failed_since = now(), next_run_at = NULL WHERE repository_id = $1 AND task = 'check'",
        &[repository],
    )
    .await;

    let view = fx.call("GET", &uri, None).await;
    assert_eq!(
        view["check"]["schedule_failure"]["reason"],
        "unknown_time_zone"
    );
    assert!(view["check"]["schedule_failure"]["retry_after"].is_null());
    assert!(view["retention"]["schedule_failure"].is_null());

    let unchanged = fx
        .call(
            "PUT",
            &uri,
            Some(maintenance_body("0 3 * * 0", "0 4 1 * *")),
        )
        .await;
    assert_eq!(
        unchanged["check"]["schedule_failure"]["reason"], "unknown_time_zone",
        "a save without a change keeps the failure"
    );

    let edited = fx
        .call(
            "PUT",
            &uri,
            Some(maintenance_body("0 3 * * 0", "0 5 1 * *")),
        )
        .await;
    assert!(edited["check"]["schedule_failure"].is_null(), "{edited}");
    assert!(edited["check"]["next_run_at"].is_string());
    fx.db.cleanup().await;
}
