//! The alert evaluation and the delivery of the messages (FR-MN-03, FR-MN-04, AC-MN-01, AC-MN-02,
//! AC-MN-07, ADR-0036).
//!
//! The tests drive the loop with explicit times, so no test sleeps. The channels point to a local
//! SMTP sink and a local webhook sink.

#![allow(clippy::expect_used)]

mod alert_support;
mod common;

use std::sync::Arc;

use alert_support::{
    HookRequest, SmtpSink, WebhookSink, backup_job, finish_job, local_sender, plan_created_at,
    scoped, set_last_seen,
};
use axum::Router;
use axum::http::StatusCode;
use restichub_cp::alerts::sender::Sender;
use restichub_cp::config::TenancyConfig;
use restichub_cp::server::{AppState, router};
use restichub_db::test_util::TestDatabase;
use restichub_db::{AgentId, PlanId, RepositoryId, TenantId};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};

use common::{
    create_tenant_with_key, request, seed_agent, seed_profile, seed_repository,
    seed_scheduled_plan, seed_secret, seed_session, send, test_kek,
};

struct Fixture {
    db: TestDatabase,
    tenant: TenantId,
    cookie: String,
    app: Router,
    sender: Sender,
    smtp: SmtpSink,
    hook: WebhookSink,
    plan: PlanId,
    agent: AgentId,
    repository: RepositoryId,
    profile: restichub_db::ProfileId,
    /// The creation time of the plan: the anchor of the test clock.
    t0: OffsetDateTime,
    signing_secret: String,
}

