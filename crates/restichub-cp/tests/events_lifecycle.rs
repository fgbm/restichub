//! How an event stream ends, and how the bus keeps tenants and clients apart (FR-UI-02, AC-UI-02).

#![allow(clippy::expect_used)]

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{
    create_tenant_with_key, create_user, request, seed_api_token, seed_session, send_response,
    session_cookie, test_kek,
};
use http_body_util::BodyExt;
use restichub_cp::config::TenancyConfig;
use restichub_cp::events::EventLimits;
use restichub_cp::server::{AppState, router};
use restichub_db::{JobId, memberships};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const WAIT: Duration = Duration::from_secs(5);

fn state(db: &restichub_db::test_util::TestDatabase, limits: EventLimits) -> AppState {
    AppState::new(db.database().clone())
        .with_kek(test_kek())
        .with_tenancy(TenancyConfig::multi())
        .with_event_limits(limits)
}

fn fast() -> EventLimits {
    EventLimits {
        recheck: Duration::from_millis(100),
        ..EventLimits::default()
    }
}

async fn open(app: &axum::Router, cookie: &str) -> axum::response::Response {
    send_response(app, request("GET", "/api/v1/events", Some(cookie), None)).await
}

async fn open_token(app: &axum::Router, token: &str) -> axum::response::Response {
    let mut request = Request::builder()
        .uri("/api/v1/events")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .expect("cannot build the request");
    request.extensions_mut().insert(common::peer());
    send_response(app, request).await
}

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

/// Reads until the stream ends. A stream that does not end fails the test after `WAIT`.
async fn read_to_end(body: &mut Body) {
    while next_text(body).await.is_some() {}
}

async fn eventually(mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..200 {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// AC-UI-02: the stop of the server closes an open stream, so the graceful shutdown ends at once.
#[tokio::test]
async fn ac_ui_02_the_server_stops_with_an_open_stream() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-stop").await;
    let state = state(&db, EventLimits::default());
    let cookie = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("cannot bind");
    let address = listener.local_addr().expect("the listener has an address");
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let events = state.events_shared();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router(state).into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = stop_rx.await;
            events.close();
        })
        .await
    });

    let mut client = tokio::net::TcpStream::connect(address)
        .await
        .expect("cannot connect");
    let head =
        format!("GET /api/v1/events HTTP/1.1\r\nHost: localhost\r\nCookie: {cookie}\r\n\r\n");
    client
        .write_all(head.as_bytes())
        .await
        .expect("cannot send the request");
    let mut seen = Vec::new();
    let mut buffer = [0_u8; 1024];
    while !String::from_utf8_lossy(&seen).contains("event: resync") {
        let read = tokio::time::timeout(WAIT, client.read(&mut buffer))
            .await
            .expect("the stream sent nothing in time")
            .expect("cannot read");
        assert_ne!(read, 0, "the stream ended early");
        seen.extend_from_slice(&buffer[..read]);
    }

    stop_tx.send(()).expect("the server is running");
    let stopped = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .expect("the server did not stop with an open stream")
        .expect("the server task failed");
    assert!(stopped.is_ok());
}

/// AC-UI-02: a client that does not read loses its stream, and the slot is free again.
#[tokio::test]
async fn ac_ui_02_a_client_that_does_not_read_loses_its_stream() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-slow").await;
    let limits = EventLimits {
        send_timeout: Duration::from_millis(100),
        ..EventLimits::default()
    };
    let state = state(&db, limits);
    let app = router(state.clone());
    let cookie = seed_session(db.database(), tenant, "viewer", "viewer").await;

    // The response stays open and nobody polls its body.
    let _unread = open(&app, &cookie).await;
    assert_eq!(state.events().open_streams(tenant), 1);
    for _ in 0..200 {
        state.events().job_log(tenant, JobId::new());
    }
    let events = state.events_shared();
    assert!(
        eventually(|| events.open_streams(tenant) == 0).await,
        "the stream of a client that does not read kept its slot"
    );
}

