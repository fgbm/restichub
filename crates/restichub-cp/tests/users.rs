//! User management tests (issue #18, FR-AU-03).

mod common;

use axum::http::StatusCode;
use restichub_cp::password;
use serde_json::{Value, json};

use common::{app, count, create_tenant, create_user, request, seed_session, send};

/// Creates an Admin session and returns the cookie.
async fn admin_cookie(
    db: &restichub_db::test_util::TestDatabase,
    tenant: restichub_db::TenantId,
) -> String {
    seed_session(db.database(), tenant, "admin", "admin").await
}

#[tokio::test]
async fn an_admin_creates_lists_changes_and_disables_a_user() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "users").await;
    let cookie = admin_cookie(&db, tenant).await;
    let app = app(db.database());

    // Create.
    let body = json!({
        "username": "bob",
        "password": "a long enough password",
        "display_name": "Bob",
        "role": "operator",
    });
    let (status, _, created) = send(
        &app,
        request("POST", "/api/v1/users", Some(&cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let user_id = created
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    assert_eq!(
        created.get("role").and_then(Value::as_str),
        Some("operator")
    );

    // List.
    let (status, _, page) = send(&app, request("GET", "/api/v1/users", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        page.get("items").and_then(Value::as_array).map(Vec::len),
        Some(2)
    );

    // Get.
    let (status, _, user) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/users/{user_id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(user.get("username").and_then(Value::as_str), Some("bob"));

    // Change the display name and the role.
    let body = json!({"display_name": "Bobby", "role": "viewer"});
    let (status, _, changed) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/users/{user_id}"),
            Some(&cookie),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        changed.get("display_name").and_then(Value::as_str),
        Some("Bobby")
    );
    assert_eq!(changed.get("role").and_then(Value::as_str), Some("viewer"));

    // A session of bob and then a disable.
    let bob_id: restichub_db::UserId = user_id.parse().unwrap_or_default();
    let bob_session = common::session_cookie(db.database(), tenant, bob_id).await;
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/users/{user_id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // The session of the disabled user no longer works.
    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/auth/me", Some(&bob_session), None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(count(db.database(), tenant, "sessions").await, 1); // only the admin session

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_02_an_operator_cannot_manage_users() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "operator-users").await;
    let cookie = seed_session(db.database(), tenant, "op", "operator").await;
    let app = app(db.database());

    let (status, _, _) = send(&app, request("GET", "/api/v1/users", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let body = json!({
        "username": "x",
        "password": "a long enough password",
        "display_name": "X",
        "role": "viewer",
    });
    let (status, _, _) = send(
        &app,
        request("POST", "/api/v1/users", Some(&cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_02_an_admin_cannot_grant_the_platform_admin_role() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "grant").await;
    let cookie = admin_cookie(&db, tenant).await;
    let app = app(db.database());

    let body = json!({
        "username": "x",
        "password": "a long enough password",
        "display_name": "X",
        "role": "platform_admin",
    });
    let (status, _, _) = send(
        &app,
        request("POST", "/api/v1/users", Some(&cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
}

#[tokio::test]
async fn the_last_admin_cannot_be_removed() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "last-admin").await;
    let cookie = admin_cookie(&db, tenant).await;
    let admin = common::list_users(db.database(), tenant).await;
    let admin_id = admin.first().map(|user| user.id).unwrap_or_default();
    let app = app(db.database());

    // A disable is refused.
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/users/{admin_id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // A change of the role away from admin is refused.
    let body = json!({"role": "viewer"});
    let (status, _, _) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/users/{admin_id}"),
            Some(&cookie),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    db.cleanup().await;
}

#[tokio::test]
async fn a_second_admin_allows_the_change() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "two-admins").await;
    let cookie = admin_cookie(&db, tenant).await;
    let app = app(db.database());

    // Create a second Admin.
    let body = json!({
        "username": "admin2",
        "password": "a long enough password",
        "display_name": "Admin Two",
        "role": "admin",
    });
    let (status, _, created) = send(
        &app,
        request("POST", "/api/v1/users", Some(&cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let second = created
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default();

    // Now the first Admin can be demoted.
    let list = common::list_users(db.database(), tenant).await;
    let first = list
        .iter()
        .find(|user| user.username == "admin")
        .map(|user| user.id)
        .unwrap_or_default();
    let body = json!({"role": "viewer"});
    let (status, _, _) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/users/{first}"),
            Some(&cookie),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(first.to_string(), second);

    db.cleanup().await;
}

#[tokio::test]
async fn the_last_active_admin_cannot_be_removed() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "last-active").await;
    // The actor is the first Admin. The second Admin is a target.
    let cookie = seed_session(db.database(), tenant, "admin1", "admin").await;
    let second = create_user(db.database(), tenant, "admin2", "phc", "admin").await;
    let app = app(db.database());

    // Disabling the second Admin leaves one active Admin and succeeds.
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/users/{second}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Disabling the last active Admin is refused.
    let first = common::list_users(db.database(), tenant)
        .await
        .into_iter()
        .find(|user| user.username == "admin1")
        .map(|user| user.id)
        .unwrap_or_default();
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/users/{first}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(!common::user_disabled(db.database(), tenant, first).await);

    db.cleanup().await;
}

#[tokio::test]
async fn the_last_admin_guard_skips_a_disabled_admin() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "disabled-admin").await;
    // The actor is an active Admin. The second Admin is disabled first.
    let cookie = seed_session(db.database(), tenant, "admin1", "admin").await;
    let disabled = create_user(db.database(), tenant, "admin2", "phc", "admin").await;
    let app = app(db.database());
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/users/{disabled}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The target is disabled, so it is not the last active Admin. The guard does not fire.
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/users/{disabled}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/users/{disabled}"),
            Some(&cookie),
            Some(json!({"role": "viewer"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    db.cleanup().await;
}

#[tokio::test]
async fn two_concurrent_admin_disables_leave_one_admin() {
    use restichub_db::memberships;

    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "concurrent").await;
    // The actor is an Admin API token whose creator is not an Admin, so disabling the two Admins
    // does not invalidate the actor.
    let token = common::seed_admin_token_actor(db.database(), tenant).await;
    let first = create_user(db.database(), tenant, "admin1", "phc", "admin").await;
    let second = create_user(db.database(), tenant, "admin2", "phc", "admin").await;
    let app = app(db.database());

    // Two parallel PATCH requests disable one Admin each. The tenant lock serializes them, so
    // exactly one succeeds and the other sees the last Admin.
    let uri_first = format!("/api/v1/users/{first}");
    let uri_second = format!("/api/v1/users/{second}");
    let (first_result, second_result) = tokio::join!(
        send(
            &app,
            common::bearer_request("PATCH", &uri_first, &token, Some(json!({"disabled": true}))),
        ),
        send(
            &app,
            common::bearer_request(
                "PATCH",
                &uri_second,
                &token,
                Some(json!({"disabled": true}))
            ),
        ),
    );
    let statuses = [first_result.0, second_result.0];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1,
        "exactly one disable must succeed: {statuses:?}"
    );
    assert!(
        statuses
            .iter()
            .all(|status| *status == StatusCode::OK || *status == StatusCode::CONFLICT)
    );

    // One active Admin remains.
    let mut tx = db
        .database()
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let remaining = memberships::count_admins(&mut tx)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(remaining, 1);

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_02_a_lower_role_is_refused_in_the_platform_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = seed_session(
        db.database(),
        restichub_db::PLATFORM_TENANT_ID,
        "root",
        "platform_admin",
    )
    .await;
    let app = app(db.database());

    let body = json!({
        "username": "lower",
        "password": "a long enough password",
        "display_name": "Lower",
        "role": "admin",
    });
    let (status, _, _) = send(
        &app,
        request("POST", "/api/v1/users", Some(&cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
}

#[tokio::test]
async fn a_duplicate_username_is_a_conflict() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "duplicate").await;
    let cookie = admin_cookie(&db, tenant).await;
    let app = app(db.database());

    let body = json!({
        "username": "bob",
        "password": "a long enough password",
        "display_name": "Bob",
        "role": "viewer",
    });
    let (status, _, _) = send(
        &app,
        request("POST", "/api/v1/users", Some(&cookie), Some(body.clone())),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _, _) = send(
        &app,
        request("POST", "/api/v1/users", Some(&cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    db.cleanup().await;
}

#[tokio::test]
async fn an_admin_password_reset_kills_the_sessions_of_the_user() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "reset").await;
    let admin = admin_cookie(&db, tenant).await;
    let app = app(db.database());

    // Create a user and sign it in through a session.
    let hash = password::hash_password("a long enough password").unwrap_or_default();
    let user = common::create_user(db.database(), tenant, "bob", &hash, "viewer").await;
    let bob_session = common::session_cookie(db.database(), tenant, user).await;

    let body = json!({"new_password": "another long password"});
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/users/{user}/password"),
            Some(&admin),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/auth/me", Some(&bob_session), None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The membership is unchanged.
    let role = common::role_of(db.database(), tenant, user).await;
    assert_eq!(role.as_deref(), Some("viewer"));

    db.cleanup().await;
}
