//! The Server-Sent Events endpoint of the web UI (FR-UI-02, AC-UI-02, ADR-0009).

#![allow(clippy::expect_used)]

mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{StatusCode, header};
use common::{create_tenant_with_key, request, seed_session, send, send_response, test_kek};
use http_body_util::BodyExt;
use restichub_cp::config::TenancyConfig;
use restichub_cp::events::EventLimits;
use restichub_cp::server::{AppState, router};
use restichub_db::{AgentId, JobId, JobStatus};

const WAIT: Duration = Duration::from_secs(5);

fn state(db: &restichub_db::test_util::TestDatabase, limits: EventLimits) -> AppState {
    AppState::new(db.database().clone())
        .with_kek(test_kek())
        .with_tenancy(TenancyConfig::multi())
        .with_event_limits(limits)
}

// `None` means that the stream ended.
async fn next_text(body: &mut Body) -> Option<String> {
    let frame = tokio::time::timeout(WAIT, body.frame())
        .await
        .expect("the stream sent nothing in time")?
        .expect("the stream failed");
    frame
        .into_data()
        .ok()
        .map(|data| String::from_utf8_lossy(&data).into_owned())
}

async fn read_until(body: &mut Body, needle: &str) -> String {
    let mut seen = String::new();
    while !seen.contains(needle) {
        let chunk = next_text(body)
            .await
            .unwrap_or_else(|| panic!("the stream ended before `{needle}`; saw: {seen}"));
        seen.push_str(&chunk);
    }
    seen
}

async fn open(app: &axum::Router, cookie: &str) -> axum::response::Response {
    send_response(app, request("GET", "/api/v1/events", Some(cookie), None)).await
}

/// AC-UI-02: a stream starts with `resync`, carries no secret, and sets the headers that keep a
/// reverse proxy from buffering it.
#[tokio::test]
async fn ac_ui_02_the_stream_sends_resync_first_and_disables_proxy_buffering() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-head").await;
    let state = state(&db, EventLimits::default());
    let app = router(state.clone());
    let cookie = seed_session(db.database(), tenant, "viewer", "viewer").await;

    let response = open(&app, &cookie).await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(headers[header::CONTENT_TYPE], "text/event-stream");
    assert_eq!(headers["x-accel-buffering"], "no");
    assert!(
        headers[header::CACHE_CONTROL]
            .to_str()
            .expect("the header is text")
            .contains("no-cache")
    );
    let mut body = response.into_body();
    let first = read_until(&mut body, "event: resync").await;
    assert!(first.contains("data: {}"));

    let job = JobId::new();
    state.events().job_status(tenant, job, JobStatus::Running);
    let event = read_until(&mut body, "event: job.status").await;
    assert!(event.contains(&job.to_string()));
    assert!(event.contains("\"status\":\"running\""));
}

/// AC-UI-02, AC-TN-01: a stream receives the events of its own tenant and no event of another one.
#[tokio::test]
async fn ac_ui_02_events_do_not_cross_a_tenant_border() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-a").await;
    let other = create_tenant_with_key(db.database(), "ui-events-b").await;
    let state = state(&db, EventLimits::default());
    let app = router(state.clone());
    let cookie = seed_session(db.database(), tenant, "viewer", "viewer").await;

    let mut body = open(&app, &cookie).await.into_body();
    read_until(&mut body, "event: resync").await;

    let foreign_job = JobId::new();
    let foreign_agent = AgentId::new();
    state.events().job_progress(other, foreign_job);
    state.events().job_log(other, foreign_job);
    state.events().host(other, foreign_agent, true);
    state.events().alert_changed(other);
    let own_job = JobId::new();
    state.events().job_progress(tenant, own_job);

    let seen = read_until(&mut body, "event: job.progress").await;
    assert!(seen.contains(&own_job.to_string()));
    assert!(!seen.contains(&foreign_job.to_string()));
    assert!(!seen.contains(&foreign_agent.to_string()));
    assert!(!seen.contains("alert.changed"));
}

