//! The dashboard and the RPO field of the plan API (FR-MN-01, AC-MN-09, ADR-0035).

#![allow(clippy::expect_used)]

mod alert_support;
mod common;

use alert_support::{finish_job, local_sender, scoped, set_last_seen};
use axum::http::StatusCode;
use restichub_cp::config::TenancyConfig;
use restichub_cp::server::{AppState, router};
use serde_json::json;
use time::{Duration, OffsetDateTime};

use common::{
    create_tenant_with_key, request, seed_agent, seed_job, seed_profile, seed_repository,
    seed_scheduled_job, seed_scheduled_plan, seed_secret, seed_session, send, test_kek,
};

/// AC-MN-09: the dashboard shows the hosts that are online and offline, the plans with the last
/// success and the RPO state, the repository lock state, the failed jobs and the open alerts.
#[tokio::test]
async fn ac_mn_09_dashboard_overview() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ac-mn-09").await;
    let other = create_tenant_with_key(db.database(), "ac-mn-09-other").await;
    let sender = local_sender(None);
    let app = router(
        AppState::new(db.database().clone())
            .with_kek(test_kek())
            .with_tenancy(TenancyConfig::multi())
            .with_sender(sender.clone()),
    );
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let secret = seed_secret(db.database(), tenant, "dash.password").await;
    let repository = seed_repository(db.database(), tenant, "dash-repo", secret).await;
    let locked = seed_repository(
        db.database(),
        tenant,
        "dash-locked",
        seed_secret(db.database(), tenant, "dash.two").await,
    )
    .await;
    let profile = seed_profile(db.database(), tenant, "dash-profile", repository).await;
    let online = seed_agent(db.database(), tenant, "host-online").await;
    let offline = seed_agent(db.database(), tenant, "host-offline").await;
    let now = OffsetDateTime::now_utc();
    set_last_seen(&db, tenant, online, Some(now - Duration::seconds(20))).await;
    set_last_seen(&db, tenant, offline, Some(now - Duration::minutes(10))).await;

    let healthy = seed_scheduled_plan(
        db.database(),
        tenant,
        "healthy",
        profile,
        online,
        "0 2 * * *",
        "UTC",
        restichub_db::MissedRunPolicy::Skip,
        None,
    )
    .await;
    let risky = seed_scheduled_plan(
        db.database(),
        tenant,
        "risky",
        profile,
        online,
        "0 3 * * *",
        "UTC",
        restichub_db::MissedRunPolicy::Skip,
        None,
    )
    .await;
    let broken = seed_scheduled_plan(
        db.database(),
        tenant,
        "broken",
        profile,
        online,
        "0 4 * * *",
        "UTC",
        restichub_db::MissedRunPolicy::Skip,
        None,
    )
    .await;
    let mut tx = scoped(&db, tenant).await;
    sqlx::query("UPDATE plans SET created_at = $1")
        .bind(now - Duration::days(5))
        .execute(&mut *tx)
        .await
        .expect("age the plans");
    tx.commit().await.expect("commit");
    for (plan, age) in [(healthy, Duration::hours(2)), (risky, Duration::hours(21))] {
        let job =
            seed_scheduled_job(db.database(), tenant, plan, online, repository, now - age).await;
        finish_job(&db, tenant, job, "succeeded", "succeeded", None, now - age).await;
    }
    // A failed job of the broken plan, and a held and a stale lock.
    let failed = seed_scheduled_job(
        db.database(),
        tenant,
        broken,
        online,
        repository,
        now - Duration::hours(1),
    )
    .await;
    finish_job(
        &db,
        tenant,
        failed,
        "failed",
        "failed",
        Some("disk full"),
        now - Duration::minutes(50),
    )
    .await;
    let waiting = seed_job(db.database(), tenant, online, locked).await;
    let mut tx = scoped(&db, tenant).await;
    sqlx::query("UPDATE jobs SET lock_reported = true WHERE id = $1")
        .bind(waiting.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("hold the lock");
    tx.commit().await.expect("commit");
    let stale = seed_job(db.database(), tenant, online, repository).await;
    let mut tx = scoped(&db, tenant).await;
    sqlx::query("UPDATE jobs SET stale_lock_since = $2 WHERE id = $1")
        .bind(stale.as_uuid())
        .bind(now - Duration::minutes(5))
        .execute(&mut *tx)
        .await
        .expect("stale lock");
    tx.commit().await.expect("commit");

    // Open the alerts with a pass at the current time.
    let kek = test_kek();
    restichub_cp::alerts::run_once(db.database(), &sender, kek.as_ref(), now)
        .await
        .expect("pass");

    let (status, _, body) = send(
        &app,
        request("GET", "/api/v1/dashboard", Some(&viewer), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["hosts"]["total"], 2);
    assert_eq!(body["hosts"]["online"], 1);
    assert_eq!(body["hosts"]["offline"], 1);
    assert_eq!(
        body["hosts"]["items"][0]["hostname"], "host-offline",
        "offline hosts come first"
    );

    assert_eq!(body["plans"]["total"], 3);
    assert_eq!(body["plans"]["violated"], 1);
    assert_eq!(body["plans"]["at_risk"], 1);
    let plans = body["plans"]["items"].as_array().expect("plans");
    assert_eq!(plans[0]["name"], "broken");
    assert_eq!(plans[0]["state"], "violated");
    assert!(plans[0]["last_success_at"].is_null());
    assert_eq!(plans[1]["name"], "risky");
    assert_eq!(plans[1]["state"], "at_risk");
    assert!(plans[1]["last_success_at"].is_string());
    assert_eq!(plans[2]["name"], "healthy");
    assert_eq!(plans[2]["state"], "ok");
    assert_eq!(plans[2]["rpo_seconds"], 26 * 3600);

    let repositories = body["repositories"].as_array().expect("repositories");
    let state_of = |name: &str| {
        repositories
            .iter()
            .find(|repository| repository["name"] == name)
            .map(|repository| repository["lock_state"].clone())
    };
    assert_eq!(state_of("dash-repo"), Some(json!("stale")));
    assert_eq!(state_of("dash-locked"), Some(json!("held")));

    let failed_jobs = body["failed_jobs"].as_array().expect("failed jobs");
    assert_eq!(failed_jobs.len(), 1);
    assert_eq!(failed_jobs[0]["plan_name"], "broken");

    let kinds: Vec<&str> = body["alerts"]["items"]
        .as_array()
        .expect("alerts")
        .iter()
        .filter_map(|alert| alert["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"rpo_violated"), "{kinds:?}");
    assert!(kinds.contains(&"job_failed"), "{kinds:?}");
    assert!(body["alerts"]["total"].as_u64().expect("total") >= 2);

    // The dashboard of another tenant shows none of these rows.
    let other_viewer = seed_session(db.database(), other, "viewer-other", "viewer").await;
    let (_, _, empty) = send(
        &app,
        request("GET", "/api/v1/dashboard", Some(&other_viewer), None),
    )
    .await;
    assert_eq!(empty["hosts"]["total"], 0);
    assert_eq!(empty["plans"]["total"], 0);
    assert_eq!(empty["alerts"]["total"], 0);
}

/// The plan API stores and validates the recovery point objective (ADR-0035).
#[tokio::test]
async fn ac_mn_01_plan_api_rpo_field() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ac-mn-01-plan-rpo").await;
    let app = router(
        AppState::new(db.database().clone())
            .with_kek(test_kek())
            .with_tenancy(TenancyConfig::multi()),
    );
    let cookie = seed_session(db.database(), tenant, "operator", "operator").await;
    let secret = seed_secret(db.database(), tenant, "rpo.password").await;
    let repository = seed_repository(db.database(), tenant, "rpo-repo", secret).await;
    let profile = seed_profile(db.database(), tenant, "rpo-profile", repository).await;
    let agent = seed_agent(db.database(), tenant, "rpo-host").await;
    let body = |rpo: serde_json::Value| {
        json!({
            "name": "p", "profile_id": profile.to_string(), "agent_id": agent.to_string(),
            "cron": "0 2 * * *", "time_zone": "UTC", "missed_run_policy": "skip",
            "rpo_seconds": rpo,
        })
    };
    for bad in [json!(899), json!(34_560_001), json!(-5)] {
        let (status, _, response) = send(
            &app,
            request(
                "POST",
                "/api/v1/plans",
                Some(&cookie),
                Some(body(bad.clone())),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {response}");
        assert_eq!(response["field"], "rpo_seconds");
    }
    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/plans",
            Some(&cookie),
            Some(body(json!(7200))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["plan"]["rpo_seconds"], 7200);
    assert_eq!(created["plan"]["effective_rpo_seconds"], 7200);
    let id = created["plan"]["id"].as_str().expect("id");

    // Null restores the default of the schedule: 26 hours for a daily plan.
    let (status, _, updated) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/plans/{id}"),
            Some(&cookie),
            Some(body(json!(null))),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert!(updated["plan"]["rpo_seconds"].is_null());
    assert_eq!(updated["plan"]["effective_rpo_seconds"], 26 * 3600);
}
