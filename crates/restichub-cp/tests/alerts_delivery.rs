//! The delivery of alert messages reads the channel again, counts an attempt only for a send that
//! starts, and survives an alert row that it cannot fully read (FR-MN-04, AC-MN-07, ADR-0036).

#![allow(clippy::expect_used)]

mod alert_support;
mod common;

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use alert_support::{FixedResolver, WebhookSink, judged_view, local_sender, scoped, set_last_seen};
use axum::Router;
use axum::http::StatusCode;
use restichub_cp::alerts::deliver::{self, Report};
use restichub_cp::alerts::evaluate;
use restichub_cp::alerts::transport::{ResolveFuture, Resolver};
use restichub_cp::clock::{Clock, ManualClock};
use restichub_cp::config::TenancyConfig;
use restichub_cp::server::{AppState, router};
use restichub_db::alerts::{self, AlertKind, AlertParams, DeliveryEvent, NewAlert};
use restichub_db::test_util::TestDatabase;
use restichub_db::{AlertId, TenantId};
use serde_json::json;
use time::OffsetDateTime;
use tokio::sync::Notify;
use tokio::time::Instant;

use common::{create_tenant_with_key, request, seed_agent, seed_session, send, test_kek};

/// The first lookup waits until the test lets it go; every lookup then answers with the loopback.
struct GateResolver {
    first: AtomicBool,
    entered: Arc<Notify>,
    open: Arc<Notify>,
}

impl Resolver for GateResolver {
    fn resolve(&self, _host: &str, _port: u16) -> ResolveFuture {
        let wait = self.first.swap(false, Ordering::SeqCst);
        let (entered, open) = (self.entered.clone(), self.open.clone());
        Box::pin(async move {
            if wait {
                entered.notify_one();
                open.notified().await;
            }
            Ok(vec![IpAddr::from([127, 0, 0, 1])])
        })
    }
}

// Makes a webhook send run into its timeout.
struct HangingResolver;

impl Resolver for HangingResolver {
    fn resolve(&self, _host: &str, _port: u16) -> ResolveFuture {
        Box::pin(std::future::pending())
    }
}

fn app_for(db: &TestDatabase) -> Router {
    let sender =
        local_sender(None).with_resolver(Arc::new(FixedResolver(IpAddr::from([127, 0, 0, 1]))));
    router(
        AppState::new(db.database().clone())
            .with_kek(test_kek())
            .with_tenancy(TenancyConfig::multi())
            .with_sender(sender),
    )
}