async fn fixture(name: &str) -> Option<Fixture> {
    let db = common::test_db().await?;
    let tenant = create_tenant_with_key(db.database(), name).await;
    let smtp = SmtpSink::start().await;
    let hook = WebhookSink::start().await;
    let sender = local_sender(Some(smtp.port));
    let app = router(
        AppState::new(db.database().clone())
            .with_kek(test_kek())
            .with_tenancy(TenancyConfig::multi())
            .with_sender(sender.clone()),
    );
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let secret = seed_secret(db.database(), tenant, "alert.password").await;
    let repository = seed_repository(db.database(), tenant, "alert-repo", secret).await;
    let profile = seed_profile(db.database(), tenant, "alert-profile", repository).await;
    let agent = seed_agent(db.database(), tenant, "alert-host").await;
    let plan = seed_scheduled_plan(
        db.database(),
        tenant,
        "nightly",
        profile,
        agent,
        "0 2 * * *",
        "UTC",
        restichub_db::MissedRunPolicy::RunAll,
        None,
    )
    .await;
    let t0 = plan_created_at(&db, tenant, plan).await;
    // The agent stays online unless a test says otherwise.
    set_last_seen(&db, tenant, agent, Some(t0 + Duration::days(30))).await;

    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/notification-channels",
            Some(&cookie),
            Some(json!({"name": "ops-mail", "kind": "email", "recipients": ["ops@example.com"]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/notification-channels",
            Some(&cookie),
            Some(json!({
                "name": "ops-hook",
                "kind": "webhook",
                "url": format!("http://127.0.0.1:{}/hook", hook.port),
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let signing_secret = created["signing_secret"]
        .as_str()
        .expect("the create response shows the secret")
        .to_owned();
    Some(Fixture {
        db,
        tenant,
        cookie,
        app,
        sender,
        smtp,
        hook,
        plan,
        agent,
        repository,
        profile,
        t0,
        signing_secret,
    })
}

impl Fixture {
    async fn run(&self, now: OffsetDateTime) {
        self.run_with(&self.sender, now).await;
    }

    async fn run_with(&self, sender: &Sender, now: OffsetDateTime) {
        let kek = test_kek();
        let clock = restichub_cp::clock::ManualClock::new(now).shared();
        restichub_cp::alerts::run_once(self.db.database(), sender, kek, clock)
            .await
            .expect("the alert pass works");
    }

    fn hooks(&self) -> Vec<HookRequest> {
        self.hook.received()
    }

    fn events(&self) -> Vec<(String, String)> {
        self.hooks()
            .iter()
            .map(|request| {
                let body = request.json();
                (
                    body["event"].as_str().unwrap_or_default().to_owned(),
                    body["alert"]["kind"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                )
            })
            .collect()
    }

    async fn finished_job(
        &self,
        plan: PlanId,
        fire_time: OffsetDateTime,
        status: &str,
        code: &str,
        message: Option<&str>,
        finished_at: OffsetDateTime,
    ) -> restichub_db::JobId {
        let job = backup_job(
            &self.db,
            self.tenant,
            plan,
            self.agent,
            self.repository,
            fire_time,
        )
        .await;
        finish_job(
            &self.db,
            self.tenant,
            job,
            status,
            code,
            message,
            finished_at,
        )
        .await;
        job
    }

    async fn put_rules(&self, body: Value) -> (StatusCode, Value) {
        let (status, _, response) = send(
            &self.app,
            request("PUT", "/api/v1/alert-rules", Some(&self.cookie), Some(body)),
        )
        .await;
        (status, response)
    }
}

/// AC-MN-01: a plan with no successful backup for the RPO sends an email and a webhook within five
/// minutes, and a successful backup sends a recovery notification.
#[tokio::test]
async fn ac_mn_01_rpo_violation_alerts_and_recovers() {
    let Some(fx) = fixture("ac-mn-01").await else {
        return;
    };
    let t0 = fx.t0;
    // The daily plan has the default objective of 26 hours.
    fx.run(t0 + Duration::hours(25)).await;
    assert!(
        fx.hooks().is_empty() && fx.smtp.received().is_empty(),
        "no alert before the RPO"
    );

    // The loop runs every 30 seconds; the violation begins at 26 hours.
    fx.run(t0 + Duration::hours(26) + Duration::minutes(4))
        .await;
    let hooks = fx.hooks();
    let mails = fx.smtp.received();
    assert_eq!(
        (hooks.len(), mails.len()),
        (1, 1),
        "one webhook and one email"
    );
    let body = hooks[0].json();
    assert_eq!(body["event"], "alert.opened");
    assert_eq!(body["alert"]["kind"], "rpo_violated");
    assert_eq!(body["alert"]["plan"]["name"], "nightly");
    assert_eq!(mails[0].to, vec!["ops@example.com".to_owned()]);
    assert!(
        mails[0]
            .header("subject")
            .expect("subject")
            .contains("Backup is overdue")
    );

    // The open alert sends nothing again (AC-MN-07).
    fx.run(t0 + Duration::hours(26) + Duration::minutes(10))
        .await;
    assert_eq!((fx.hooks().len(), fx.smtp.received().len()), (1, 1));

    // A successful backup resolves the alert and sends the recovery notification.
    fx.finished_job(
        fx.plan,
        t0 + Duration::hours(27),
        "succeeded",
        "succeeded",
        None,
        t0 + Duration::hours(27),
    )
    .await;
    fx.run(t0 + Duration::hours(27) + Duration::minutes(1))
        .await;
    let hooks = fx.hooks();
    assert_eq!(hooks.len(), 2);
    assert_eq!(hooks[1].json()["event"], "alert.resolved");
    let mails = fx.smtp.received();
    assert_eq!(mails.len(), 2);
    assert!(
        mails[1]
            .header("subject")
            .expect("subject")
            .contains("Resolved")
    );
    fx.run(t0 + Duration::hours(28)).await;
    assert_eq!((fx.hooks().len(), fx.smtp.received().len()), (2, 2));

    let (status, _, open) = send(
        &fx.app,
        request("GET", "/api/v1/alerts?status=all", Some(&fx.cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{open}");
    assert_eq!(open["items"][0]["status"], "resolved");

    // Each send is audited with no address and no secret.
    let (status, _, audit) = send(
        &fx.app,
        request("GET", "/api/v1/audit?limit=200", Some(&fx.cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{audit}");
    let sent: Vec<&Value> = audit["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|record| record["action"] == "alert_sent")
        .collect();
    assert_eq!(sent.len(), 4, "two events on two channels");
    for record in sent {
        let text = record.to_string();
        assert!(!text.contains("ops@example.com") && !text.contains(&fx.signing_secret));
    }
    assert!(!audit.to_string().contains(&fx.signing_secret));
}

/// AC-MN-01: an RPO that a plan sets itself replaces the default.
#[tokio::test]
async fn ac_mn_01_explicit_rpo_replaces_the_default() {
    let Some(fx) = fixture("ac-mn-01-explicit").await else {
        return;
    };
    let mut tx = scoped(&fx.db, fx.tenant).await;
    sqlx::query("UPDATE plans SET rpo_seconds = 3600 WHERE id = $1")
        .bind(fx.plan.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("set the objective");
    tx.commit().await.expect("commit");
    fx.run(fx.t0 + Duration::minutes(59)).await;
    assert!(fx.hooks().is_empty());
    fx.run(fx.t0 + Duration::minutes(61)).await;
    assert_eq!(
        fx.events(),
        vec![("alert.opened".to_owned(), "rpo_violated".to_owned())]
    );
}

/// AC-MN-02: the webhook keeps the stable error code and adds the words for it.
#[tokio::test]
async fn ac_mn_02_the_alert_translates_a_stable_error_code() {
    let Some(fx) = fixture("ac-mn-02-code").await else {
        return;
    };
    let t0 = fx.t0;
    let failed_at = t0 + Duration::minutes(30);
    fx.finished_job(
        fx.plan,
        t0 + Duration::minutes(10),
        "failed",
        "failed",
        Some("interrupted"),
        failed_at,
    )
    .await;
    fx.run(failed_at + Duration::minutes(1)).await;

    let hooks = fx.hooks();
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0].json()["alert"]["error_text"], "interrupted");
    assert_eq!(
        hooks[0].json()["alert"]["error_label"],
        "interrupted by an agent restart"
    );
    fx.db.cleanup().await;
}

/// AC-MN-02: a failed job sends an alert with the tenant, the agent, the plan, the error text and
/// the link to the job. An agent that is offline for long sends a separate alert.
#[tokio::test]
async fn ac_mn_02_job_failure_and_offline_agent_alert() {
    let Some(fx) = fixture("ac-mn-02").await else {
        return;
    };
    let t0 = fx.t0;
    let failed_at = t0 + Duration::minutes(30);
    let job = fx
        .finished_job(
            fx.plan,
            t0 + Duration::minutes(10),
            "failed",
            "failed",
            Some("restic exited with 1: repository password=hunter2 rejected"),
            failed_at,
        )
        .await;
    fx.run(failed_at + Duration::minutes(1)).await;

    let hooks = fx.hooks();
    assert_eq!(hooks.len(), 1);
    let body = hooks[0].json();
    assert_eq!(body["schema_version"], 1);
    assert_eq!(body["tenant"]["name"], "ac-mn-02");
    assert_eq!(body["alert"]["kind"], "job_failed");
    assert_eq!(body["alert"]["agent"]["name"], "alert-host");
    assert_eq!(body["alert"]["plan"]["name"], "nightly");
    assert_eq!(body["alert"]["repository"]["name"], "alert-repo");
    let message = body["alert"]["error_text"].as_str().expect("error text");
    assert!(
        message.contains("rejected"),
        "the error text is in the alert: {message}"
    );
    assert!(
        !message.contains("hunter2"),
        "the scrub hides the password: {message}"
    );
    assert_eq!(
        body["alert"]["message"], "The backup job failed (failed).",
        "the sentence comes from the typed values"
    );
    assert_eq!(body["alert"]["params"]["error_code"], "failed");
    assert_eq!(
        body["alert"]["url"],
        format!("{}/jobs/{job}", alert_support::ORIGIN)
    );
    let mails = fx.smtp.received();
    assert_eq!(mails.len(), 1);
    let text = mails[0].text();
    assert!(
        text.contains("Host: alert-host") && text.contains("Plan: nightly"),
        "{text}"
    );
    assert!(text.contains(&format!("/jobs/{job}")), "{text}");

    // An agent that was silent for longer than 30 minutes raises its own alert.
    set_last_seen(&fx.db, fx.tenant, fx.agent, Some(t0)).await;
    fx.run(t0 + Duration::minutes(31)).await;
    let events = fx.events();
    assert_eq!(events.len(), 2, "{events:?}");
    assert_eq!(
        events[1],
        ("alert.opened".to_owned(), "agent_offline".to_owned())
    );

    // The agent comes back: a recovery notification for the offline alert only.
    set_last_seen(
        &fx.db,
        fx.tenant,
        fx.agent,
        Some(t0 + Duration::minutes(40)),
    )
    .await;
    fx.run(t0 + Duration::minutes(41)).await;
    let events = fx.events();
    assert_eq!(events.len(), 3, "{events:?}");
    assert_eq!(
        events[2],
        ("alert.resolved".to_owned(), "agent_offline".to_owned())
    );
}

/// AC-MN-07: a failure that stays open sends no repeat, and a later success resolves it.
#[tokio::test]
async fn ac_mn_07_dedupe_and_recovery_of_a_failed_job() {
    let Some(fx) = fixture("ac-mn-07-dedupe").await else {
        return;
    };
    let t0 = fx.t0;
    fx.finished_job(
        fx.plan,
        t0 + Duration::minutes(1),
        "failed",
        "failed",
        Some("boom"),
        t0 + Duration::minutes(2),
    )
    .await;
    fx.run(t0 + Duration::minutes(3)).await;
    fx.run(t0 + Duration::minutes(4)).await;
    assert_eq!(fx.hooks().len(), 1, "the second pass sends nothing");

    // A second failure of the same plan stays under the open alert.
    fx.finished_job(
        fx.plan,
        t0 + Duration::minutes(10),
        "failed",
        "failed",
        Some("boom again"),
        t0 + Duration::minutes(11),
    )
    .await;
    fx.run(t0 + Duration::minutes(12)).await;
    assert_eq!(fx.hooks().len(), 1, "no repeat while the alert is open");

    fx.finished_job(
        fx.plan,
        t0 + Duration::minutes(20),
        "succeeded",
        "succeeded",
        None,
        t0 + Duration::minutes(21),
    )
    .await;
    fx.run(t0 + Duration::minutes(22)).await;
    assert_eq!(
        fx.events(),
        vec![
            ("alert.opened".to_owned(), "job_failed".to_owned()),
            ("alert.resolved".to_owned(), "job_failed".to_owned())
        ]
    );

    fx.finished_job(
        fx.plan,
        t0 + Duration::minutes(30),
        "failed",
        "failed",
        Some("third"),
        t0 + Duration::minutes(31),
    )
    .await;
    fx.run(t0 + Duration::minutes(32)).await;
    assert_eq!(fx.hooks().len(), 3);
}

/// AC-MN-07: a dead letter job and a job with warnings raise their own kinds, and the settings
/// switch a kind off.
#[tokio::test]
async fn ac_mn_07_dead_letter_warnings_and_rule_toggles() {
    let Some(fx) = fixture("ac-mn-07-kinds").await else {
        return;
    };
    let t0 = fx.t0;
    let second = seed_scheduled_plan(
        fx.db.database(),
        fx.tenant,
        "weekly",
        fx.profile,
        fx.agent,
        "0 3 * * 1",
        "UTC",
        restichub_db::MissedRunPolicy::Skip,
        None,
    )
    .await;
    fx.finished_job(
        fx.plan,
        t0 + Duration::minutes(1),
        "dead_letter",
        "repository_locked",
        None,
        t0 + Duration::minutes(2),
    )
    .await;
    // Warnings are off by default.
    fx.finished_job(
        second,
        t0 + Duration::minutes(1),
        "succeeded",
        "succeeded_with_warnings",
        None,
        t0 + Duration::minutes(2),
    )
    .await;
    fx.run(t0 + Duration::minutes(3)).await;
    assert_eq!(
        fx.events(),
        vec![("alert.opened".to_owned(), "job_dead_letter".to_owned())]
    );

    let (status, rules) = fx
        .put_rules(json!({"items": [{"kind": "job_warnings", "enabled": true}]}))
        .await;
    assert_eq!(status, StatusCode::OK, "{rules}");
    fx.run(t0 + Duration::minutes(4)).await;
    assert_eq!(fx.events().len(), 2);
    assert_eq!(fx.events()[1].1, "job_warnings");

    // A rule that is switched off resolves its open alert without a message.
    let (status, _) = fx
        .put_rules(json!({"items": [{"kind": "job_dead_letter", "enabled": false}]}))
        .await;
    assert_eq!(status, StatusCode::OK);
    fx.run(t0 + Duration::minutes(5)).await;
    assert_eq!(
        fx.hooks().len(),
        2,
        "a disabled rule sends no recovery message"
    );

    let (status, _) = fx
        .put_rules(
            json!({"items": [{"kind": "job_failed", "enabled": true, "threshold_minutes": 5}]}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = fx
        .put_rules(
            json!({"items": [{"kind": "agent_offline", "enabled": true, "threshold_minutes": 0}]}),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = fx
        .put_rules(
            json!({"items": [{"kind": "agent_offline", "enabled": true, "threshold_minutes": 90}]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|rule| rule["kind"] == "agent_offline" && rule["threshold_minutes"] == 90)
    );
}

/// AC-MN-02: a repository lock that stays stale raises an alert, and its end sends a recovery.
#[tokio::test]
async fn ac_mn_02_stale_lock_alert() {
    let Some(fx) = fixture("ac-mn-02-lock").await else {
        return;
    };
    let t0 = fx.t0;
    let job = common::seed_job(fx.db.database(), fx.tenant, fx.agent, fx.repository).await;
    let mut tx = scoped(&fx.db, fx.tenant).await;
    sqlx::query("UPDATE jobs SET stale_lock_since = $2 WHERE id = $1")
        .bind(job.as_uuid())
        .bind(t0)
        .execute(&mut *tx)
        .await
        .expect("mark the lock");
    tx.commit().await.expect("commit");
    fx.run(t0 + Duration::minutes(30)).await;
    assert!(
        fx.hooks().is_empty(),
        "the lock is stale for 30 minutes only"
    );
    fx.run(t0 + Duration::minutes(61)).await;
    assert_eq!(
        fx.events(),
        vec![("alert.opened".to_owned(), "stale_lock".to_owned())]
    );

    let mut tx = scoped(&fx.db, fx.tenant).await;
    sqlx::query("UPDATE jobs SET stale_lock_since = NULL WHERE id = $1")
        .bind(job.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("clear the lock");
    tx.commit().await.expect("commit");
    fx.run(t0 + Duration::minutes(62)).await;
    assert_eq!(
        fx.events()[1],
        ("alert.resolved".to_owned(), "stale_lock".to_owned())
    );
}

/// AC-MN-07: two control plane instances send each message once.
#[tokio::test]
async fn ac_mn_07_two_instances_send_once() {
    let Some(fx) = fixture("ac-mn-07-instances").await else {
        return;
    };
    let t0 = fx.t0;
    let other = local_sender(Some(fx.smtp.port));
    let mut plans = vec![fx.plan];
    for index in 0..5 {
        plans.push(
            seed_scheduled_plan(
                fx.db.database(),
                fx.tenant,
                &format!("extra-{index}"),
                fx.profile,
                fx.agent,
                "0 4 * * *",
                "UTC",
                restichub_db::MissedRunPolicy::Skip,
                None,
            )
            .await,
        );
    }
    for (index, plan) in plans.iter().enumerate() {
        fx.finished_job(
            *plan,
            t0 + Duration::minutes(1),
            "failed",
            "failed",
            Some("boom"),
            t0 + Duration::minutes(2 + index as i64),
        )
        .await;
    }
    let (first, second) = (&fx.sender, &other);
    for round in 0..3 {
        let now = t0 + Duration::minutes(10 + round);
        tokio::join!(fx.run_with(first, now), fx.run_with(second, now));
    }
    assert_eq!(fx.hooks().len(), plans.len(), "one webhook for each alert");
    assert_eq!(
        fx.smtp.received().len(),
        plans.len(),
        "one email for each alert"
    );
    let mut ids: Vec<String> = fx
        .hooks()
        .iter()
        .map(|request| request.json()["id"].as_str().unwrap_or_default().to_owned())
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), plans.len(), "each delivery has its own id");
}

/// AC-MN-07: a target that fails is retried with a backoff and the delivery ends as failed after
/// the fifth attempt.
#[tokio::test]
async fn ac_mn_07_failed_delivery_retries_then_stops() {
    let Some(fx) = fixture("ac-mn-07-retry").await else {
        return;
    };
    let t0 = fx.t0;
    fx.hook.answer_with(500);
    fx.finished_job(
        fx.plan,
        t0 + Duration::minutes(1),
        "failed",
        "failed",
        Some("boom"),
        t0 + Duration::minutes(2),
    )
    .await;
    let mut now = t0 + Duration::minutes(3);
    fx.run(now).await;
    assert_eq!(fx.hooks().len(), 1);
    fx.run(now + Duration::seconds(10)).await;
    assert_eq!(fx.hooks().len(), 1, "the backoff holds the retry");
    for (attempt, wait) in [(2, 31), (3, 130), (4, 610), (5, 3610)] {
        now += Duration::seconds(wait);
        fx.run(now).await;
        assert_eq!(fx.hooks().len(), attempt, "attempt {attempt}");
    }
    fx.run(now + Duration::days(1)).await;
    assert_eq!(fx.hooks().len(), 5, "no sixth attempt");

    let mut tx = fx
        .db
        .database()
        .begin_tenant(fx.tenant)
        .await
        .expect("begin");
    let counts = restichub_db::alerts::delivery_counts(&mut tx)
        .await
        .expect("counts");
    tx.commit().await.expect("commit");
    assert!(
        counts.iter().any(|(status, count)| *status
            == restichub_db::alerts::DeliveryStatus::Failed
            && *count == 1),
        "{counts:?}"
    );
    assert_eq!(fx.smtp.received().len(), 1);
}

/// A suspended tenant gets no evaluation (AC-TN-03).
#[tokio::test]
async fn a_suspended_tenant_gets_no_alert() {
    let Some(fx) = fixture("alerts-suspended").await else {
        return;
    };
    common::set_tenant_status(fx.db.database(), fx.tenant, "suspended").await;
    fx.run(fx.t0 + Duration::hours(40)).await;
    assert!(fx.hooks().is_empty());
}

/// The signing secret of a webhook verifies the signature of the payload (AC-MN-06).
#[tokio::test]
async fn ac_mn_06_webhook_signature_verifies() {
    let Some(fx) = fixture("ac-mn-06").await else {
        return;
    };
    fx.finished_job(
        fx.plan,
        fx.t0 + Duration::minutes(1),
        "failed",
        "failed",
        Some("boom"),
        fx.t0 + Duration::minutes(2),
    )
    .await;
    fx.run(fx.t0 + Duration::minutes(3)).await;
    let hooks = fx.hooks();
    let request = &hooks[0];
    let timestamp: i64 = request
        .header("x-restichub-timestamp")
        .expect("timestamp")
        .parse()
        .expect("number");
    let signature = request.header("x-restichub-signature").expect("signature");
    assert!(restichub_cp::alerts::signature::verify(
        fx.signing_secret.as_bytes(),
        timestamp,
        &request.body,
        signature
    ));
    assert!(!restichub_cp::alerts::signature::verify(
        b"another-secret",
        timestamp,
        &request.body,
        signature
    ));
    assert!(request.request_line.starts_with("POST /hook HTTP/1.1"));
    assert_eq!(
        request.header("x-restichub-delivery"),
        request.json()["id"].as_str()
    );
    let _ = Arc::strong_count(&fx.hook.requests);
}

/// FR-MN-04: the language setting of the installation selects the text of an email and of a
/// webhook message. The error text stays a separate field.
#[tokio::test]
async fn ac_mn_02_messages_follow_the_notification_language() {
    let Some(fx) = fixture("ac-mn-02-ru").await else {
        return;
    };
    let t0 = fx.t0;
    let failed_at = t0 + Duration::minutes(30);
    fx.finished_job(
        fx.plan,
        t0 + Duration::minutes(10),
        "failed",
        "failed",
        Some("restic exited with 1"),
        failed_at,
    )
    .await;
    let mut settings = alert_support::local_settings(
        Some(fx.smtp.port),
        restichub_cp::alerts::settings::SmtpSecurity::None,
    );
    settings.language = restichub_cp::alerts::text::Language::Ru;
    let russian = Sender::new(Arc::new(settings)).expect("sender");
    fx.run_with(&russian, failed_at + Duration::minutes(1))
        .await;

    let body = fx.hooks()[0].json();
    assert_eq!(
        body["alert"]["message"],
        "Задача «резервное копирование» завершилась ошибкой (failed)."
    );
    assert_eq!(body["alert"]["error_text"], "restic exited with 1");
    let mails = fx.smtp.received();
    let subject = mails[0].header("subject").expect("subject");
    assert!(subject.starts_with("=?UTF-8?B?"), "{subject}");
    let text = mails[0].text();
    assert!(text.contains("Хост: alert-host"), "{text}");
    assert!(text.contains("Ошибка: restic exited with 1"), "{text}");
}