async fn assert_ends_after(app: &axum::Router, cookie: &str, revoke: impl AsyncFnOnce()) {
    let mut body = open(app, cookie).await.into_body();
    read_until(&mut body, "event: resync").await;
    revoke().await;
    read_to_end(&mut body).await;
}

/// AC-UI-02: sign-out closes the open streams of the session.
#[tokio::test]
async fn ac_ui_02_sign_out_closes_the_stream() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-out").await;
    let app = router(state(&db, fast()));
    let cookie = seed_session(db.database(), tenant, "viewer", "viewer").await;
    assert_ends_after(&app, &cookie, async || {
        let response = send_response(
            &app,
            request("POST", "/api/v1/auth/sign-out", Some(&cookie), None),
        )
        .await;
        assert!(response.status().is_success());
    })
    .await;
}

/// AC-UI-02: disabling the user closes the open stream.
#[tokio::test]
async fn ac_ui_02_a_disabled_user_loses_the_stream() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-off").await;
    let app = router(state(&db, fast()));
    let user = create_user(db.database(), tenant, "viewer", "phc", "viewer").await;
    let cookie = session_cookie(db.database(), tenant, user).await;
    assert_ends_after(&app, &cookie, async || {
        let mut tx = db.database().begin_tenant(tenant).await.expect("begin");
        memberships::set_disabled(&mut tx, user, true)
            .await
            .expect("cannot disable");
        tx.commit().await.expect("commit");
    })
    .await;
}

/// AC-UI-02: a change of the role closes the open stream, so the client reads with the new role.
#[tokio::test]
async fn ac_ui_02_a_role_change_closes_the_stream() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-role").await;
    let app = router(state(&db, fast()));
    let user = create_user(db.database(), tenant, "viewer", "phc", "admin").await;
    let cookie = session_cookie(db.database(), tenant, user).await;
    assert_ends_after(&app, &cookie, async || {
        let mut tx = db.database().begin_tenant(tenant).await.expect("begin");
        memberships::set_role(&mut tx, user, restichub_db::Role::Viewer)
            .await
            .expect("cannot change the role");
        tx.commit().await.expect("commit");
    })
    .await;
}

/// AC-UI-02: the suspension of the tenant closes the open stream.
#[tokio::test]
async fn ac_ui_02_a_suspended_tenant_loses_the_stream() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-susp").await;
    let app = router(state(&db, fast()));
    let cookie = seed_session(db.database(), tenant, "viewer", "viewer").await;
    assert_ends_after(&app, &cookie, async || {
        common::set_tenant_status(db.database(), tenant, "suspended").await;
    })
    .await;
}

/// AC-UI-02: the revocation of an API token closes the open stream of the token.
#[tokio::test]
async fn ac_ui_02_a_revoked_token_loses_the_stream() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-token").await;
    let app = router(state(&db, fast()));
    let token = seed_api_token(db.database(), tenant, "viewer").await;
    let id = common::list_tokens(db.database(), tenant).await[0].id;

    let response = open_token(&app, &token).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    read_until(&mut body, "event: resync").await;
    let mut tx = db.database().begin_tenant(tenant).await.expect("begin");
    restichub_db::api_tokens::revoke(&mut tx, id, time::OffsetDateTime::now_utc())
        .await
        .expect("cannot revoke");
    tx.commit().await.expect("commit");
    read_to_end(&mut body).await;
}

/// AC-UI-02: a stream keeps running while the access stays the same.
#[tokio::test]
async fn ac_ui_02_a_valid_stream_survives_the_recheck() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-keep").await;
    let state = state(&db, fast());
    let app = router(state.clone());
    let cookie = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let mut body = open(&app, &cookie).await.into_body();
    read_until(&mut body, "event: resync").await;
    tokio::time::sleep(Duration::from_millis(450)).await;
    state.events().alert_changed(tenant);
    read_until(&mut body, "event: alert.changed").await;
}