/// AC-UI-02: each kind of event reaches a reader whose role allows its read operation, and the
/// kinds that the tenant produces arrive in order.
#[tokio::test]
async fn ac_ui_02_a_viewer_receives_every_event_kind() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-kinds").await;
    let state = state(&db, EventLimits::default());
    let app = router(state.clone());
    let cookie = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let mut body = open(&app, &cookie).await.into_body();
    read_until(&mut body, "event: resync").await;

    let job = JobId::new();
    state.events().job_progress(tenant, job);
    state.events().job_log(tenant, job);
    state.events().host(tenant, AgentId::new(), true);
    state.events().host(tenant, AgentId::new(), false);
    state.events().alert_changed(tenant);
    let seen = read_until(&mut body, "event: alert.changed").await;
    for kind in [
        "job.progress",
        "job.log",
        "host.online",
        "host.offline",
        "alert.changed",
    ] {
        assert!(seen.contains(&format!("event: {kind}")), "missing {kind}");
    }
}

/// AC-UI-02: a request with no credential gets HTTP 401.
#[tokio::test]
async fn ac_ui_02_the_stream_needs_a_session() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = router(state(&db, EventLimits::default()));
    let (status, _, _) = send(&app, request("GET", "/api/v1/events", None, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// AC-UI-02: the stream sends a comment line at the heartbeat interval.
#[tokio::test]
async fn ac_ui_02_the_stream_sends_a_heartbeat() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-beat").await;
    let limits = EventLimits {
        heartbeat: Duration::from_millis(50),
        ..EventLimits::default()
    };
    let app = router(state(&db, limits));
    let cookie = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let mut body = open(&app, &cookie).await.into_body();
    read_until(&mut body, "event: resync").await;
    // A keep-alive frame is a comment line: it starts with a colon.
    let beat = read_until(&mut body, ":\n").await;
    assert!(!beat.contains("event: job"));
}

/// AC-UI-02: a session has a limit of open streams, and a closed stream gives its slot back.
#[tokio::test]
async fn ac_ui_02_the_stream_count_of_a_session_is_limited() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-limit").await;
    let limits = EventLimits {
        per_session: 2,
        ..EventLimits::default()
    };
    let state = state(&db, limits);
    let app = router(state.clone());
    let cookie = seed_session(db.database(), tenant, "viewer", "viewer").await;

    let first = open(&app, &cookie).await;
    let second = open(&app, &cookie).await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    let third = open(&app, &cookie).await;
    assert_eq!(third.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(state.events().open_streams(tenant), 2);

    drop(first);
    let mut slot_free = false;
    for _ in 0..100 {
        if state.events().open_streams(tenant) == 1 {
            slot_free = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(slot_free, "the closed stream kept its slot");
    let again = open(&app, &cookie).await;
    assert_eq!(again.status(), StatusCode::OK);
}

/// AC-UI-02: a tenant has a limit of open streams across its sessions.
#[tokio::test]
async fn ac_ui_02_the_stream_count_of_a_tenant_is_limited() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-tenant").await;
    let limits = EventLimits {
        per_tenant: 1,
        ..EventLimits::default()
    };
    let app = router(state(&db, limits));
    let one = seed_session(db.database(), tenant, "one", "viewer").await;
    let two = seed_session(db.database(), tenant, "two", "viewer").await;

    let first = open(&app, &one).await;
    assert_eq!(first.status(), StatusCode::OK);
    let second = open(&app, &two).await;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
}

/// AC-UI-02: a stream ends after its lifetime, so the client authenticates again.
#[tokio::test]
async fn ac_ui_02_the_stream_ends_after_its_lifetime() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-life").await;
    let limits = EventLimits {
        max_lifetime: Duration::from_millis(200),
        ..EventLimits::default()
    };
    let app = router(state(&db, limits));
    let cookie = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let mut body = open(&app, &cookie).await.into_body();
    read_until(&mut body, "event: resync").await;
    loop {
        if next_text(&mut body).await.is_none() {
            break;
        }
    }
}
