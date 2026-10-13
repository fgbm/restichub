//! The platform tenant administration endpoints (FR-TN-02).
//!
//! The tests cover the create, the list, the read, the suspend, the resume and the delete. They
//! check the name rules, the state machine, the platform tenant protections, the authorization, the
//! audit records in the two tenants and the wrapped data encryption key.

mod common;

use axum::Router;
use axum::http::StatusCode;
use restichub_cp::password;
use restichub_db::audit::{AuditAction, AuditFilter, AuditPage, AuditRecord};
use restichub_db::{PLATFORM_TENANT_ID, TenantId, audit, kek};
use restichub_secrets::{KekId, WrappedDek, unwrap_dek};
use serde_json::{Value, json};

use common::{
    app_with_kek, create_tenant, create_user, request, seed_api_token, seed_session, send,
    session_cookie, test_kek,
};

/// Returns a root session cookie of the platform tenant.
async fn platform_admin(db: &restichub_db::Database) -> String {
    seed_session(db, PLATFORM_TENANT_ID, "root", "platform_admin").await
}

/// Builds a create-tenant body. The first admin username is unique in the installation (ADR-0025).
fn create_body(name: &str) -> Value {
    let username: String = format!("admin-{}", name.chars().take(20).collect::<String>());
    json!({
        "name": name,
        "first_admin": {
            "username": username,
            "display_name": "Admin",
            "password": "a long enough password",
        },
    })
}

