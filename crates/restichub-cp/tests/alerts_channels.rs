//! The notification channel API: validation, the secret handling, the address policy, the roles and
//! the tenant isolation (FR-MN-04, AC-MN-05, AC-MN-06, AC-MN-08, AC-TN-01, ADR-0035).

#![allow(clippy::expect_used)]

mod alert_support;
mod common;

use std::net::IpAddr;
use std::sync::Arc;

use alert_support::{
    FixedResolver, SmtpSink, WebhookSink, local_sender, local_settings, scoped, tls_pair,
};
use axum::Router;
use axum::http::StatusCode;
use restichub_cp::alerts::sender::Sender;
use restichub_cp::alerts::settings::{AlertsSettings, SmtpSecurity};
use restichub_cp::config::TenancyConfig;
use restichub_cp::server::{AppState, router};
use restichub_db::test_util::TestDatabase;
use serde_json::{Value, json};

use common::{create_tenant_with_key, request, seed_session, send, test_kek};

fn app_with(db: &TestDatabase, sender: Sender) -> Router {
    router(
        AppState::new(db.database().clone())
            .with_kek(test_kek())
            .with_tenancy(TenancyConfig::multi())
            .with_sender(sender),
    )
}

/// A sender with the default policy: HTTPS only, no private address.
fn strict_sender() -> Sender {
    Sender::new(Arc::new(AlertsSettings::default())).expect("sender")
}

async fn create_channel(app: &Router, cookie: &str, body: Value) -> (StatusCode, Value) {
    let (status, _, response) = send(
        app,
        request(
            "POST",
            "/api/v1/notification-channels",
            Some(cookie),
            Some(body),
        ),
    )
    .await;
    (status, response)
}

