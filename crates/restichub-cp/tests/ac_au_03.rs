//! Installation-wide accounts, the tenant switch and the user admin rules (ADR-0025).
//!
//! AC-AU-03: sign-in with the username and the password only, one active membership.
//! AC-AU-04: two active memberships and the last used tenant.
//! AC-AU-05: the tenant switch and the audit record in both tenants.
//! AC-AU-06: no active membership and the enumeration defence.
//! AC-AU-07: the shared-account guard and the taken username.

#![allow(clippy::expect_used)]

mod common;

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use restichub_cp::password;
use restichub_db::audit::{AuditFilter, AuditPage};
use restichub_db::{PLATFORM_TENANT_ID, TenantId, audit};
use serde_json::{Value, json};

use common::{app, create_tenant, create_user, request, seed_session, send, session_cookie};

/// Builds the request body for the tests.
async fn sign_in(
    app: &axum::Router,
    username: &str,
    password_value: &str,
) -> (StatusCode, Option<String>, Value) {
    let body = json!({"username": username, "password": password_value});
    send(
        app,
        request("POST", "/api/v1/auth/sign-in", None, Some(body)),
    )
    .await
}

/// Adds a member with a real password hash to a tenant.
async fn account(
    db: &restichub_db::Database,
    tenant: TenantId,
    username: &str,
    password_value: &str,
    role: &str,
) -> restichub_db::UserId {
    let hash = password::hash_password(password_value).unwrap_or_default();
    create_user(db, tenant, username, &hash, role).await
}

/// Gives an existing identity a role in a tenant.
async fn add_membership(
    db: &restichub_db::Database,
    tenant: TenantId,
    user: restichub_db::UserId,
    role: &str,
) {
    use restichub_db::memberships;
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    memberships::set_role(
        &mut tx,
        user,
        restichub_db::Role::parse(role).expect("the role"),
    )
    .await
    .expect("cannot set the role");
    tx.commit().await.expect("cannot commit the membership");
}

/// Reads the recorded events of a tenant for the assertions.
async fn audit_actions(db: &restichub_db::Database, tenant: TenantId) -> Vec<String> {
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .expect("cannot begin the transaction");
    let records = audit::list(&mut tx, &AuditFilter::default(), &AuditPage::default())
        .await
        .expect("cannot list the audit records")
        .records;
    tx.commit().await.expect("cannot commit the read");
    records
        .into_iter()
        .map(|record| record.action.as_str().to_owned())
        .collect()
}

