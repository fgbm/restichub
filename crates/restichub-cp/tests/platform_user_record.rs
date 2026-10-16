//! The record endpoints of one platform account and the Platform Admin flag of the organization
//! user list (ADR-0028, ADR-0029).

mod common;

use axum::Router;
use axum::http::StatusCode;
use restichub_db::PLATFORM_TENANT_ID;
use serde_json::{Value, json};

use common::{app_with_kek, create_tenant, request, seed_session, send};

async fn platform_admin(db: &restichub_db::Database) -> String {
    seed_session(db, PLATFORM_TENANT_ID, "root", "platform_admin").await
}

/// Creates an account through the platform API and returns its identifier.
async fn create_account(app: &Router, cookie: &str, body: Value) -> String {
    let (status, _, account) = send(
        app,
        request("POST", "/api/v1/platform/users", Some(cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    account
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

async fn reset(app: &Router, cookie: &str, id: &str, password: &str) -> (StatusCode, Value) {
    let (status, _, body) = send(
        app,
        request(
            "POST",
            &format!("/api/v1/platform/users/{id}/password"),
            Some(cookie),
            Some(json!({"new_password": password})),
        ),
    )
    .await;
    (status, body)
}

async fn sign_in(app: &Router, username: &str, password: &str) -> (StatusCode, Option<String>) {
    let body = json!({"username": username, "password": password});
    let (status, cookie, _) = send(
        app,
        request("POST", "/api/v1/auth/sign-in", None, Some(body)),
    )
    .await;
    (status, cookie)
}

#[tokio::test]
async fn the_platform_admin_reads_one_account_with_its_memberships() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "record-a").await.to_string();
    let tenant_b = create_tenant(db.database(), "record-b").await.to_string();
    let id = create_account(
        &app,
        &cookie,
        json!({
            "username": "record-user",
            "display_name": "Record User",
            "password": "a long enough password",
            "email": "record@example.com",
            "memberships": [
                {"tenant_id": tenant_a, "role": "operator"},
                {"tenant_id": tenant_b, "role": "viewer"},
            ],
        }),
    )
    .await;
    // A second account must not leak into the answer.
    create_account(
        &app,
        &cookie,
        json!({
            "username": "record-other",
            "display_name": "Other",
            "password": "a long enough password",
            "memberships": [{"tenant_id": tenant_a, "role": "admin"}],
        }),
    )
    .await;

    let (status, _, account) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/platform/users/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{account}");
    assert_eq!(account.get("id").and_then(Value::as_str), Some(id.as_str()));
    assert_eq!(
        account.get("display_name").and_then(Value::as_str),
        Some("Record User")
    );
    assert_eq!(
        account.get("email").and_then(Value::as_str),
        Some("record@example.com")
    );
    assert_eq!(
        account.get("platform_admin").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        account.get("platform_admin_count").and_then(Value::as_i64),
        Some(1),
        "{account}"
    );
    let memberships = account
        .get("memberships")
        .and_then(Value::as_array)
        .expect("the memberships");
    let roles: Vec<(&str, &str)> = memberships
        .iter()
        .map(|item| {
            (
                item.get("tenant_name")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                item.get("role").and_then(Value::as_str).unwrap_or(""),
            )
        })
        .collect();
    assert_eq!(
        roles,
        vec![("record-a", "operator"), ("record-b", "viewer")]
    );

    db.cleanup().await;
}

#[tokio::test]
async fn an_unknown_or_malformed_account_id_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let unknown = uuid::Uuid::now_v7();
    for method in ["GET", "POST"] {
        let body = (method == "POST").then(|| json!({"new_password": "a long enough password"}));
        let suffix = if method == "POST" { "/password" } else { "" };
        let (status, _, problem) = send(
            &app,
            request(
                method,
                &format!("/api/v1/platform/users/{unknown}{suffix}"),
                Some(&cookie),
                body.clone(),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {problem}");
        assert_eq!(
            problem.get("code").and_then(Value::as_str),
            Some("not_found")
        );
        let (status, _, problem) = send(
            &app,
            request(
                method,
                &format!("/api/v1/platform/users/not-an-id{suffix}"),
                Some(&cookie),
                body,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{method} {problem}");
    }

    db.cleanup().await;
}

/// AC-AU-15: the record endpoints need the Platform Admin right; an organization Admin gets 403
/// for an account of its own organization too.
#[tokio::test]
async fn ac_au_15_a_tenant_admin_gets_403_on_the_account_record() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app_with_kek(db.database());
    let tenant = create_tenant(db.database(), "record-403").await;
    let cookie = seed_session(db.database(), tenant, "record-admin", "admin").await;
    let member = common::create_user(db.database(), tenant, "record-member", "phc", "viewer").await;

    let (status, _, problem) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/platform/users/{member}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    let (status, problem) =
        reset(&app, &cookie, &member.to_string(), "a long enough password").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("forbidden")
    );

    db.cleanup().await;
}

/// The organization user list and the user record tell an account with the installation-wide right,
/// so the organization screens do not offer a change that the API refuses (ADR-0029).
#[tokio::test]
async fn the_organization_user_views_hold_the_platform_admin_flag() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant = create_tenant(db.database(), "flag-org").await;
    let admin_cookie = seed_session(db.database(), tenant, "flag-admin", "admin").await;
    let right = create_account(
        &app,
        &cookie,
        json!({
            "username": "flag-right",
            "display_name": "Right",
            "password": "a long enough password",
            "platform_admin": true,
            "memberships": [{"tenant_id": tenant.to_string(), "role": "viewer"}],
        }),
    )
    .await;

    let (status, _, list) = send(
        &app,
        request("GET", "/api/v1/users", Some(&admin_cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let flags: Vec<(String, bool)> = list
        .get("items")
        .and_then(Value::as_array)
        .expect("the items")
        .iter()
        .map(|item| {
            (
                item.get("username")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                item.get("platform_admin")
                    .and_then(Value::as_bool)
                    .expect("the flag"),
            )
        })
        .collect();
    assert!(flags.contains(&("flag-right".to_owned(), true)), "{list}");
    assert!(flags.contains(&("flag-admin".to_owned(), false)), "{list}");

    let (status, _, user) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/users/{right}"),
            Some(&admin_cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{user}");
    assert_eq!(
        user.get("platform_admin").and_then(Value::as_bool),
        Some(true)
    );

    db.cleanup().await;
}

async fn last_sign_in(app: &Router, cookie: &str, uri: &str) -> Option<String> {
    let (status, _, body) = send(app, request("GET", uri, Some(cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body.get("last_sign_in_at")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Both user records give the last sign-in by the rule of the user list: a sign-in or a switch into
/// the organization; the platform record gives the latest over every organization.
#[tokio::test]
async fn the_user_records_hold_the_last_sign_in() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "seen-a").await;
    let tenant_b = create_tenant(db.database(), "seen-b").await;
    let admin_a = seed_session(db.database(), tenant_a, "seen-admin-a", "admin").await;
    let admin_b = seed_session(db.database(), tenant_b, "seen-admin-b", "admin").await;
    let id = create_account(
        &app,
        &cookie,
        json!({
            "username": "seen-user",
            "display_name": "Seen User",
            "password": "a long enough password",
            "memberships": [
                {"tenant_id": tenant_a.to_string(), "role": "viewer"},
                {"tenant_id": tenant_b.to_string(), "role": "viewer"},
            ],
        }),
    )
    .await;
    let platform_uri = format!("/api/v1/platform/users/{id}");
    let org_uri = format!("/api/v1/users/{id}");
    assert_eq!(last_sign_in(&app, &cookie, &platform_uri).await, None);
    assert_eq!(last_sign_in(&app, &admin_a, &org_uri).await, None);

    let (status, session) = sign_in(&app, "seen-user", "a long enough password").await;
    assert_eq!(status, StatusCode::OK);
    let session = session.expect("a session cookie");
    let (status, _, me) = send(
        &app,
        request("GET", "/api/v1/auth/me", Some(&session), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let (first, first_admin, other, other_admin) = if me
        .pointer("/tenant/id")
        .and_then(Value::as_str)
        == Some(tenant_a.to_string().as_str())
    {
        (tenant_a, &admin_a, tenant_b, &admin_b)
    } else {
        (tenant_b, &admin_b, tenant_a, &admin_a)
    };
    let signed_in = last_sign_in(&app, first_admin, &org_uri)
        .await
        .unwrap_or_else(|| panic!("no sign-in in {first}"));
    assert_eq!(last_sign_in(&app, other_admin, &org_uri).await, None);
    assert_eq!(
        last_sign_in(&app, &cookie, &platform_uri).await.as_deref(),
        Some(signed_in.as_str())
    );

    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/auth/switch",
            Some(&session),
            Some(json!({"tenant_id": other.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let switched = last_sign_in(&app, other_admin, &org_uri)
        .await
        .unwrap_or_else(|| panic!("no switch into {other}"));
    assert_eq!(
        last_sign_in(&app, &cookie, &platform_uri).await.as_deref(),
        Some(switched.as_str())
    );
    // The switch record in the source organization does not count there.
    assert_eq!(
        last_sign_in(&app, first_admin, &org_uri).await.as_deref(),
        Some(signed_in.as_str())
    );

    db.cleanup().await;
}