async fn test_send(app: &Router, cookie: &str, id: &str) -> Value {
    let (status, _, response) = send(
        app,
        request(
            "POST",
            &format!("/api/v1/notification-channels/{id}/test"),
            Some(cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    response
}

/// AC-MN-05: the control plane refuses a loopback, private, link-local or metadata address in the
/// URL of a webhook, and plain HTTP.
#[tokio::test]
async fn ac_mn_05_webhook_url_policy_at_the_api() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ac-mn-05-api").await;
    let app = app_with(&db, strict_sender());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    for (index, url) in [
        "https://127.0.0.1/hook",
        "https://10.1.2.3/hook",
        "https://192.168.0.9/hook",
        "https://169.254.169.254/latest/meta-data",
        "https://[::1]/hook",
        "https://[fd00::1]/hook",
        "https://[::ffff:127.0.0.1]/hook",
        "http://hooks.example.com/hook",
        "https://user:pass@hooks.example.com/hook",
        "ftp://hooks.example.com/hook",
        "not a url",
    ]
    .iter()
    .enumerate()
    {
        let (status, response) = create_channel(
            &app,
            &cookie,
            json!({"name": format!("bad-{index}"), "kind": "webhook", "url": url}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{url}: {response}");
        assert_eq!(response["field"], "url", "{url}: {response}");
    }
    let (status, _) = create_channel(
        &app,
        &cookie,
        json!({"name": "good", "kind": "webhook", "url": "https://hooks.example.com/in/token123"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

/// AC-MN-05: a host name that resolves to a private address is refused at the send, and the target
/// sees no request. The same check protects the test button.
#[tokio::test]
async fn ac_mn_05_dns_name_to_a_private_address_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ac-mn-05-dns").await;
    let hook = WebhookSink::start().await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    for (index, address) in ["127.0.0.1", "10.0.0.5", "169.254.169.254", "::1"]
        .into_iter()
        .enumerate()
    {
        let mut settings = AlertsSettings::default();
        settings.webhook.allow_http = true;
        let sender = Sender::new(Arc::new(settings))
            .expect("sender")
            .with_resolver(Arc::new(FixedResolver(
                address.parse::<IpAddr>().expect("ip"),
            )));
        let app = app_with(&db, sender);
        let (status, created) = create_channel(
            &app,
            &cookie,
            json!({
                "name": format!("dns-{index}"),
                "kind": "webhook",
                "url": format!("http://rebind.example.com:{}/hook", hook.port),
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["channel"]["id"].as_str().expect("id");
        let result = test_send(&app, &cookie, id).await;
        assert_eq!(result["ok"], false, "{address}: {result}");
        assert_eq!(result["error"], "blocked_address", "{address}: {result}");
    }
    assert!(hook.received().is_empty(), "the target saw no request");
}

/// AC-MN-05: the installation allow-list admits a private network.
#[tokio::test]
async fn ac_mn_05_allow_list_admits_a_network() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ac-mn-05-allow").await;
    let hook = WebhookSink::start().await;
    let app = app_with(&db, local_sender(None));
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let (status, created) = create_channel(
        &app,
        &cookie,
        json!({"name": "local", "kind": "webhook", "url": format!("http://127.0.0.1:{}/x", hook.port)}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["channel"]["id"].as_str().expect("id");
    let result = test_send(&app, &cookie, id).await;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(hook.received().len(), 1);
    let body = hook.received()[0].json();
    assert_eq!(body["event"], "test");
    assert!(body["alert"].is_null());
}

/// AC-MN-06: a webhook over TLS reaches a host name that the resolver pins to the sink, and the
/// signature verifies with the secret of the create response.
#[tokio::test]
async fn ac_mn_06_webhook_over_tls() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ac-mn-06-tls").await;
    let (server, client) = tls_pair("hooks.example.com");
    let hook = WebhookSink::start_tls(server).await;
    let mut settings = local_settings(None, SmtpSecurity::None);
    settings.webhook.allow_http = false;
    let sender = Sender::new(Arc::new(settings))
        .expect("sender")
        .with_tls(client)
        .with_resolver(Arc::new(FixedResolver("127.0.0.1".parse().expect("ip"))));
    let app = app_with(&db, sender);
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let (status, created) = create_channel(
        &app,
        &cookie,
        json!({
            "name": "tls-hook",
            "kind": "webhook",
            "url": format!("https://hooks.example.com:{}/in?x=1", hook.port),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let secret = created["signing_secret"]
        .as_str()
        .expect("secret")
        .to_owned();
    assert!(secret.starts_with("whsec_"));
    let id = created["channel"]["id"].as_str().expect("id");
    let result = test_send(&app, &cookie, id).await;
    assert_eq!(result["ok"], true, "{result}");
    let received = hook.received();
    assert_eq!(received.len(), 1);
    assert!(
        received[0]
            .request_line
            .starts_with("POST /in?x=1 HTTP/1.1")
    );
    let timestamp: i64 = received[0]
        .header("x-restichub-timestamp")
        .expect("timestamp")
        .parse()
        .expect("number");
    assert!(restichub_cp::alerts::signature::verify(
        secret.as_bytes(),
        timestamp,
        &received[0].body,
        received[0]
            .header("x-restichub-signature")
            .expect("signature"),
    ));
}

/// A redirect is a failure, not a hop (ADR-0035).
#[tokio::test]
async fn ac_mn_05_a_redirect_is_not_followed() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ac-mn-05-redirect").await;
    let hook = WebhookSink::start().await;
    hook.answer_with(302);
    let app = app_with(&db, local_sender(None));
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let (_, created) = create_channel(
        &app,
        &cookie,
        json!({"name": "redirect", "kind": "webhook", "url": format!("http://127.0.0.1:{}/x", hook.port)}),
    )
    .await;
    let id = created["channel"]["id"].as_str().expect("id");
    let result = test_send(&app, &cookie, id).await;
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"], "http_302");
    assert_eq!(hook.received().len(), 1, "no second request");
}

/// The email channel sends through the SMTP server of the installation, also with TLS from the
/// first byte (FR-MN-04).
#[tokio::test]
async fn ac_mn_01_email_channel_over_tls_and_without_smtp() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ac-mn-01-mail").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let (server, client) = tls_pair("127.0.0.1");
    let sink = SmtpSink::start_tls(server).await;
    let sender = Sender::new(Arc::new(local_settings(Some(sink.port), SmtpSecurity::Tls)))
        .expect("sender")
        .with_tls(client);
    let app = app_with(&db, sender);
    let (status, created) = create_channel(
        &app,
        &cookie,
        json!({"name": "mail", "kind": "email", "recipients": ["Ops@Example.com", "ops@example.com", "dev@example.com"]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        created["channel"]["recipients"],
        json!(["ops@example.com", "dev@example.com"])
    );
    assert!(created["signing_secret"].is_null());
    let id = created["channel"]["id"].as_str().expect("id");
    let result = test_send(&app, &cookie, id).await;
    assert_eq!(result["ok"], true, "{result}");
    let mails = sink.received();
    assert_eq!(mails.len(), 1);
    assert_eq!(mails[0].from, "restichub@backup.example.com");
    assert_eq!(mails[0].to.len(), 2);
    assert!(mails[0].text().contains("test message"));

    // An installation with no SMTP server reports it on the test button and in the list.
    let bare = app_with(&db, strict_sender());
    let result = test_send(&bare, &cookie, id).await;
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"], "smtp_not_configured");
    let (_, _, list) = send(
        &bare,
        request("GET", "/api/v1/notification-channels", Some(&cookie), None),
    )
    .await;
    assert_eq!(list["email_configured"], false);
}

/// AC-MN-08: the signing secret shows once; no read, audit record, delivery row or log field holds
/// it. A rotation gives a new value, and a delete removes the secret row.
#[tokio::test]
async fn ac_mn_08_secrets_stay_out_of_the_api_and_the_audit() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "ac-mn-08").await;
    let hook = WebhookSink::start().await;
    let app = app_with(&db, local_sender(None));
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let secrets_before = common::count(db.database(), tenant, "secrets").await;
    let path_token = "PATH-TOKEN-9f3a";
    let (status, created) = create_channel(
        &app,
        &cookie,
        json!({
            "name": "hook",
            "kind": "webhook",
            "url": format!("http://127.0.0.1:{}/{path_token}", hook.port),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let first = created["signing_secret"]
        .as_str()
        .expect("secret")
        .to_owned();
    let id = created["channel"]["id"].as_str().expect("id").to_owned();
    assert_eq!(
        created["channel"]["url_origin"],
        format!("http://127.0.0.1:{}", hook.port)
    );
    assert_eq!(
        common::count(db.database(), tenant, "secrets").await,
        secrets_before + 1
    );

    let (_, _, rotated) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/notification-channels/{id}"),
            Some(&cookie),
            Some(json!({"rotate_secret": true})),
        ),
    )
    .await;
    let second = rotated["signing_secret"]
        .as_str()
        .expect("new secret")
        .to_owned();
    assert_ne!(first, second);
    // The rotation changes the key that signs the next message.
    let result = test_send(&app, &cookie, &id).await;
    assert_eq!(result["ok"], true);
    let request_seen = hook.received().remove(0);
    let timestamp: i64 = request_seen
        .header("x-restichub-timestamp")
        .expect("ts")
        .parse()
        .expect("n");
    let header = request_seen
        .header("x-restichub-signature")
        .expect("signature");
    assert!(restichub_cp::alerts::signature::verify(
        second.as_bytes(),
        timestamp,
        &request_seen.body,
        header
    ));
    assert!(!restichub_cp::alerts::signature::verify(
        first.as_bytes(),
        timestamp,
        &request_seen.body,
        header
    ));

    let mut everything = String::new();
    for uri in [
        "/api/v1/notification-channels".to_owned(),
        format!("/api/v1/notification-channels/{id}"),
        "/api/v1/secrets".to_owned(),
        "/api/v1/audit".to_owned(),
        "/api/v1/alerts?status=all".to_owned(),
    ] {
        let (status, _, body) = send(&app, request("GET", &uri, Some(&cookie), None)).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        everything.push_str(&body.to_string());
    }
    let mut tx = scoped(&db, tenant).await;
    for table in [
        "audit_log",
        "alert_deliveries",
        "alert_state",
        "notification_channels",
    ] {
        let rows: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT row_to_json(t)::text FROM {table} t"
        )))
        .fetch_all(&mut *tx)
        .await
        .expect("rows");
        for (row,) in rows {
            everything.push_str(&row);
        }
    }
    tx.commit().await.expect("commit");
    for secret in [&first, &second] {
        assert!(!everything.contains(secret.as_str()), "a secret leaked");
    }
    // The path of the URL is write-only, in the reads and in the audit records.
    let audit_only: String = {
        let (_, _, audit) = send(&app, request("GET", "/api/v1/audit", Some(&cookie), None)).await;
        audit.to_string()
    };
    assert!(
        !audit_only.contains(path_token),
        "the audit holds the URL path"
    );
    for action in ["channel_create", "channel_update", "alert_sent"] {
        assert!(audit_only.contains(action), "{action} is audited");
    }

    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/notification-channels/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        common::count(db.database(), tenant, "secrets").await,
        secrets_before,
        "the secret row goes with the channel"
    );
    let (_, _, audit) = send(&app, request("GET", "/api/v1/audit", Some(&cookie), None)).await;
    assert!(audit.to_string().contains("channel_delete"));
}

/// The validation of the channel fields, the unique name and the limit.
#[tokio::test]
async fn channel_validation_and_limit() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "channel-validation").await;
    let app = app_with(&db, local_sender(None));
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    for (body, field) in [
        (
            json!({"name": "m", "kind": "email", "recipients": []}),
            "recipients",
        ),
        (
            json!({"name": "m", "kind": "email", "recipients": ["not-an-address"]}),
            "recipients",
        ),
        (
            json!({"name": "m", "kind": "email", "recipients": ["a@example.com"], "url": "https://x.example.com"}),
            "url",
        ),
        (json!({"name": "m", "kind": "webhook"}), "url"),
        (
            json!({"name": "m", "kind": "webhook", "url": "https://x.example.com", "recipients": ["a@example.com"]}),
            "recipients",
        ),
        (
            json!({"name": "", "kind": "email", "recipients": ["a@example.com"]}),
            "name",
        ),
    ] {
        let (status, response) = create_channel(&app, &cookie, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {response}");
        assert_eq!(response["field"], field, "{body}: {response}");
    }
    let many: Vec<String> = (0..21).map(|i| format!("u{i}@example.com")).collect();
    let (status, _) = create_channel(
        &app,
        &cookie,
        json!({"name": "many", "kind": "email", "recipients": many}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = create_channel(
        &app,
        &cookie,
        json!({"name": "dup", "kind": "email", "recipients": ["a@example.com"]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = create_channel(
        &app,
        &cookie,
        json!({"name": "dup", "kind": "email", "recipients": ["a@example.com"]}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // An update checks the kind: a webhook takes no recipients.
    let (_, created) = create_channel(
        &app,
        &cookie,
        json!({"name": "hook", "kind": "webhook", "url": "http://127.0.0.1:9/x"}),
    )
    .await;
    let id = created["channel"]["id"].as_str().expect("id");
    let (status, _, _) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/notification-channels/{id}"),
            Some(&cookie),
            Some(json!({"recipients": ["a@example.com"]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, updated) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/notification-channels/{id}"),
            Some(&cookie),
            Some(json!({"enabled": false, "name": "hook-off"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["channel"]["enabled"], false);
    assert_eq!(updated["channel"]["name"], "hook-off");
    assert!(updated["signing_secret"].is_null());

    for index in 0..48 {
        let (status, _) = create_channel(
            &app,
            &cookie,
            json!({"name": format!("fill-{index}"), "kind": "email", "recipients": ["a@example.com"]}),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "channel {index}");
    }
    let (status, _) = create_channel(
        &app,
        &cookie,
        json!({"name": "over", "kind": "email", "recipients": ["a@example.com"]}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "the tenant holds at most 50 channels"
    );
}

#[tokio::test]
async fn alert_role_matrix() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "alert-roles").await;
    let app = app_with(&db, local_sender(None));
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let operator = seed_session(db.database(), tenant, "operator", "operator").await;
    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let (_, created) = create_channel(
        &app,
        &admin,
        json!({"name": "m", "kind": "email", "recipients": ["a@example.com"]}),
    )
    .await;
    let id = created["channel"]["id"].as_str().expect("id").to_owned();
    let channel_uri = format!("/api/v1/notification-channels/{id}");

    let cases: Vec<(&str, String, Option<Value>, [StatusCode; 3])> = vec![
        // Admin, Operator, Viewer
        (
            "GET",
            "/api/v1/notification-channels".into(),
            None,
            [StatusCode::OK, StatusCode::FORBIDDEN, StatusCode::FORBIDDEN],
        ),
        (
            "GET",
            channel_uri.clone(),
            None,
            [StatusCode::OK, StatusCode::FORBIDDEN, StatusCode::FORBIDDEN],
        ),
        (
            "POST",
            "/api/v1/notification-channels".into(),
            Some(json!({"name": "x", "kind": "email", "recipients": ["b@example.com"]})),
            [
                StatusCode::CREATED,
                StatusCode::FORBIDDEN,
                StatusCode::FORBIDDEN,
            ],
        ),
        (
            "PATCH",
            channel_uri.clone(),
            Some(json!({"enabled": true})),
            [StatusCode::OK, StatusCode::FORBIDDEN, StatusCode::FORBIDDEN],
        ),
        (
            "POST",
            format!("{channel_uri}/test"),
            None,
            [StatusCode::OK, StatusCode::FORBIDDEN, StatusCode::FORBIDDEN],
        ),
        (
            "GET",
            "/api/v1/alerts".into(),
            None,
            [StatusCode::OK, StatusCode::OK, StatusCode::OK],
        ),
        (
            "GET",
            "/api/v1/alert-rules".into(),
            None,
            [StatusCode::OK, StatusCode::OK, StatusCode::OK],
        ),
        (
            "PUT",
            "/api/v1/alert-rules".into(),
            Some(json!({"items": []})),
            [StatusCode::OK, StatusCode::FORBIDDEN, StatusCode::FORBIDDEN],
        ),
        (
            "GET",
            "/api/v1/dashboard".into(),
            None,
            [StatusCode::OK, StatusCode::OK, StatusCode::OK],
        ),
        (
            "DELETE",
            channel_uri.clone(),
            None,
            [
                StatusCode::NO_CONTENT,
                StatusCode::FORBIDDEN,
                StatusCode::FORBIDDEN,
            ],
        ),
    ];
    // The Operator and the Viewer go first, so the Admin delete comes last.
    for (method, uri, body, expected) in &cases {
        for (index, cookie) in [&operator, &viewer].into_iter().enumerate() {
            let (status, _, response) =
                send(&app, request(method, uri, Some(cookie), body.clone())).await;
            assert_eq!(
                status,
                expected[index + 1],
                "{method} {uri} as {}: {response}",
                ["operator", "viewer"][index]
            );
        }
    }
    for (method, uri, body, expected) in &cases {
        let (status, _, response) =
            send(&app, request(method, uri, Some(&admin), body.clone())).await;
        assert_eq!(status, expected[0], "{method} {uri} as admin: {response}");
    }
    for (method, uri) in [
        ("GET", "/api/v1/alerts"),
        ("GET", "/api/v1/dashboard"),
        ("GET", "/api/v1/notification-channels"),
    ] {
        let (status, _, _) = send(&app, request(method, uri, None, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }
}

/// AC-TN-01: a channel of another tenant gives HTTP 404, and the alert list shows own rows only.
#[tokio::test]
async fn ac_tn_01_channels_and_alerts_are_tenant_scoped() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant_a = create_tenant_with_key(db.database(), "alerts-tenant-a").await;
    let tenant_b = create_tenant_with_key(db.database(), "alerts-tenant-b").await;
    let app = app_with(&db, local_sender(None));
    let admin_a = seed_session(db.database(), tenant_a, "admin-a", "admin").await;
    let admin_b = seed_session(db.database(), tenant_b, "admin-b", "admin").await;
    let (_, created) = create_channel(
        &app,
        &admin_a,
        json!({"name": "m", "kind": "email", "recipients": ["a@example.com"]}),
    )
    .await;
    let id = created["channel"]["id"].as_str().expect("id");
    let uri = format!("/api/v1/notification-channels/{id}");
    for (method, path, body) in [
        ("GET", uri.clone(), None),
        ("PATCH", uri.clone(), Some(json!({"enabled": false}))),
        ("DELETE", uri.clone(), None),
        ("POST", format!("{uri}/test"), None),
    ] {
        let (status, _, response) = send(&app, request(method, &path, Some(&admin_b), body)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}: {response}");
    }
    let (_, _, list) = send(
        &app,
        request("GET", "/api/v1/notification-channels", Some(&admin_b), None),
    )
    .await;
    assert_eq!(list["items"].as_array().expect("items").len(), 0);
    let (status, _, _) = send(&app, request("GET", &uri, Some(&admin_a), None)).await;
    assert_eq!(status, StatusCode::OK, "the owner still reads the channel");
}