async fn webhook_channel(app: &Router, cookie: &str, url: String) -> String {
    let (status, _, body) = send(
        app,
        request(
            "POST",
            "/api/v1/notification-channels",
            Some(cookie),
            Some(json!({"name": "hook", "kind": "webhook", "url": url})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["channel"]["id"].as_str().expect("id").to_owned()
}

/// Opens `count` alerts and queues one delivery for each.
async fn queue_alerts(db: &TestDatabase, tenant: TenantId, count: usize, now: OffsetDateTime) {
    let mut tx = db.database().begin_tenant(tenant).await.expect("begin");
    let params = AlertParams::default();
    for index in 0..count {
        let id = AlertId::new();
        let key = format!("agent:{index}");
        alerts::insert_open(
            &mut tx,
            &NewAlert {
                id,
                kind: AlertKind::AgentOffline,
                dedupe_key: &key,
                agent_id: None,
                plan_id: None,
                repository_id: None,
                job_id: None,
                agent_hostname: None,
                plan_name: None,
                repository_name: None,
                params: &params,
                detail: "",
            },
            now,
        )
        .await
        .expect("open");
        alerts::enqueue(&mut tx, id, DeliveryEvent::Opened, now)
            .await
            .expect("enqueue");
    }
    tx.commit().await.expect("commit");
}

async fn run_pass(
    db: &TestDatabase,
    resolver: Arc<dyn Resolver>,
    tenant: TenantId,
    clock: Arc<dyn Clock>,
    deadline: Instant,
) -> Report {
    deliver::deliver_tenant(
        db.database(),
        &local_sender(None).with_resolver(resolver),
        test_kek().as_ref(),
        clock.as_ref(),
        tenant,
        "tenant",
        deadline,
    )
    .await
    .expect("pass")
}

/// Starts a pass that stops at its first lookup, runs `change` on the channel, then lets the pass
/// go on. The pass has claimed both deliveries before the change.
async fn pass_with_change(
    db: &TestDatabase,
    tenant: TenantId,
    change_sql: &'static str,
    origin: Option<String>,
) -> Report {
    let entered = Arc::new(Notify::new());
    let open = Arc::new(Notify::new());
    let resolver = Arc::new(GateResolver {
        first: AtomicBool::new(true),
        entered: entered.clone(),
        open: open.clone(),
    });
    let clock = ManualClock::new(OffsetDateTime::now_utc() + time::Duration::hours(3)).shared();
    let pass = {
        let (database, sender) = (
            db.database().clone(),
            local_sender(None).with_resolver(resolver),
        );
        tokio::spawn(async move {
            deliver::deliver_tenant(
                &database,
                &sender,
                test_kek().as_ref(),
                clock.as_ref(),
                tenant,
                "tenant",
                Instant::now() + Duration::from_secs(300),
            )
            .await
        })
    };
    entered.notified().await;
    let mut tx = scoped(db, tenant).await;
    let mut query = sqlx::query(change_sql);
    if let Some(origin) = origin {
        query = query.bind(origin);
    }
    query.execute(&mut *tx).await.expect("change the channel");
    tx.commit().await.expect("commit");
    open.notify_one();
    pass.await.expect("join").expect("pass")
}

/// AC-MN-07: a channel that the operator disables after the claim gets no send of the claim.
#[tokio::test]
async fn ac_mn_07_a_channel_disabled_after_the_claim_gets_no_send() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ac-mn-07-disable").await;
    let hook = WebhookSink::start().await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    webhook_channel(
        &app_for(&db),
        &cookie,
        format!("http://hooks.example.com:{}/hook", hook.port),
    )
    .await;
    queue_alerts(&db, tenant, 2, OffsetDateTime::now_utc()).await;

    let report = pass_with_change(
        &db,
        tenant,
        "UPDATE notification_channels SET enabled = false",
        None,
    )
    .await;

    // The first send is already past its re-read when the change lands; the second one is not.
    assert_eq!(report.sent, 1, "{report:?}");
    assert_eq!(report.failed, 1, "{report:?}");
    assert_eq!(hook.received().len(), 1, "no send to a disabled channel");
    db.cleanup().await;
}

/// AC-MN-07: a channel that the operator points to another URL after the claim gets no send to
/// the old URL.
#[tokio::test]
async fn ac_mn_07_a_channel_changed_after_the_claim_gets_no_send_to_the_old_url() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ac-mn-07-change").await;
    let old = WebhookSink::start().await;
    let new = WebhookSink::start().await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    webhook_channel(
        &app_for(&db),
        &cookie,
        format!("http://hooks.example.com:{}/hook", old.port),
    )
    .await;
    queue_alerts(&db, tenant, 2, OffsetDateTime::now_utc()).await;

    let report = pass_with_change(
        &db,
        tenant,
        "UPDATE notification_channels SET url_origin = $1",
        Some(format!("http://hooks.example.com:{}", new.port)),
    )
    .await;

    assert_eq!(report.sent, 2, "{report:?}");
    assert_eq!(old.received().len(), 1, "only the send that had started");
    assert_eq!(new.received().len(), 1, "the next send uses the new URL");
    db.cleanup().await;
}

/// AC-MN-07: a batch that does not fit the budget of the tenant counts an attempt only for the
/// sends that start, and the unsent rows go back due at once.
#[tokio::test]
async fn ac_mn_07_unsent_rows_of_a_spent_budget_keep_their_attempts() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ac-mn-07-budget").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    webhook_channel(
        &app_for(&db),
        &cookie,
        "http://hooks.example.com:9/hook".to_owned(),
    )
    .await;
    let now = OffsetDateTime::now_utc() + time::Duration::hours(3);
    queue_alerts(&db, tenant, 10, now).await;

    // One webhook send takes at most ten seconds, so the budget holds exactly one.
    let clock = ManualClock::new(now).shared();
    let report = run_pass(
        &db,
        Arc::new(HangingResolver),
        tenant,
        clock,
        Instant::now() + Duration::from_secs(11),
    )
    .await;
    assert_eq!(report.retried, 1, "{report:?}");
    assert_eq!(report.released, 9, "{report:?}");

    let mut tx = scoped(&db, tenant).await;
    let rows: Vec<(i32, bool)> = sqlx::query_as(
        "SELECT attempts, next_attempt_at <= $1 FROM alert_deliveries ORDER BY attempts DESC",
    )
    .bind(now)
    .fetch_all(&mut *tx)
    .await
    .expect("rows");
    tx.commit().await.expect("commit");
    assert_eq!(rows.iter().filter(|row| row.0 == 1).count(), 1, "{rows:?}");
    assert_eq!(rows.iter().filter(|row| row.0 == 0).count(), 9, "{rows:?}");
    assert!(
        rows.iter().all(|row| row.0 == 1 || row.1),
        "the unsent rows are due at once: {rows:?}"
    );
    db.cleanup().await;
}

/// FR-MN-01: one alert row with a job kind that this version does not know does not stop the
/// evaluation of the tenant.
#[tokio::test]
async fn fr_mn_01_one_unreadable_alert_row_does_not_stop_the_evaluation() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "fr-mn-01-tolerant").await;
    let now = OffsetDateTime::now_utc();
    queue_alerts(&db, tenant, 1, now).await;
    let mut tx = scoped(&db, tenant).await;
    sqlx::query("UPDATE alert_state SET params = '{\"job_kind\":\"xyz\"}'::jsonb")
        .execute(&mut *tx)
        .await
        .expect("params");
    tx.commit().await.expect("commit");
    let agent = seed_agent(db.database(), tenant, "silent-host").await;
    set_last_seen(&db, tenant, agent, Some(now - time::Duration::days(2))).await;

    let view = judged_view(&db, now).await;
    evaluate::evaluate_tenant(db.database(), tenant, now, &view)
        .await
        .expect("evaluation");

    let mut tx = scoped(&db, tenant).await;
    let (open,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM alert_state WHERE kind = 'agent_offline' AND agent_id = $1",
    )
    .bind(agent.as_uuid())
    .fetch_one(&mut *tx)
    .await
    .expect("count");
    tx.commit().await.expect("commit");
    assert_eq!(open, 1, "the offline agent got its alert");
    db.cleanup().await;
}