/// AC-UI-02, AC-TN-01: a burst in one tenant makes no stream of another tenant read again.
#[tokio::test]
async fn ac_ui_02_a_burst_of_one_tenant_does_not_reach_another() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let noisy = create_tenant_with_key(db.database(), "ui-events-noisy").await;
    let quiet = create_tenant_with_key(db.database(), "ui-events-quiet").await;
    let state = state(&db, EventLimits::default());
    let app = router(state.clone());
    let noisy_cookie = seed_session(db.database(), noisy, "noisy", "viewer").await;
    let quiet_cookie = seed_session(db.database(), quiet, "quiet", "viewer").await;
    let mut noisy_body = open(&app, &noisy_cookie).await.into_body();
    let mut quiet_body = open(&app, &quiet_cookie).await.into_body();
    read_until(&mut noisy_body, "event: resync").await;
    read_until(&mut quiet_body, "event: resync").await;

    for _ in 0..5000 {
        state.events().job_log(noisy, JobId::new());
    }
    state.events().alert_changed(quiet);
    let seen = read_until(&mut quiet_body, "event: alert.changed").await;
    assert!(!seen.contains("event: resync"), "the other tenant lagged");

    // The noisy tenant lags itself and reads again.
    read_until(&mut noisy_body, "event: resync").await;
}

/// AC-UI-02: a message for a job that is not a job of the agent makes no event.
#[tokio::test]
async fn ac_ui_02_an_unknown_job_makes_no_event() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-own").await;
    let state = state(&db, EventLimits::default());
    let app = router(state.clone());
    let cookie = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let agent = common::seed_agent(db.database(), tenant, "events-host").await;
    let mut body = open(&app, &cookie).await.into_body();
    read_until(&mut body, "event: resync").await;

    let unknown = JobId::new();
    restichub_cp::job_data::handle_progress(
        &state,
        tenant,
        agent,
        &restichub_proto::agent::v1::StepProgress {
            job_id: unknown.to_string(),
            step_id: "backup".to_owned(),
            ..Default::default()
        },
    )
    .await;
    restichub_cp::job_data::handle_step_result(
        &state,
        tenant,
        agent,
        &restichub_proto::agent::v1::StepResult {
            job_id: unknown.to_string(),
            step_id: "backup".to_owned(),
            ..Default::default()
        },
    )
    .await;
    state
        .events()
        .job_status(tenant, JobId::new(), restichub_db::JobStatus::Running);
    let seen = read_until(&mut body, "event: job.status").await;
    assert!(!seen.contains("job.progress"));
    assert!(!seen.contains(&unknown.to_string()));
}

/// AC-UI-02: an API token has its own limit, below the limit of the tenant.
#[tokio::test]
async fn ac_ui_02_the_stream_count_of_a_token_is_limited() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-tokens").await;
    let limits = EventLimits {
        per_token: 2,
        ..EventLimits::default()
    };
    let app = router(state(&db, limits));
    let token = seed_api_token(db.database(), tenant, "viewer").await;
    let first = open_token(&app, &token).await;
    let second = open_token(&app, &token).await;
    let third = open_token(&app, &token).await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(third.status(), StatusCode::TOO_MANY_REQUESTS);
}

/// AC-UI-02: the streams of one address and of the whole server are limited.
#[tokio::test]
async fn ac_ui_02_the_address_and_the_server_limit_the_streams() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ui-events-wide").await;
    let limits = EventLimits {
        per_ip: 2,
        ..EventLimits::default()
    };
    let app = router(state(&db, limits));
    let one = seed_session(db.database(), tenant, "one", "viewer").await;
    let two = seed_session(db.database(), tenant, "two", "viewer").await;
    let three = seed_session(db.database(), tenant, "three", "viewer").await;
    let _a = open(&app, &one).await;
    let _b = open(&app, &two).await;
    assert_eq!(
        open(&app, &three).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );

    let limits = EventLimits {
        global: 1,
        ..EventLimits::default()
    };
    let app = router(state(&db, limits));
    let _c = open(&app, &one).await;
    assert_eq!(
        open(&app, &two).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}