/// Creates a tenant through the API and returns its identifier.
async fn create_tenant_ok(app: &Router, cookie: &str, name: &str) -> String {
    let (status, _, body) = send(
        app,
        request(
            "POST",
            "/api/v1/platform/tenants",
            Some(cookie),
            Some(create_body(name)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body.get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("the response holds no identifier: {body}"))
}

/// Reads the records of one action in a tenant.
async fn tenant_audit(
    db: &restichub_db::Database,
    tenant: TenantId,
    action: AuditAction,
) -> Vec<AuditRecord> {
    let mut tx = db
        .begin_tenant(tenant)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let filter = AuditFilter {
        action: Some(action),
        ..AuditFilter::default()
    };
    let records = audit::list(&mut tx, &filter, &AuditPage::default())
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .records;
    tx.commit().await.unwrap_or_else(|error| panic!("{error}"));
    records
}

/// Reads the items of a platform query string.
async fn platform_audit(app: &Router, cookie: &str, query: &str) -> Vec<Value> {
    let uri = format!("/api/v1/platform/audit?{query}");
    let (status, _, page) = send(app, request("GET", &uri, Some(cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    page.get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[tokio::test]
async fn create_list_and_get_a_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());

    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/platform/tenants",
            Some(&cookie),
            Some(create_body("acme")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body.get("name").and_then(Value::as_str), Some("acme"));
    assert_eq!(body.get("status").and_then(Value::as_str), Some("active"));
    assert_eq!(body.get("user_count").and_then(Value::as_u64), Some(1));
    assert!(body.get("created_at").and_then(Value::as_str).is_some());
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    // The list holds the tenant and the user count.
    let (status, _, list) = send(
        &app,
        request("GET", "/api/v1/platform/tenants", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let items = list
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no items: {list}"));
    let found = items
        .iter()
        .find(|item| item.get("id").and_then(Value::as_str) == Some(id.as_str()))
        .unwrap_or_else(|| panic!("the list holds no created tenant: {list}"));
    assert_eq!(found.get("user_count").and_then(Value::as_u64), Some(1));

    let (status, _, one) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/platform/tenants/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{one}");
    assert_eq!(one.get("name").and_then(Value::as_str), Some("acme"));

    let body = json!({"username": "admin-acme", "password": "a long enough password"});
    let (status, _, _) = send(
        &app,
        request("POST", "/api/v1/auth/sign-in", None, Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    db.cleanup().await;
}

#[tokio::test]
async fn the_name_rules_are_enforced() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());

    for name in [
        "",
        "1acme",
        "-acme",
        "Acme",
        "acme_1",
        "with space",
        "platform",
    ] {
        let (status, _, body) = send(
            &app,
            request(
                "POST",
                "/api/v1/platform/tenants",
                Some(&cookie),
                Some(create_body(name)),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "the name `{name}`: {body}");
        // A validation problem names the field and gives a reason code, with no user input.
        assert_eq!(
            body.get("field").and_then(Value::as_str),
            Some("name"),
            "the name `{name}`: {body}"
        );
        assert!(
            body.get("reason").and_then(Value::as_str).is_some(),
            "the name `{name}`: {body}"
        );
    }

    let long = "a".repeat(63);
    let _ = create_tenant_ok(&app, &cookie, &long).await;

    db.cleanup().await;
}

#[tokio::test]
async fn a_duplicate_name_is_a_conflict() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());

    let _ = create_tenant_ok(&app, &cookie, "duplicate").await;

    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/platform/tenants",
            Some(&cookie),
            Some(create_body("DUPLICATE")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    // The uppercase name is refused by the validation first, so a lower-case duplicate is used.
    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/platform/tenants",
            Some(&cookie),
            Some(create_body("duplicate")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    db.cleanup().await;
}

#[tokio::test]
async fn suspend_resume_and_delete_follow_the_state_machine() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let id = create_tenant_ok(&app, &cookie, "flow").await;

    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/platform/tenants/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Suspend and suspend again (idempotent).
    for _ in 0..2 {
        let (status, _, _) = send(
            &app,
            request(
                "POST",
                &format!("/api/v1/platform/tenants/{id}/suspend"),
                Some(&cookie),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    // Resume and resume again (idempotent).
    for _ in 0..2 {
        let (status, _, _) = send(
            &app,
            request(
                "POST",
                &format!("/api/v1/platform/tenants/{id}/resume"),
                Some(&cookie),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    // Suspend then delete.
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/platform/tenants/{id}/suspend"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/platform/tenants/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // A deleted tenant is an idempotent no-op for a second delete, and it cannot be resumed.
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/platform/tenants/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/platform/tenants/{id}/resume"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // A deleted tenant cannot be suspended either.
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/platform/tenants/{id}/suspend"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // The default list excludes the deleted tenant, and the status filter finds it.
    let (_, _, list) = send(
        &app,
        request("GET", "/api/v1/platform/tenants", Some(&cookie), None),
    )
    .await;
    let default_ids: Vec<&str> = list
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    assert!(!default_ids.contains(&id.as_str()));

    let (_, _, deleted) = send(
        &app,
        request(
            "GET",
            "/api/v1/platform/tenants?status=deleted",
            Some(&cookie),
            None,
        ),
    )
    .await;
    let deleted_ids: Vec<&str> = deleted
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    assert!(deleted_ids.contains(&id.as_str()));

    db.cleanup().await;
}

#[tokio::test]
async fn the_platform_tenant_cannot_be_suspended_or_deleted() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let platform = PLATFORM_TENANT_ID.to_string();

    for path in [
        format!("/api/v1/platform/tenants/{platform}/suspend"),
        format!("/api/v1/platform/tenants/{platform}"),
    ] {
        let method = if path.ends_with("/suspend") {
            "POST"
        } else {
            "DELETE"
        };
        let (status, _, _) = send(&app, request(method, &path, Some(&cookie), None)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{method} {path}");
    }

    db.cleanup().await;
}

#[tokio::test]
async fn a_non_platform_role_gets_403() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "other").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app_with_kek(db.database());

    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/platform/tenants", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/platform/tenants",
            Some(&cookie),
            Some(create_body("nope")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
}

#[tokio::test]
async fn the_audit_records_are_in_both_tenants() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let id = create_tenant_ok(&app, &cookie, "audited").await;
    let tenant = id
        .parse::<TenantId>()
        .unwrap_or_else(|error| panic!("{error}"));

    // The creation: `tenant_create` in the platform tenant and `user_create` in the new tenant.
    let platform_events = platform_audit(
        &app,
        &cookie,
        &format!("action=tenant_create&tenant_id={PLATFORM_TENANT_ID}"),
    )
    .await;
    assert_eq!(
        platform_events.len(),
        1,
        "the platform tenant_create is missing"
    );
    assert_eq!(
        platform_events[0].get("object_id").and_then(Value::as_str),
        Some(id.as_str())
    );
    // The record holds the actor and the source IP of the request.
    assert_eq!(
        platform_events[0].get("actor_type").and_then(Value::as_str),
        Some("user")
    );
    assert!(
        platform_events[0]
            .get("actor_id")
            .and_then(Value::as_str)
            .is_some()
    );
    assert_eq!(
        platform_events[0].get("source_ip").and_then(Value::as_str),
        Some("203.0.113.7")
    );
    let created_events = tenant_audit(db.database(), tenant, AuditAction::UserCreate).await;
    assert_eq!(
        created_events.len(),
        1,
        "the user_create of the tenant is missing"
    );
    // The `user_create` record holds the actor and the source IP too.
    assert_eq!(created_events[0].actor.actor_type(), "user");
    assert!(created_events[0].actor.actor_id().is_some());
    assert_eq!(
        created_events[0].source_ip.map(|ip| ip.to_string()),
        Some("203.0.113.7".to_owned())
    );

    // The suspend: the event is in the platform tenant and in the affected tenant.
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/platform/tenants/{id}/suspend"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let platform_events = platform_audit(
        &app,
        &cookie,
        &format!("action=tenant_suspend&tenant_id={PLATFORM_TENANT_ID}"),
    )
    .await;
    assert_eq!(
        platform_events.len(),
        1,
        "the platform tenant_suspend is missing"
    );
    let affected_events = platform_audit(
        &app,
        &cookie,
        &format!("action=tenant_suspend&tenant_id={tenant}"),
    )
    .await;
    assert_eq!(
        affected_events.len(),
        1,
        "the affected tenant_suspend is missing"
    );
    let tenant_events = tenant_audit(db.database(), tenant, AuditAction::TenantSuspend).await;
    assert_eq!(
        tenant_events.len(),
        1,
        "the tenant_suspend of the tenant is missing"
    );
    // The suspend record holds the source IP as well.
    assert_eq!(
        platform_events[0].get("source_ip").and_then(Value::as_str),
        Some("203.0.113.7")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_dek_row_unwraps_with_the_kek() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let id = create_tenant_ok(&app, &cookie, "wrapped").await;
    let tenant = id
        .parse::<TenantId>()
        .unwrap_or_else(|error| panic!("{error}"));

    let key = {
        let mut tx = db
            .database()
            .begin_tenant(tenant)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        let key = kek::read_tenant_key(&mut tx, tenant)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        tx.commit().await.unwrap_or_else(|error| panic!("{error}"));
        key.unwrap_or_else(|| panic!("the tenant holds no key row"))
    };
    assert_eq!(key.key_version, 1);

    let wrapped = WrappedDek {
        kek_id: KekId::parse(&key.kek_id).unwrap_or_else(|error| panic!("{error}")),
        nonce: key
            .nonce
            .as_slice()
            .try_into()
            .unwrap_or_else(|error| panic!("{error}")),
        ciphertext: key.wrapped_dek,
    };
    let kek = test_kek();
    assert!(
        unwrap_dek(kek.as_ref(), tenant.as_uuid(), key.key_version, &wrapped).is_ok(),
        "the DEK row must unwrap with the KEK"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_list_paginates_with_a_keyset_cursor() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    for index in 0..3 {
        let _ = create_tenant_ok(&app, &cookie, &format!("page-{index}")).await;
    }

    let (_, _, first) = send(
        &app,
        request(
            "GET",
            "/api/v1/platform/tenants?limit=2",
            Some(&cookie),
            None,
        ),
    )
    .await;
    let first_items = first
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(first_items.len(), 2);
    let cursor = first
        .get("next_cursor")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no cursor: {first}"))
        .to_owned();

    let (_, _, second) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/platform/tenants?limit=2&cursor={cursor}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    let second_items = second
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(!second_items.is_empty());

    // No tenant repeats across the two pages.
    let first_ids: Vec<&str> = first_items
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .collect();
    for item in &second_items {
        let id = item.get("id").and_then(Value::as_str);
        assert!(!first_ids.contains(&id.unwrap_or_default()));
    }

    db.cleanup().await;
}

#[tokio::test]
async fn a_deleted_tenant_is_refused_for_sign_in_session_and_token() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    let id = create_tenant_ok(&app, &cookie, "gone").await;
    let tenant = id
        .parse::<TenantId>()
        .unwrap_or_else(|error| panic!("{error}"));

    // A user with a real password, a session and an API token of the tenant.
    let hash = password::hash_password("a long enough password").unwrap_or_default();
    let user = create_user(db.database(), tenant, "alice", &hash, "admin").await;
    let session = session_cookie(db.database(), tenant, user).await;
    let token = seed_api_token(db.database(), tenant, "admin").await;

    // Suspend, then delete the tenant.
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/platform/tenants/{id}/suspend"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/platform/tenants/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let body = json!({"tenant": "gone", "username": "alice", "password": "a long enough password"});
    let (status, _, _) = send(
        &app,
        request("POST", "/api/v1/auth/sign-in", None, Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/auth/me", Some(&session), None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _, _) = send(
        &app,
        common::bearer_request("GET", "/api/v1/users", &token, None),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    db.cleanup().await;
}

#[tokio::test]
async fn a_taken_first_admin_username_is_a_username_taken_conflict() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let cookie = platform_admin(db.database()).await;
    let app = app_with_kek(db.database());
    // The first tenant holds the account `admin-first` (ADR-0025).
    let _ = create_tenant_ok(&app, &cookie, "first").await;

    let body = json!({
        "name": "second",
        "first_admin": {
            "username": "admin-first",
            "display_name": "Admin",
            "password": "a long enough password",
        },
    });
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/platform/tenants",
            Some(&cookie),
            Some(body),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("username_taken")
    );
    assert_eq!(
        problem.get("field").and_then(Value::as_str),
        Some("first_admin.username")
    );

    db.cleanup().await;
}
