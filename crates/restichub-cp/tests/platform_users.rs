//! Platform account endpoints and the installation account list (ADR-0028).
//!
//! The tests cover the account create with memberships, the installation-wide account list, the
//! email rules, the role change, the membership removal and the protections of the Platform Admin
//! (AC-AU-08, AC-AU-09, AC-AU-10).

mod common;

use axum::Router;
use axum::http::StatusCode;
use restichub_db::PLATFORM_TENANT_ID;
use serde_json::{Value, json};

use common::{app_with_kek, create_tenant, create_user, request, seed_session, send};

/// Returns a root session cookie of the platform tenant.
async fn platform_admin(db: &restichub_db::Database) -> String {
    seed_session(db, PLATFORM_TENANT_ID, "root", "platform_admin").await
}

/// Creates an account through the API and returns the response body.
async fn create_account(app: &Router, cookie: &str, body: Value) -> (StatusCode, Value) {
    let (status, _, response) = send(
        app,
        request("POST", "/api/v1/platform/users", Some(cookie), Some(body)),
    )
    .await;
    (status, response)
}

/// Reads one item of the list by its username.
fn find_account(list: &Value, username: &str) -> Option<Value> {
    list.get("items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("username").and_then(Value::as_str) == Some(username))
                .cloned()
        })
}

