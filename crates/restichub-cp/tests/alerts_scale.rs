//! The alert loop at scale: tenants run on their own, and the evaluation reads every row
//! (FR-MN-03, FR-MN-04, AC-MN-07, ADR-0036).

#![allow(clippy::expect_used)]

mod alert_support;
mod common;

use std::time::{Duration, Instant};

use alert_support::{WebhookSink, local_sender, scoped};
use axum::Router;
use axum::http::StatusCode;
use restichub_cp::clock::ManualClock;
use restichub_cp::config::TenancyConfig;
use restichub_cp::server::{AppState, router};
use restichub_db::test_util::TestDatabase;
use restichub_db::{PLATFORM_TENANT_ID, TenantId, TenantStatus, tenants};
use serde_json::json;
use time::OffsetDateTime;
use tokio::net::TcpListener;

use common::{create_tenant_with_key, request, seed_agent, seed_session, send, test_kek};

fn app_for(db: &TestDatabase) -> Router {
    router(
        AppState::new(db.database().clone())
            .with_kek(test_kek())
            .with_tenancy(TenancyConfig::multi())
            .with_sender(local_sender(None)),
    )
}

async fn webhook_channel(app: &Router, cookie: &str, port: u16) {
    let (status, _, body) = send(
        app,
        request(
            "POST",
            "/api/v1/notification-channels",
            Some(cookie),
            Some(json!({
                "name": "hook",
                "kind": "webhook",
                "url": format!("http://127.0.0.1:{port}/hook"),
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

/// A server that accepts a connection and never answers.
async fn silent_target() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("address").port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });
    port
}

/// AC-MN-07: a target of one tenant that never answers does not delay the messages of another
/// tenant.
#[tokio::test]
async fn ac_mn_07_a_slow_target_does_not_delay_another_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let slow = create_tenant_with_key(db.database(), "slow-tenant").await;
    let fast = create_tenant_with_key(db.database(), "fast-tenant").await;
    let app = app_for(&db);
    let hook = WebhookSink::start().await;
    let silent = silent_target().await;
    for (tenant, port, user) in [
        (slow, silent, "admin-slow"),
        (fast, hook.port, "admin-fast"),
    ] {
        let cookie = seed_session(db.database(), tenant, user, "admin").await;
        webhook_channel(&app, &cookie, port).await;
        seed_agent(db.database(), tenant, "silent-host").await;
    }

    // Start the pass with the slow tenant first, so a serial loop would wait for it.
    let offset = {
        let mut tx = db.database().begin_platform().await.expect("begin");
        let mut list = tenants::list(&mut tx).await.expect("list");
        tx.commit().await.expect("commit");
        list.retain(|tenant| {
            tenant.status == TenantStatus::Active && tenant.id != PLATFORM_TENANT_ID
        });
        list.iter()
            .position(|tenant| tenant.id == slow)
            .expect("the slow tenant")
    };
    let clock = ManualClock::new(OffsetDateTime::now_utc() + time::Duration::hours(3)).shared();
    let (database, sender, kek) = (db.database().clone(), local_sender(None), test_kek());
    let pass = tokio::spawn(async move {
        restichub_cp::alerts::run_once_with(
            &database,
            &sender,
            kek,
            clock,
            restichub_cp::alerts::CONCURRENCY,
            restichub_cp::alerts::TENANT_BUDGET,
            offset,
        )
        .await
    });
    let started = Instant::now();
    while hook.received().is_empty() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the fast tenant waits for the slow target"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!pass.is_finished(), "the slow send still runs");
    pass.abort();
    db.cleanup().await;
}

/// Seeds an agent and copies its row, so a test gets many agents with few statements.
async fn bulk_agents(db: &TestDatabase, tenant: TenantId, count: i32) {
    let first = seed_agent(db.database(), tenant, "template").await;
    let mut tx = scoped(db, tenant).await;
    sqlx::query(
        "INSERT INTO agents SELECT (jsonb_populate_record(a, jsonb_build_object(\
             'id', gen_random_uuid()::text, \
             'hostname', 'bulk-' || lpad(n::text, 6, '0'), \
             'certificate_serial', '\\x' || md5(n::text)))).* \
         FROM agents a, generate_series(1, $2) AS n WHERE a.id = $1",
    )
    .bind(first.as_uuid())
    .bind(count)
    .execute(&mut *tx)
    .await
    .expect("cannot copy the agents");
    tx.commit().await.expect("commit");
}

async fn open_alerts(db: &TestDatabase, tenant: TenantId) -> i64 {
    let mut tx = scoped(db, tenant).await;
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM alert_state WHERE status = 'open'")
        .fetch_one(&mut *tx)
        .await
        .expect("count");
    tx.commit().await.expect("commit");
    count
}

/// AC-MN-07: the evaluation reads every agent of a tenant. It resolves no alert of an agent that it
/// read, and it opens one for the last one.
#[tokio::test]
async fn ac_mn_07_the_evaluation_reads_more_than_two_thousand_agents() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "many-agents").await;
    bulk_agents(&db, tenant, 2000).await;
    let now = OffsetDateTime::now_utc() + time::Duration::hours(3);
    let first = restichub_cp::alerts::evaluate::evaluate_tenant(db.database(), tenant, now)
        .await
        .expect("evaluation");
    assert_eq!(first.opened, 2001);
    assert_eq!(open_alerts(&db, tenant).await, 2001);
    let again = restichub_cp::alerts::evaluate::evaluate_tenant(db.database(), tenant, now)
        .await
        .expect("evaluation");
    assert_eq!((again.opened, again.resolved), (0, 0));
    assert_eq!(open_alerts(&db, tenant).await, 2001);
    db.cleanup().await;
}