#[tokio::test]
async fn ac_au_03_sign_in_with_the_username_only() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "ac-au-03").await;
    account(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let app = app(db.database());

    let (status, cookie, body) = sign_in(&app, "alice", "correct horse battery").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(cookie.is_some());
    assert_eq!(
        body.pointer("/tenant/name").and_then(Value::as_str),
        Some("ac-au-03")
    );
    assert_eq!(body.get("role").and_then(Value::as_str), Some("admin"));

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_04_two_memberships_and_the_last_used_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let alpha = create_tenant(db.database(), "ac-au-alpha").await;
    let beta = create_tenant(db.database(), "ac-au-beta").await;
    let user = account(
        db.database(),
        alpha,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    add_membership(db.database(), beta, user, "viewer").await;
    let app = app(db.database());

    // No last used tenant: the first membership by tenant name gives the session.
    let (status, cookie, body) = sign_in(&app, "alice", "correct horse battery").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body.pointer("/tenant/name").and_then(Value::as_str),
        Some("ac-au-alpha")
    );
    let cookie = cookie.expect("a session cookie");

    // The tenants list holds the two memberships only.
    let (status, _, list) = send(
        &app,
        request("GET", "/api/v1/auth/tenants", Some(&cookie), None),
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
        vec!["ac-au-alpha".to_owned(), "ac-au-beta".to_owned()]
    );
    assert!(!list.to_string().contains("ac-au-gamma"));

    // A switch to beta stores the last used tenant. The next sign-in uses it.
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/auth/switch",
            Some(&cookie),
            Some(json!({"tenant_id": beta.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, body) = sign_in(&app, "alice", "correct horse battery").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body.pointer("/tenant/name").and_then(Value::as_str),
        Some("ac-au-beta")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_05_switch_rotates_the_session_and_audits_both_tenants() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let alpha = create_tenant(db.database(), "ac-au-switch-a").await;
    let beta = create_tenant(db.database(), "ac-au-switch-b").await;
    let gamma = create_tenant(db.database(), "ac-au-switch-c").await;
    let user = account(
        db.database(),
        alpha,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    add_membership(db.database(), beta, user, "viewer").await;
    let app = app(db.database());

    let (status, old_cookie, _) = sign_in(&app, "alice", "correct horse battery").await;
    assert_eq!(status, StatusCode::OK);
    let old_cookie = old_cookie.expect("a session cookie");

    let (status, new_cookie, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/auth/switch",
            Some(&old_cookie),
            Some(json!({"tenant_id": beta.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let new_cookie = new_cookie.expect("a rotated session cookie");
    assert_ne!(old_cookie, new_cookie);
    assert_eq!(
        body.pointer("/tenant/name").and_then(Value::as_str),
        Some("ac-au-switch-b")
    );

    // Old cookie rejected after the rotation.
    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/auth/me", Some(&old_cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // New cookie accepted.
    let (status, _, me) = send(
        &app,
        request("GET", "/api/v1/auth/me", Some(&new_cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(
        me.pointer("/tenant/name").and_then(Value::as_str),
        Some("ac-au-switch-b")
    );

    // The audit holds `session_switch` in both tenants.
    assert!(
        audit_actions(db.database(), alpha)
            .await
            .iter()
            .any(|action| action == "session_switch")
    );
    assert!(
        audit_actions(db.database(), beta)
            .await
            .iter()
            .any(|action| action == "session_switch")
    );

    // A tenant without a membership gives HTTP 403.
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/auth/switch",
            Some(&new_cookie),
            Some(json!({"tenant_id": gamma.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_06_no_active_membership_and_the_enumeration_defence() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "ac-au-06").await;
    // An account with a real password and no membership.
    let hash = password::hash_password("correct horse battery").unwrap_or_default();
    {
        let mut tx = db
            .database()
            .begin_tenant(tenant)
            .await
            .expect("cannot begin the transaction");
        restichub_db::users::create(
            &mut tx,
            &restichub_db::users::NewUser {
                username: "memberless",
                password_hash: &hash,
                display_name: "Memberless",
            },
        )
        .await
        .expect("cannot create the account");
        tx.commit().await.expect("cannot commit the account");
    }
    account(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let app = app(db.database());

    // The answer of an absent account and of an account with no membership is identical.
    let (s1, _, b1) = sign_in(&app, "nobody", "correct horse battery").await;
    let (s2, _, b2) = sign_in(&app, "memberless", "correct horse battery").await;
    let (s3, _, b3) = sign_in(&app, "alice", "wrong password value").await;
    for (status, body) in [(s1, &b1), (s2, &b2), (s3, &b3)] {
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            body.get("code").and_then(Value::as_str),
            Some("invalid_credentials")
        );
    }

    // The time path is identical: the absent account verifies a dummy hash, so both requests
    // spend the Argon2id time. A missing hash would answer in well under a millisecond.
    let mut absent = Duration::MAX;
    let mut memberless = Duration::MAX;
    for _ in 0..3 {
        let start = Instant::now();
        let _ = sign_in(&app, "nobody", "correct horse battery").await;
        absent = absent.min(start.elapsed());
        let start = Instant::now();
        let _ = sign_in(&app, "memberless", "correct horse battery").await;
        memberless = memberless.min(start.elapsed());
    }
    assert!(
        absent >= Duration::from_millis(1),
        "the dummy hash must run for an absent account: {absent:?}"
    );
    assert!(
        memberless >= Duration::from_millis(1),
        "the real hash must run for a memberless account: {memberless:?}"
    );
    let ratio = absent.as_secs_f64() / memberless.as_secs_f64();
    assert!(
        (0.2..=5.0).contains(&ratio),
        "the timing paths differ: absent {absent:?}, memberless {memberless:?}"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_07_the_shared_account_guard_and_the_taken_username() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let alpha = create_tenant(db.database(), "ac-au-07-a").await;
    let beta = create_tenant(db.database(), "ac-au-07-b").await;
    let shared = account(
        db.database(),
        alpha,
        "shared",
        "correct horse battery",
        "viewer",
    )
    .await;
    add_membership(db.database(), beta, shared, "viewer").await;
    let admin = seed_session(db.database(), alpha, "admin", "admin").await;
    let app = app(db.database());

    // A tenant Admin cannot reset the password of a shared account.
    let (status, _, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/users/{shared}/password"),
            Some(&admin),
            Some(json!({"new_password": "another long password"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("shared_account")
    );

    // A tenant Admin cannot change the display name of a shared account.
    let (status, _, body) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/users/{shared}"),
            Some(&admin),
            Some(json!({"display_name": "Renamed"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("shared_account")
    );

    // A tenant Admin cannot disable the account itself: the platform route refuses it.
    let (status, _, _) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/platform/users/{shared}"),
            Some(&admin),
            Some(json!({"disabled": true})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A create with a username that exists in another tenant gives HTTP 409 `username_taken`.
    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/users",
            Some(&admin),
            Some(json!({
                "username": "shared",
                "password": "a long enough password",
                "display_name": "Shared",
                "role": "viewer",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("username_taken")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_05_a_concurrent_switch_keeps_one_session() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let alpha = create_tenant(db.database(), "ac-au-conc-a").await;
    let beta = create_tenant(db.database(), "ac-au-conc-b").await;
    let user = account(
        db.database(),
        alpha,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    add_membership(db.database(), beta, user, "viewer").await;
    let app = app(db.database());

    let (status, cookie, _) = sign_in(&app, "alice", "correct horse battery").await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("a session cookie");
    let body = json!({"tenant_id": beta.to_string()});
    let (first, second) = tokio::join!(
        send(
            &app,
            request(
                "POST",
                "/api/v1/auth/switch",
                Some(&cookie),
                Some(body.clone())
            )
        ),
        send(
            &app,
            request("POST", "/api/v1/auth/switch", Some(&cookie), Some(body))
        ),
    );
    // Exactly one switch wins; the other finds the source session already removed.
    let statuses = [first.0, second.0];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1,
        "exactly one switch must succeed: {statuses:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::UNAUTHORIZED)
            .count(),
        1,
        "the other switch must find the source session gone: {statuses:?}"
    );
    // The old cookie is dead and exactly one session of the account remains.
    let (status, _, _) = send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(count_sessions(&db, beta, user).await, 1);

    db.cleanup().await;
}

#[tokio::test]
async fn a_suspended_tenant_is_skipped_and_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let alpha = create_tenant(db.database(), "ac-au-susp-a").await;
    let beta = create_tenant(db.database(), "ac-au-susp-b").await;
    let user = account(
        db.database(),
        alpha,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    add_membership(db.database(), beta, user, "viewer").await;
    common::set_tenant_status(db.database(), alpha, "suspended").await;
    let app = app(db.database());

    // Sign-in skips the suspended tenant and uses the active one.
    let (status, cookie, body) = sign_in(&app, "alice", "correct horse battery").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body.pointer("/tenant/name").and_then(Value::as_str),
        Some("ac-au-susp-b")
    );
    let cookie = cookie.expect("a session cookie");

    // A switch to the suspended tenant is refused.
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/auth/switch",
            Some(&cookie),
            Some(json!({"tenant_id": alpha.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // An account whose only membership is suspended cannot sign in.
    let only_suspended = account(
        db.database(),
        alpha,
        "bob",
        "correct horse battery",
        "viewer",
    )
    .await;
    let _ = only_suspended;
    let (status, _, body) = sign_in(&app, "bob", "correct horse battery").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("invalid_credentials")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_tenant_admin_reads_only_its_members() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let alpha = create_tenant(db.database(), "ac-au-read-a").await;
    let beta = create_tenant(db.database(), "ac-au-read-b").await;
    let admin = seed_session(db.database(), alpha, "admin", "admin").await;
    // Member of tenant beta only.
    account(
        db.database(),
        beta,
        "foreign",
        "correct horse battery",
        "viewer",
    )
    .await;
    let app = app(db.database());

    let (status, _, page) = send(&app, request("GET", "/api/v1/users", Some(&admin), None)).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(
        !page.to_string().contains("foreign"),
        "the list holds a member of another tenant: {page}"
    );

    // A read of the foreign account gives HTTP 404.
    let foreign = common::list_users(db.database(), beta)
        .await
        .first()
        .map(|member| member.id)
        .expect("tenant beta holds a member");
    let (status, _, _) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/users/{foreign}"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    db.cleanup().await;
}

#[tokio::test]
async fn a_platform_admin_adds_a_membership_for_an_existing_account() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let target = create_tenant(db.database(), "ac-au-platform").await;
    // An existing account with a membership in another tenant.
    let other = create_tenant(db.database(), "ac-au-platform-other").await;
    let user = account(
        db.database(),
        other,
        "existing",
        "correct horse battery",
        "viewer",
    )
    .await;
    let platform = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let app = app(db.database());

    let (status, _, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/platform/users/{user}/memberships"),
            Some(&platform),
            Some(json!({"tenant_id": target.to_string(), "role": "operator"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // The account is now a member of the target tenant.
    let role = common::role_of(db.database(), target, user).await;
    assert_eq!(role, Some(restichub_db::Role::Operator));

    db.cleanup().await;
}

#[tokio::test]
async fn a_platform_admin_disables_an_account() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "ac-au-disable").await;
    let user = account(
        db.database(),
        tenant,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let cookie = session_cookie(db.database(), tenant, user).await;
    let platform = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let app = app(db.database());

    let (status, _, body) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/platform/users/{user}"),
            Some(&platform),
            Some(json!({"disabled": true})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // The disabled account loses its sessions and cannot sign in.
    let (status, _, _) = send(&app, request("GET", "/api/v1/auth/me", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = sign_in(&app, "alice", "correct horse battery").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    db.cleanup().await;
}

/// The absolute expiry of the session that the cookie names, read through the owner connection.
async fn session_expiry(
    db: &restichub_db::test_util::TestDatabase,
    tenant: TenantId,
    cookie: &str,
) -> time::OffsetDateTime {
    let value = cookie.split('=').nth(1).expect("the cookie holds a value");
    let hash = restichub_cp::token::hash_token(value);
    let mut tx = db
        .owner_pool()
        .begin()
        .await
        .expect("cannot begin the transaction");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    let (expires,): (time::OffsetDateTime,) =
        sqlx::query_as("SELECT expires_at FROM sessions WHERE token_hash = $1")
            .bind(&hash)
            .fetch_one(&mut *tx)
            .await
            .expect("cannot read the session");
    tx.commit().await.expect("cannot commit the read");
    expires
}

/// The number of sessions of an account in a tenant, read through the owner connection.
async fn count_sessions(
    db: &restichub_db::test_util::TestDatabase,
    tenant: TenantId,
    user: restichub_db::UserId,
) -> i64 {
    let mut tx = db
        .owner_pool()
        .begin()
        .await
        .expect("cannot begin the transaction");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM sessions WHERE user_id = $1")
        .bind(user)
        .fetch_one(&mut *tx)
        .await
        .expect("cannot count the sessions");
    tx.commit().await.expect("cannot commit the count");
    count
}

#[tokio::test]
async fn ac_au_07_a_suspended_tenant_still_shares_the_account() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let alpha = create_tenant(db.database(), "ac-au-share-a").await;
    let beta = create_tenant(db.database(), "ac-au-share-b").await;
    let shared = account(
        db.database(),
        alpha,
        "shared",
        "correct horse battery",
        "viewer",
    )
    .await;
    add_membership(db.database(), beta, shared, "viewer").await;
    common::set_tenant_status(db.database(), beta, "suspended").await;
    let admin = seed_session(db.database(), alpha, "admin", "admin").await;
    let app = app(db.database());

    let (status, _, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/users/{shared}/password"),
            Some(&admin),
            Some(json!({"new_password": "another long password"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("shared_account")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_07_a_disabled_membership_still_shares_the_account() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let alpha = create_tenant(db.database(), "ac-au-share2-a").await;
    let beta = create_tenant(db.database(), "ac-au-share2-b").await;
    let shared = account(
        db.database(),
        alpha,
        "shared",
        "correct horse battery",
        "viewer",
    )
    .await;
    add_membership(db.database(), beta, shared, "viewer").await;
    // A disabled membership of tenant beta still shares the account.
    {
        use restichub_db::memberships;
        let mut tx = db
            .database()
            .begin_tenant(beta)
            .await
            .expect("cannot begin the transaction");
        memberships::set_disabled(&mut tx, shared, true)
            .await
            .expect("cannot disable the membership");
        tx.commit().await.expect("cannot commit the membership");
    }
    let admin = seed_session(db.database(), alpha, "admin", "admin").await;
    let app = app(db.database());

    // Refuse the password reset.
    let (status, _, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/users/{shared}/password"),
            Some(&admin),
            Some(json!({"new_password": "another long password"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("shared_account")
    );

    // Refuse the display name change too.
    let (status, _, body) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/users/{shared}"),
            Some(&admin),
            Some(json!({"display_name": "Renamed"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("shared_account")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_05_the_switch_keeps_the_absolute_expiry() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let alpha = create_tenant(db.database(), "ac-au-exp-a").await;
    let beta = create_tenant(db.database(), "ac-au-exp-b").await;
    let user = account(
        db.database(),
        alpha,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    add_membership(db.database(), beta, user, "viewer").await;
    let app = app(db.database());

    let (status, cookie, _) = sign_in(&app, "alice", "correct horse battery").await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("a session cookie");
    let before = session_expiry(&db, alpha, &cookie).await;

    let (status, new_cookie, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/auth/switch",
            Some(&cookie),
            Some(json!({"tenant_id": beta.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let new_cookie = new_cookie.expect("a rotated session cookie");
    let after = session_expiry(&db, beta, &new_cookie).await;
    assert_eq!(after, before, "the switch must keep the absolute expiry");

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_05_a_switch_to_the_current_tenant_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let alpha = create_tenant(db.database(), "ac-au-same").await;
    account(
        db.database(),
        alpha,
        "alice",
        "correct horse battery",
        "admin",
    )
    .await;
    let app = app(db.database());

    let (status, cookie, _) = sign_in(&app, "alice", "correct horse battery").await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("a session cookie");
    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/auth/switch",
            Some(&cookie),
            Some(json!({"tenant_id": alpha.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body.get("field").and_then(Value::as_str), Some("tenant_id"));

    db.cleanup().await;
}

#[tokio::test]
async fn ac_au_07_a_role_change_keeps_a_disabled_membership_disabled() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "ac-au-role").await;
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let bob = account(
        db.database(),
        tenant,
        "bob",
        "correct horse battery",
        "viewer",
    )
    .await;
    let app = app(db.database());

    // Disable the membership and change the role in one request.
    let (status, _, body) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/users/{bob}"),
            Some(&admin),
            Some(json!({"disabled": true, "role": "operator"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body.get("disabled").and_then(Value::as_bool), Some(true));
    assert_eq!(body.get("role").and_then(Value::as_str), Some("operator"));

    // The membership stays disabled, so bob cannot sign in.
    let (status, _, _) = sign_in(&app, "bob", "correct horse battery").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    db.cleanup().await;
}

#[tokio::test]
async fn the_last_platform_admin_and_the_caller_are_protected() {
    let Some(db) = common::test_db().await else {
        return;
    };
    // The caller is a platform API token whose creator is a Platform Admin.
    let token = common::seed_platform_api_token(db.database()).await;
    let second = common::create_user(
        db.database(),
        PLATFORM_TENANT_ID,
        "second-admin",
        "phc",
        "platform_admin",
    )
    .await;
    let app = app(db.database());

    // Disabling the second Platform Admin leaves one active one and succeeds.
    let (status, _, body) = send(
        &app,
        common::bearer_request(
            "PATCH",
            &format!("/api/v1/platform/users/{second}"),
            &token,
            Some(json!({"disabled": true})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    // Disabling the last active Platform Admin is refused.
    let creator = common::list_users(db.database(), PLATFORM_TENANT_ID)
        .await
        .into_iter()
        .find(|member| member.username == "platform-token-owner")
        .map(|member| member.id)
        .expect("the token creator is a member");
    let (status, _, _) = send(
        &app,
        common::bearer_request(
            "PATCH",
            &format!("/api/v1/platform/users/{creator}"),
            &token,
            Some(json!({"disabled": true})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    db.cleanup().await;
}

#[tokio::test]
async fn a_platform_admin_cannot_disable_its_own_account() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let root = common::create_user(
        db.database(),
        PLATFORM_TENANT_ID,
        "root",
        "phc",
        "platform_admin",
    )
    .await;
    let cookie = common::session_cookie(db.database(), PLATFORM_TENANT_ID, root).await;
    let app = app(db.database());

    let (status, _, _) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/platform/users/{root}"),
            Some(&cookie),
            Some(json!({"disabled": true})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
}

#[tokio::test]
async fn add_membership_refuses_the_platform_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let root = common::create_user(
        db.database(),
        PLATFORM_TENANT_ID,
        "root",
        "phc",
        "platform_admin",
    )
    .await;
    let cookie = common::session_cookie(db.database(), PLATFORM_TENANT_ID, root).await;
    let app = app(db.database());

    let (status, _, body) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/platform/users/{root}/memberships"),
            Some(&cookie),
            Some(json!({"tenant_id": PLATFORM_TENANT_ID.to_string(), "role": "admin"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    db.cleanup().await;
}
