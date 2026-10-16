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

/// T22: the reset from the platform is a record in the audit of each organization of the account,
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
