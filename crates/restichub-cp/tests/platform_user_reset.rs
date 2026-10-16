//! The password reset of an account by a Platform Admin and the refusal of the own reset (ADR-0029,
//! T33).

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

async fn own_id(app: &Router, cookie: &str) -> String {
    let (status, _, me) = send(app, request("GET", "/api/v1/auth/me", Some(cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    me.pointer("/user/id")
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

/// The platform reset sets the new password, ends every session of the account in every
/// organization and writes a platform audit record without the secret.
#[tokio::test]
async fn the_platform_reset_changes_the_password_of_a_shared_account() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "reset-a").await.to_string();
    let tenant_b = create_tenant(db.database(), "reset-b").await.to_string();
    // Two memberships make the account shared: an organization Admin cannot reset it (ADR-0025).
    let id = create_account(
        &app,
        &cookie,
        json!({
            "username": "reset-user",
            "display_name": "Reset User",
            "password": "the first password",
            "memberships": [
                {"tenant_id": tenant_a, "role": "viewer"},
                {"tenant_id": tenant_b, "role": "viewer"},
            ],
        }),
    )
    .await;
    let sessions = sessions_in_both(&app, &tenant_a, &tenant_b).await;

    let (status, problem) = reset(&app, &cookie, &id, "short").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");

    let (status, problem) = reset(&app, &cookie, &id, "the second password").await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{problem}");

    let (status, _) = sign_in(&app, "reset-user", "the first password").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    for session in &sessions {
        let (status, _, _) =
            send(&app, request("GET", "/api/v1/auth/me", Some(session), None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _) = sign_in(&app, "reset-user", "the second password").await;
    assert_eq!(status, StatusCode::OK);

    let record = reset_record(&app, &cookie, PLATFORM_TENANT_ID.to_string().as_str(), &id).await;
    assert!(record.pointer("/diff/new/password").is_some(), "{record}");
    assert_no_secret(&record, "the second password");

    db.cleanup().await;
}

/// Signs `reset-user` in twice and moves the second session to the other organization, so the
/// account holds one session in each of the two organizations.
async fn sessions_in_both(app: &Router, tenant_a: &str, tenant_b: &str) -> [String; 2] {
    let (status, first) = sign_in(app, "reset-user", "the first password").await;
    assert_eq!(status, StatusCode::OK);
    let first = first.unwrap_or_else(|| panic!("no session cookie"));
    let (status, _, me) = send(app, request("GET", "/api/v1/auth/me", Some(&first), None)).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let other = if me.pointer("/tenant/id").and_then(Value::as_str) == Some(tenant_a) {
        tenant_b
    } else {
        tenant_a
    };
    let (status, second) = sign_in(app, "reset-user", "the first password").await;
    assert_eq!(status, StatusCode::OK);
    let second = second.unwrap_or_else(|| panic!("no session cookie"));
    let (status, switched, body) = send(
        app,
        request(
            "POST",
            "/api/v1/auth/switch",
            Some(&second),
            Some(json!({"tenant_id": other})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let switched = switched.unwrap_or_else(|| panic!("no cookie of the switched session"));
    [first, switched]
}

/// The newest password reset record of the account in the audit of the tenant.
async fn reset_record(app: &Router, cookie: &str, tenant: &str, id: &str) -> Value {
    let uri =
        format!("/api/v1/platform/audit?action=user_update&tenant_id={tenant}&object_id={id}");
    let (status, _, page) = send(app, request("GET", &uri, Some(cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    page.get("items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.pointer("/diff/new/password").is_some())
                .cloned()
        })
        .unwrap_or_else(|| panic!("no reset record in {tenant}: {page}"))
}

fn assert_no_secret(record: &Value, password: &str) {
    let text = record.to_string();
    assert!(!text.contains(password), "{record}");
    assert!(!text.contains("$argon2"), "{record}");
}

/// T33: the reset from the platform is a record in the audit of each organization of the account,
/// so an organization Admin sees that its member got a new password.
#[tokio::test]
async fn the_platform_reset_is_audited_in_every_organization_of_the_account() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "audit-a").await;
    let tenant_b = create_tenant(db.database(), "audit-b").await;
    let tenant_c = create_tenant(db.database(), "audit-c").await;
    let id = create_account(
        &app,
        &cookie,
        json!({
            "username": "audit-user",
            "display_name": "Audit User",
            "password": "the first password",
            "memberships": [
                {"tenant_id": tenant_a.to_string(), "role": "viewer"},
                {"tenant_id": tenant_b.to_string(), "role": "operator"},
            ],
        }),
    )
    .await;

    let (status, problem) = reset(&app, &cookie, &id, "the second password").await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{problem}");

    for (tenant, name) in [(tenant_a, "audit-admin-a"), (tenant_b, "audit-admin-b")] {
        let admin = seed_session(db.database(), tenant, name, "admin").await;
        let uri = format!("/api/v1/audit?action=user_update&object_id={id}");
        let (status, _, page) = send(&app, request("GET", &uri, Some(&admin), None)).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let record = page
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item.pointer("/diff/new/password").is_some())
                    .cloned()
            })
            .unwrap_or_else(|| panic!("no reset record in {name}: {page}"));
        assert_no_secret(&record, "the second password");
    }
    let uri =
        format!("/api/v1/platform/audit?action=user_update&tenant_id={tenant_c}&object_id={id}");
    let (status, _, page) = send(&app, request("GET", &uri, Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        page.get("items").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "{page}"
    );

    db.cleanup().await;
}

/// A Platform Admin can revoke the right of another one and then reset the password, so the reset
/// of another Platform Admin is allowed and audited.
#[tokio::test]
async fn the_platform_reset_of_another_platform_admin_is_allowed_and_audited() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let me = own_id(&app, &cookie).await;
    let id = create_account(
        &app,
        &cookie,
        json!({
            "username": "peer-admin",
            "display_name": "Peer Admin",
            "password": "the first password",
            "platform_admin": true,
        }),
    )
    .await;

    let (status, problem) = reset(&app, &cookie, &id, "the second password").await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{problem}");
    let (status, _) = sign_in(&app, "peer-admin", "the second password").await;
    assert_eq!(status, StatusCode::OK);

    let record = reset_record(&app, &cookie, PLATFORM_TENANT_ID.to_string().as_str(), &id).await;
    assert_eq!(
        record.pointer("/actor_id").and_then(Value::as_str),
        Some(me.as_str()),
        "{record}"
    );
    assert_no_secret(&record, "the second password");

    db.cleanup().await;
}

/// An organization Admin changes its own password only through the self change that checks the
/// current password.
#[tokio::test]
async fn the_organization_reset_refuses_the_own_account() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant = create_tenant(db.database(), "self-reset").await;
    let id = create_account(
        &app,
        &cookie,
        json!({
            "username": "self-admin",
            "display_name": "Self Admin",
            "password": "the first password",
            "memberships": [{"tenant_id": tenant.to_string(), "role": "admin"}],
        }),
    )
    .await;
    let (status, session) = sign_in(&app, "self-admin", "the first password").await;
    assert_eq!(status, StatusCode::OK);
    let session = session.expect("a session cookie");

    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/users/{id}/password"),
            Some(&session),
            Some(json!({"new_password": "the second password"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("forbidden")
    );
    let (status, _) = sign_in(&app, "self-admin", "the first password").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/auth/me", Some(&session), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    db.cleanup().await;
}

/// The caller resets the password of its own account only through the self change that checks the
/// current password.
#[tokio::test]
async fn the_platform_reset_refuses_the_own_account() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let me = own_id(&app, &cookie).await;

    let (status, problem) = reset(&app, &cookie, &me, "a long enough password").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("forbidden")
    );
    let (status, _, _) = send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::OK);

    db.cleanup().await;
}