#[tokio::test]
async fn ac_au_09_the_platform_admin_lists_every_account_with_its_memberships() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "acct-a").await.to_string();
    let tenant_b = create_tenant(db.database(), "acct-b").await.to_string();

    let body = json!({
        "username": "multi",
        "display_name": "Multi",
        "password": "a long enough password",
        "email": "Multi@Example.com",
        "memberships": [
            {"tenant_id": tenant_a, "role": "admin"},
            {"tenant_id": tenant_b, "role": "viewer"},
        ],
    });
    let (status, account) = create_account(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    assert_eq!(
        account.get("email").and_then(Value::as_str),
        Some("multi@example.com")
    );
    // Stored unverified in the MVP.
    assert_eq!(
        account.get("email_verified").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        account
            .get("memberships")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );

    let (status, _, list) = send(
        &app,
        request("GET", "/api/v1/platform/users", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let found = find_account(&list, "multi").unwrap_or_else(|| panic!("no account: {list}"));
    let memberships = found
        .get("memberships")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(memberships.len(), 2);
    assert!(memberships.iter().any(|member| {
        member.get("tenant_id").and_then(Value::as_str) == Some(tenant_a.as_str())
            && member.get("role").and_then(Value::as_str) == Some("admin")
    }));
    assert!(memberships.iter().any(|member| {
        member.get("tenant_id").and_then(Value::as_str) == Some(tenant_b.as_str())
            && member.get("role").and_then(Value::as_str) == Some("viewer")
    }));

    // Tenant Admin: HTTP 403.
    let tenant_id = tenant_a.parse().unwrap_or_else(|error| panic!("{error}"));
    let admin_cookie = seed_session(db.database(), tenant_id, "tenant-admin", "admin").await;
    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/platform/users", Some(&admin_cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_08_the_account_signs_in_and_switches_between_its_memberships() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "signin-a").await;
    let tenant_b = create_tenant(db.database(), "signin-b").await;

    let body = json!({
        "username": "switcher",
        "display_name": "Switcher",
        "password": "a long enough password",
        "memberships": [{"tenant_id": tenant_a.to_string(), "role": "admin"}],
    });
    let (status, account) = create_account(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    let user_id = account
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    // The account signs in with the username and the password only (ADR-0025).
    let sign_in = json!({"username": "switcher", "password": "a long enough password"});
    let (status, session, me) = send(
        &app,
        request("POST", "/api/v1/auth/sign-in", None, Some(sign_in)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let session = session.unwrap_or_else(|| panic!("no session cookie: {me}"));

    // Add the second membership through the platform route.
    let (status, _, added) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/platform/users/{user_id}/memberships"),
            Some(&cookie),
            Some(json!({"tenant_id": tenant_b.to_string(), "role": "viewer"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{added}");

    // The switch to the second organization works (AC-AU-04).
    let (status, _, switched) = send(
        &app,
        request(
            "POST",
            "/api/v1/auth/switch",
            Some(&session),
            Some(json!({"tenant_id": tenant_b.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{switched}");
    assert_eq!(
        switched.pointer("/tenant/name").and_then(Value::as_str),
        Some("signin-b")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_10_a_duplicate_username_or_email_is_a_conflict() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());

    let first = json!({
        "username": "taken",
        "display_name": "Taken",
        "password": "a long enough password",
        "email": "taken@example.com",
    });
    let (status, account) = create_account(&app, &cookie, first).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");

    let same_username = json!({
        "username": "TAKEN",
        "display_name": "Other",
        "password": "a long enough password",
    });
    let (status, problem) = create_account(&app, &cookie, same_username).await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("username_taken")
    );

    let same_email = json!({
        "username": "other",
        "display_name": "Other",
        "password": "a long enough password",
        "email": "Taken@Example.com",
    });
    let (status, problem) = create_account(&app, &cookie, same_email).await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("email_taken")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_platform_tenant_is_refused_as_a_membership_target() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "target-a").await;

    // The account create refuses the platform tenant.
    let body = json!({
        "username": "platform-target",
        "display_name": "Target",
        "password": "a long enough password",
        "memberships": [{"tenant_id": PLATFORM_TENANT_ID.to_string(), "role": "admin"}],
    });
    let (status, problem) = create_account(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");

    // The add endpoint refuses the platform tenant too.
    let body = json!({
        "username": "target-owner",
        "display_name": "Owner",
        "password": "a long enough password",
    });
    let (status, account) = create_account(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    let user_id = account
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/platform/users/{user_id}/memberships"),
            Some(&cookie),
            Some(json!({"tenant_id": PLATFORM_TENANT_ID.to_string(), "role": "admin"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");

    // Adding a real organization succeeds.
    let (status, _, added) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/platform/users/{user_id}/memberships"),
            Some(&cookie),
            Some(json!({"tenant_id": tenant_a.to_string(), "role": "viewer"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{added}");

    db.cleanup().await;
}

#[tokio::test]
async fn the_role_change_and_the_membership_removal_change_the_membership() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "role-a").await.to_string();

    let body = json!({
        "username": "role-owner",
        "display_name": "Owner",
        "password": "a long enough password",
        "memberships": [{"tenant_id": tenant_a, "role": "viewer"}],
    });
    let (status, account) = create_account(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    let user_id = account
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    // Change the role.
    let path = format!("/api/v1/platform/users/{user_id}/memberships/{tenant_a}");
    let (status, _, problem) = send(
        &app,
        request(
            "PATCH",
            &path,
            Some(&cookie),
            Some(json!({"role": "operator"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{problem}");
    let (_, _, list) = send(
        &app,
        request("GET", "/api/v1/platform/users", Some(&cookie), None),
    )
    .await;
    let found = find_account(&list, "role-owner").unwrap_or_else(|| panic!("no account: {list}"));
    assert_eq!(
        found.pointer("/memberships/0/role").and_then(Value::as_str),
        Some("operator")
    );

    // Remove the membership.
    let (status, _, problem) = send(&app, request("DELETE", &path, Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{problem}");
    let (_, _, list) = send(
        &app,
        request("GET", "/api/v1/platform/users", Some(&cookie), None),
    )
    .await;
    let found = find_account(&list, "role-owner").unwrap_or_else(|| panic!("no account: {list}"));
    assert_eq!(
        found
            .get("memberships")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0)
    );

    // The removal of an absent membership gives HTTP 404.
    let (status, _, _) = send(&app, request("DELETE", &path, Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    db.cleanup().await;
}

#[tokio::test]
async fn the_account_update_sets_the_email_and_keeps_the_other_fields() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());

    let body = json!({
        "username": "email-owner",
        "display_name": "Owner",
        "password": "a long enough password",
    });
    let (status, account) = create_account(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    let user_id = account
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    let (status, _, problem) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/platform/users/{user_id}"),
            Some(&cookie),
            Some(json!({"email": "Owner@Example.com"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{problem}");

    let (_, _, list) = send(
        &app,
        request("GET", "/api/v1/platform/users", Some(&cookie), None),
    )
    .await;
    let found = find_account(&list, "email-owner").unwrap_or_else(|| panic!("no account: {list}"));
    assert_eq!(
        found.get("email").and_then(Value::as_str),
        Some("owner@example.com")
    );
    assert_eq!(
        found.get("email_verified").and_then(Value::as_bool),
        Some(false)
    );

    // Both fields absent gives HTTP 400.
    let (status, _, problem) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/platform/users/{user_id}"),
            Some(&cookie),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");

    db.cleanup().await;
}

/// AC-AU-14: the platform screens open for a Platform Admin in any current organization.
#[tokio::test]
async fn ac_au_14_the_platform_screens_open_in_any_organization() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "screen-a").await.to_string();

    // A Platform Admin with the right and a membership in A.
    let body = json!({
        "username": "org-admin",
        "display_name": "Org Admin",
        "password": "a long enough password",
        "platform_admin": true,
        "memberships": [{"tenant_id": tenant_a, "role": "admin"}],
    });
    let (status, account) = create_account(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");

    // The account signs in and the session is in organization A.
    let sign_in = json!({"username": "org-admin", "password": "a long enough password"});
    let (status, session, me) = send(
        &app,
        request("POST", "/api/v1/auth/sign-in", None, Some(sign_in)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(
        me.pointer("/tenant/name").and_then(Value::as_str),
        Some("screen-a")
    );
    assert_eq!(
        me.get("platform_admin").and_then(Value::as_bool),
        Some(true)
    );
    let session = session.expect("a session cookie");

    // The platform screens open in the current organization, with no switch.
    for path in [
        "/api/v1/platform/tenants",
        "/api/v1/platform/users",
        "/api/v1/platform/audit",
    ] {
        let (status, _, problem) = send(&app, request("GET", path, Some(&session), None)).await;
        assert_eq!(status, StatusCode::OK, "{path}: {problem}");
    }

    db.cleanup().await;
}

/// AC-AU-15: a tenant Admin gets HTTP 403 on the platform screens.
#[tokio::test]
async fn ac_au_15_a_tenant_admin_gets_403_on_the_platform_screens() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app_with_kek(db.database());
    let tenant = create_tenant(db.database(), "tenant-403").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    for path in [
        "/api/v1/platform/tenants",
        "/api/v1/platform/users",
        "/api/v1/platform/audit",
    ] {
        let (status, _, _) = send(&app, request("GET", path, Some(&cookie), None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
    }
    db.cleanup().await;
}

/// AC-AU-16: the caller cannot remove its own right; the right of another account changes.
#[tokio::test]
async fn ac_au_16_the_self_right_and_the_last_admin_stay() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let (_, _, me) = send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)).await;
    let me_id = me
        .pointer("/user/id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    // The caller cannot remove its own right and cannot disable itself (ADR-0029).
    for body in [json!({"platform_admin": false}), json!({"disabled": true})] {
        let (status, _, _) = send(
            &app,
            request(
                "PATCH",
                &format!("/api/v1/platform/users/{me_id}"),
                Some(&cookie),
                Some(body),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    // Granting the right to a second account and removing it again is allowed.
    let body = json!({
        "username": "second-right",
        "display_name": "Second",
        "password": "a long enough password",
    });
    let (status, account) = create_account(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    let second = account
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    for value in [true, false] {
        let (status, _, problem) = send(
            &app,
            request(
                "PATCH",
                &format!("/api/v1/platform/users/{second}"),
                Some(&cookie),
                Some(json!({"platform_admin": value})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{problem}");
    }

    db.cleanup().await;
}

/// AC-AU-17: the Platform Admin right alone gives no tenant data access (ADR-0006).
#[tokio::test]
async fn ac_au_17_the_right_gives_no_tenant_access() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "viewer-a").await;
    // Organization A holds a member.
    create_user(db.database(), tenant_a, "a-user", "phc", "admin").await;

    // A Platform Admin with the Viewer role in A.
    let body = json!({
        "username": "viewer-admin",
        "display_name": "Viewer Admin",
        "password": "a long enough password",
        "platform_admin": true,
        "memberships": [{"tenant_id": tenant_a.to_string(), "role": "viewer"}],
    });
    let (status, account) = create_account(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");

    let sign_in = json!({"username": "viewer-admin", "password": "a long enough password"});
    let (status, session, me) = send(
        &app,
        request("POST", "/api/v1/auth/sign-in", None, Some(sign_in)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me.get("role").and_then(Value::as_str), Some("viewer"));
    let session = session.expect("a session cookie");

    // The Viewer role refuses the tenant user list, while the platform screens open.
    let (status, _, _) = send(&app, request("GET", "/api/v1/users", Some(&session), None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/platform/users", Some(&session), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    db.cleanup().await;
}

/// AC-AU-12 and AC-AU-13: the membership list holds real organizations only.
#[tokio::test]
async fn ac_au_12_13_the_membership_list_holds_real_organizations_only() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "switch-a").await.to_string();
    let tenant_b = create_tenant(db.database(), "switch-b").await.to_string();

    // A Platform Admin with no membership: the list is empty (AC-AU-12).
    let (status, _, list) = send(
        &app,
        request("GET", "/api/v1/auth/tenants", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(
        list.get("items").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "{list}"
    );

    // A Platform Admin with two memberships: the list holds A and B only (AC-AU-13).
    let body = json!({
        "username": "switcher-two",
        "display_name": "Switcher",
        "password": "a long enough password",
        "platform_admin": true,
        "memberships": [
            {"tenant_id": tenant_a, "role": "admin"},
            {"tenant_id": tenant_b, "role": "viewer"},
        ],
    });
    let (status, account) = create_account(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    let sign_in = json!({"username": "switcher-two", "password": "a long enough password"});
    let (status, session, me) = send(
        &app,
        request("POST", "/api/v1/auth/sign-in", None, Some(sign_in)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    let session = session.expect("a session cookie");
    let (status, _, list) = send(
        &app,
        request("GET", "/api/v1/auth/tenants", Some(&session), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let names: Vec<String> = list
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("name").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        names,
        vec!["switch-a".to_owned(), "switch-b".to_owned()],
        "{list}"
    );
    assert!(
        !list.to_string().contains(&PLATFORM_TENANT_ID.to_string()),
        "{list}"
    );

    db.cleanup().await;
}

/// Reads the platform audit items for one action and one tenant.
async fn platform_audit(app: &Router, cookie: &str, action: &str, tenant: &str) -> Vec<Value> {
    let uri = format!("/api/v1/platform/audit?action={action}&tenant_id={tenant}");
    let (status, _, page) = send(app, request("GET", &uri, Some(cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    page.get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn holds_object_id(items: &[Value], id: &str) -> bool {
    items
        .iter()
        .any(|item| item.get("object_id").and_then(Value::as_str) == Some(id))
}

/// The membership add writes the target organization record and the platform record in one
/// transaction (ADR-0029).
#[tokio::test]
async fn add_membership_writes_both_audit_records() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "audit-add").await.to_string();

    let body = json!({
        "username": "audit-owner",
        "display_name": "Owner",
        "password": "a long enough password",
    });
    let (status, account) = create_account(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    let user_id = account
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/platform/users/{user_id}/memberships"),
            Some(&cookie),
            Some(json!({"tenant_id": tenant_a, "role": "operator"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{problem}");

    // Platform audit: the record must be there.
    let platform_items = platform_audit(
        &app,
        &cookie,
        "user_update",
        &PLATFORM_TENANT_ID.to_string(),
    )
    .await;
    assert!(
        holds_object_id(&platform_items, &user_id),
        "the platform record is missing: {platform_items:?}"
    );
    // The target organization audit holds the record.
    let tenant_items = platform_audit(&app, &cookie, "user_update", &tenant_a).await;
    assert!(
        holds_object_id(&tenant_items, &user_id),
        "the organization record is missing: {tenant_items:?}"
    );

    db.cleanup().await;
}

/// A role change and a membership removal are also platform audit records (ADR-0029).
#[tokio::test]
async fn a_role_change_and_a_removal_are_in_the_platform_audit() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "audit-role").await.to_string();

    let body = json!({
        "username": "role-audit",
        "display_name": "Role",
        "password": "a long enough password",
        "memberships": [{"tenant_id": tenant_a, "role": "viewer"}],
    });
    let (status, account) = create_account(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    let user_id = account
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    let path = format!("/api/v1/platform/users/{user_id}/memberships/{tenant_a}");
    let (status, _, problem) = send(
        &app,
        request(
            "PATCH",
            &path,
            Some(&cookie),
            Some(json!({"role": "operator"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{problem}");
    let (status, _, problem) = send(&app, request("DELETE", &path, Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{problem}");

    let platform_items = platform_audit(
        &app,
        &cookie,
        "user_update",
        &PLATFORM_TENANT_ID.to_string(),
    )
    .await;
    let count = platform_items
        .iter()
        .filter(|item| item.get("object_id").and_then(Value::as_str) == Some(user_id.as_str()))
        .count();
    assert!(
        count >= 2,
        "the platform records for the role change and the removal are missing: {platform_items:?}"
    );

    db.cleanup().await;
}

/// The Platform Admin right is enough to grant a role, whatever the role of the caller in its
/// current organization (ADR-0029).
#[tokio::test]
async fn a_platform_admin_with_a_viewer_org_role_grants_admin() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let tenant_a = create_tenant(db.database(), "viewer-grant")
        .await
        .to_string();

    // A Platform Admin with the Viewer role in organization A.
    let body = json!({
        "username": "viewer-granter",
        "display_name": "Granter",
        "password": "a long enough password",
        "platform_admin": true,
        "memberships": [{"tenant_id": tenant_a, "role": "viewer"}],
    });
    let (status, account) = create_account(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::CREATED, "{account}");
    let sign_in = json!({"username": "viewer-granter", "password": "a long enough password"});
    let (status, session, me) = send(
        &app,
        request("POST", "/api/v1/auth/sign-in", None, Some(sign_in)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me.get("role").and_then(Value::as_str), Some("viewer"));
    let session = session.expect("a session cookie");

    // The Viewer-role Platform Admin creates an account and grants the Admin role.
    let body = json!({
        "username": "grantee",
        "display_name": "Grantee",
        "password": "a long enough password",
    });
    let (status, grantee) = create_account(&app, &session, body).await;
    assert_eq!(status, StatusCode::CREATED, "{grantee}");
    let grantee_id = grantee
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/platform/users/{grantee_id}/memberships"),
            Some(&session),
            Some(json!({"tenant_id": tenant_a, "role": "admin"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{problem}");

    db.cleanup().await;
}

/// Two concurrent account creates race on the unique username and email indexes. The loser gets
/// HTTP 409, not HTTP 500 (ADR-0028).
#[tokio::test]
async fn a_concurrent_account_create_is_a_conflict() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());

    let body = |username: &str, email: &str| {
        json!({
            "username": username,
            "display_name": "Race",
            "password": "a long enough password",
            "email": email,
        })
    };
    let (a, b) = tokio::join!(
        create_account(&app, &cookie, body("race-name", "race-name@example.com")),
        create_account(&app, &cookie, body("race-name", "race-name@example.com")),
    );
    let statuses = [a.0, b.0];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "exactly one create must win: {statuses:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CONFLICT)
            .count(),
        1,
        "the loser must be a conflict, not an internal error: {statuses:?}"
    );

    // Race on the email index gives `email_taken`.
    let (c, d) = tokio::join!(
        create_account(
            &app,
            &cookie,
            body("race-email-a", "race-email@example.com")
        ),
        create_account(
            &app,
            &cookie,
            body("race-email-b", "race-email@example.com")
        ),
    );
    let email_statuses = [c.0, d.0];
    let email_problems = [c.1, d.1];
    assert_eq!(
        email_statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "exactly one create must win: {email_statuses:?}"
    );
    assert!(
        email_problems
            .iter()
            .any(|problem| { problem.get("code").and_then(Value::as_str) == Some("email_taken") }),
        "the loser must answer email_taken: {email_problems:?}"
    );

    db.cleanup().await;
}
