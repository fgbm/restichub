//! AC-TN-06: the single-tenant mode and the switch between the two modes (FR-TN-06, ADR-0024).

#![allow(clippy::expect_used)]

mod common;

use axum::Router;
use axum::http::StatusCode;
use restichub_cp::clock::SystemClock;
use restichub_cp::config::TenancyConfig;
use restichub_cp::routes::ROUTES;
use restichub_cp::server::{AppState, router};
use restichub_cp::setup::ensure_setup_token;
use restichub_cp::startup::ensure_tenancy_mode;
use restichub_db::test_util::TestDatabase;
use restichub_db::{Database, PLATFORM_TENANT_ID, Role, TenantId, setup, tenants};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use common::{
    app, app_single_with_kek, app_with_kek, count, create_tenant, request, seed_platform_api_token,
    seed_session, send, set_tenant_status,
};

const ROLES: [Role; 4] = [
    Role::PlatformAdmin,
    Role::Admin,
    Role::Operator,
    Role::Viewer,
];

async fn tenant_by_name(db: &Database, name: &str) -> TenantId {
    let mut tx = db.begin_platform().await.expect("the database call failed");
    let tenant = tenants::find_by_name(&mut tx, name)
        .await
        .expect("the database call failed")
        .expect("the tenant is missing");
    tx.commit().await.expect("the database call failed");
    tenant.id
}

async fn tenants_named(db: &Database, name: &str) -> usize {
    let mut tx = db.begin_platform().await.expect("the database call failed");
    let list = tenants::list(&mut tx)
        .await
        .expect("the database call failed");
    tx.commit().await.expect("the database call failed");
    list.iter()
        .filter(|tenant| tenant.name.eq_ignore_ascii_case(name))
        .count()
}

/// An app in single mode with a given pool size, so a test can exhaust the pool.
async fn small_pool_single_app(db: &TestDatabase, max: u32) -> Router {
    let database = db
        .database_with_max(max)
        .await
        .expect("cannot open the small pool");
    router(
        AppState::new(database)
            .with_kek(common::test_kek())
            .with_tenancy(TenancyConfig::default()),
    )
}

fn setup_body(token: &str, username: &str) -> Value {
    json!({
        "setup_token": token,
        "username": username,
        "password": "a long enough password",
        "display_name": "Admin",
    })
}

/// Inserts a setup token in the platform tenant and returns its value.
///
/// The server issues the same token at start; a test needs the value, so it inserts one.
async fn insert_setup_token(db: &Database) -> String {
    let value = restichub_cp::token::random_token().expect("cannot build the token");
    let hash = restichub_cp::token::hash_token(&value);
    let expires_at = OffsetDateTime::now_utc() + time::Duration::hours(24);
    let mut tx = db
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .expect("the database call failed");
    setup::create(&mut tx, &hash, expires_at)
        .await
        .expect("the database call failed");
    tx.commit().await.expect("the database call failed");
    value
}

fn sign_in_body(tenant: Option<&str>, username: &str) -> Value {
    match tenant {
        Some(tenant) => {
            json!({"tenant": tenant, "username": username, "password": "a long enough password"})
        }
        None => json!({"username": username, "password": "a long enough password"}),
    }
}

#[tokio::test]
async fn ac_tn_06_single_mode_setup_and_sign_in() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app_single_with_kek(db.database());

    // The instance reports the single mode and that the setup is required.
    let (status, _, body) = send(&app, request("GET", "/api/v1/instance", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("tenancy_mode").and_then(Value::as_str),
        Some("single")
    );
    assert_eq!(
        body.get("setup_required").and_then(Value::as_bool),
        Some(true)
    );

    // The setup needs the one-time token and creates the tenant `default` with its first Admin.
    let token = insert_setup_token(db.database()).await;
    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/setup",
            None,
            Some(setup_body(&token, "admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created.get("role").and_then(Value::as_str), Some("admin"));

    // The tenant `default` holds a key row and one user.
    let tenant = tenant_by_name(db.database(), "default").await;
    assert_eq!(count(db.database(), tenant, "tenant_keys").await, 1);
    assert_eq!(count(db.database(), tenant, "users").await, 1);

    // The setup is done and the status reports the mode.
    let (_, _, body) = send(&app, request("GET", "/api/v1/setup/status", None, None)).await;
    assert_eq!(
        body.get("setup_required").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        body.get("tenancy_mode").and_then(Value::as_str),
        Some("single")
    );

    // Sign-in needs no tenant and signs the Admin of `default` in.
    let (status, cookie, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/auth/sign-in",
            None,
            Some(sign_in_body(None, "admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let cookie = cookie.expect("a session cookie");
    assert_eq!(
        body.pointer("/tenant/name").and_then(Value::as_str),
        Some("default")
    );

    // The platform routes are 404 with the session of a real user.
    let (status, _, problem) = send(
        &app,
        request("GET", "/api/v1/platform/tenants", Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("not_found")
    );

    // An object created through the API belongs to the tenant `default`.
    let body = json!({
        "username": "bob",
        "password": "a long enough password",
        "display_name": "Bob",
        "role": "viewer",
    });
    let (status, _, _) = send(
        &app,
        request("POST", "/api/v1/users", Some(&cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(count(db.database(), tenant, "users").await, 2);

    db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_06_single_mode_setup_requires_the_token() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app_single_with_kek(db.database());

    // A body with no token is refused with the same code as multi mode.
    let no_token = json!({
        "username": "admin",
        "password": "a long enough password",
        "display_name": "Admin",
    });
    let (status, _, problem) =
        send(&app, request("POST", "/api/v1/setup", None, Some(no_token))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("invalid_argument")
    );
    assert_eq!(
        problem.get("field").and_then(Value::as_str),
        Some("setup_token")
    );

    // The token is not in the database, so the endpoint refuses it.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/setup",
            None,
            Some(setup_body("not-a-real-token", "admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("invalid_credentials")
    );

    // The setup is still required and no tenant exists.
    let (_, _, body) = send(&app, request("GET", "/api/v1/setup/status", None, None)).await;
    assert_eq!(
        body.get("setup_required").and_then(Value::as_bool),
        Some(true)
    );

    // The right token creates the tenant and its first Admin.
    let token = insert_setup_token(db.database()).await;
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/setup",
            None,
            Some(setup_body(&token, "admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // A second call with the same token is refused, because the tenant exists.
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/setup",
            None,
            Some(setup_body(&token, "root")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("conflict")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_06_switch_to_multi_issues_a_platform_setup_token() {
    let Some(db) = common::test_db().await else {
        return;
    };
    // A single-mode installation with its first Admin.
    let single = app_single_with_kek(db.database());
    let token = insert_setup_token(db.database()).await;
    let (status, _, _) = send(
        &single,
        request(
            "POST",
            "/api/v1/setup",
            None,
            Some(setup_body(&token, "admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // The switch to multi mode: the platform tenant has no user, so a token is issued.
    ensure_setup_token(db.database(), &SystemClock, &TenancyConfig::multi())
        .await
        .expect("the database call failed");
    assert_eq!(
        count(db.database(), PLATFORM_TENANT_ID, "setup_tokens").await,
        2
    );

    // The data of `default` stays available: sign-in with the tenant in multi mode works.
    let multi = app_with_kek(db.database());
    let (status, _, body) = send(
        &multi,
        request(
            "POST",
            "/api/v1/auth/sign-in",
            None,
            Some(sign_in_body(Some("default"), "admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body.pointer("/tenant/name").and_then(Value::as_str),
        Some("default")
    );

    // The platform setup runs with a token; the platform tenant has no user after the switch.
    let platform_token = insert_setup_token(db.database()).await;
    let (status, _, created) = send(
        &multi,
        request(
            "POST",
            "/api/v1/setup",
            None,
            Some(setup_body(&platform_token, "root")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        created.get("role").and_then(Value::as_str),
        Some("platform_admin")
    );

    // The platform routes answer again. The Platform Admin reads the tenant list, which holds
    // the tenant `default` of the single-mode installation.
    let (status, cookie, _) = send(
        &multi,
        request(
            "POST",
            "/api/v1/auth/sign-in",
            None,
            Some(sign_in_body(Some("platform"), "root")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cookie = cookie.expect("a session cookie");
    let (status, _, list) = send(
        &multi,
        request("GET", "/api/v1/platform/tenants", Some(&cookie), None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the platform routes answer in multi mode"
    );
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
    assert!(names.iter().any(|name| name == "default"), "{names:?}");

    db.cleanup().await;
}

#[tokio::test]
async fn multi_to_single_refuses_with_two_active_tenants() {
    let Some(db) = common::test_db().await else {
        return;
    };
    // A fresh installation with no tenant is allowed: the setup creates `default` later.
    assert!(
        ensure_tenancy_mode(db.database(), &TenancyConfig::single("default"))
            .await
            .is_ok()
    );

    create_tenant(db.database(), "default").await;
    let other = create_tenant(db.database(), "other").await;
    // Two active tenants: the switch is refused and the error names the problem.
    let error = ensure_tenancy_mode(db.database(), &TenancyConfig::single("default"))
        .await
        .expect_err("two active tenants must refuse the switch");
    assert!(
        error.to_string().contains("exactly one active tenant"),
        "{error}"
    );

    // A suspended tenant does not block the switch.
    set_tenant_status(db.database(), other, "suspended").await;
    assert!(
        ensure_tenancy_mode(db.database(), &TenancyConfig::single("default"))
            .await
            .is_ok()
    );

    // An active tenant of another name is refused unless the setting names it.
    let default = tenant_by_name(db.database(), "default").await;
    set_tenant_status(db.database(), default, "suspended").await;
    set_tenant_status(db.database(), other, "active").await;
    assert!(
        ensure_tenancy_mode(db.database(), &TenancyConfig::single("default"))
            .await
            .is_err()
    );
    assert!(
        ensure_tenancy_mode(db.database(), &TenancyConfig::single("other"))
            .await
            .is_ok()
    );

    db.cleanup().await;
}

#[tokio::test]
async fn multi_mode_requires_the_tenant_on_sign_in() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app(db.database());
    let (status, _, problem) = send(
        &app,
        request(
            "POST",
            "/api/v1/auth/sign-in",
            None,
            Some(sign_in_body(None, "admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("invalid_argument")
    );
    assert_eq!(problem.get("field").and_then(Value::as_str), Some("tenant"));

    db.cleanup().await;
}

#[tokio::test]
async fn the_instance_endpoint_reports_the_mode_in_both_modes() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let multi = app(db.database());
    let (status, _, body) = send(&multi, request("GET", "/api/v1/instance", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("tenancy_mode").and_then(Value::as_str),
        Some("multi")
    );
    assert_eq!(
        body.get("setup_required").and_then(Value::as_bool),
        Some(true)
    );

    let single = app_single_with_kek(db.database());
    let (status, _, body) = send(&single, request("GET", "/api/v1/instance", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body.get("tenancy_mode").and_then(Value::as_str),
        Some("single")
    );
    assert_eq!(
        body.get("setup_required").and_then(Value::as_bool),
        Some(true)
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_06_single_mode_platform_routes_are_404_for_every_role() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app_single_with_kek(db.database());
    let tenant = create_tenant(db.database(), "default").await;
    let bearer = seed_platform_api_token(db.database()).await;
    let id = Uuid::now_v7().to_string();
    let mut index = 0;

    for route in ROUTES {
        if !route.access.is_platform() {
            continue;
        }
        let path = route.path.replace("{id}", &id);
        let method = route.method.to_ascii_uppercase();
        for role in ROLES {
            index += 1;
            let session_tenant = if role == Role::PlatformAdmin {
                PLATFORM_TENANT_ID
            } else {
                tenant
            };
            let cookie = seed_session(
                db.database(),
                session_tenant,
                &format!("user-{}-{index}", role.as_str()),
                role.as_str(),
            )
            .await;
            let (status, _, problem) =
                send(&app, request(&method, &path, Some(&cookie), None)).await;
            assert_eq!(
                status,
                StatusCode::NOT_FOUND,
                "{method} {path} as {}",
                role.as_str()
            );
            assert_eq!(
                problem.get("code").and_then(Value::as_str),
                Some("not_found"),
                "{method} {path} as {}",
                role.as_str()
            );
        }
        // A request with no credential and a request with a bearer token give the same 404.
        let (status, _, _) = send(&app, request(&method, &path, None, None)).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "no credential {method} {path}"
        );
        let (status, _, _) =
            send(&app, common::bearer_request(&method, &path, &bearer, None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "bearer {method} {path}");
    }

    db.cleanup().await;
}

#[tokio::test]
async fn two_concurrent_single_mode_setups_leave_one_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app_single_with_kek(db.database());
    let token = insert_setup_token(db.database()).await;
    let body = setup_body(&token, "admin");
    let (first, second) = tokio::join!(
        send(
            &app,
            request("POST", "/api/v1/setup", None, Some(body.clone()))
        ),
        send(&app, request("POST", "/api/v1/setup", None, Some(body)))
    );
    let statuses = [first.0, second.0];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "{statuses:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CONFLICT)
            .count(),
        1,
        "{statuses:?}"
    );

    let tenant = tenant_by_name(db.database(), "default").await;
    assert_eq!(count(db.database(), tenant, "users").await, 1);
    assert_eq!(tenants_named(db.database(), "default").await, 1);

    db.cleanup().await;
}

#[tokio::test]
async fn single_mode_sign_in_ignores_the_tenant_field() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app_single_with_kek(db.database());
    let token = insert_setup_token(db.database()).await;
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/setup",
            None,
            Some(setup_body(&token, "admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // A request that names the platform tenant still signs the Admin of `default` in.
    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/auth/sign-in",
            None,
            Some(sign_in_body(Some("platform"), "admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body.pointer("/tenant/name").and_then(Value::as_str),
        Some("default")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ensure_tenancy_mode_names_a_deleted_default_tenant() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let default = create_tenant(db.database(), "default").await;
    set_tenant_status(db.database(), default, "deleted").await;
    let error = ensure_tenancy_mode(db.database(), &TenancyConfig::single("default"))
        .await
        .expect_err("a deleted tenant must refuse the switch");
    let message = error.to_string();
    assert!(
        message.contains("default") && message.contains("deleted"),
        "{message}"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn a_successful_setup_marks_every_unused_token_used() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let single = app_single_with_kek(db.database());
    let first = insert_setup_token(db.database()).await;
    let second = insert_setup_token(db.database()).await;
    let (status, _, _) = send(
        &single,
        request(
            "POST",
            "/api/v1/setup",
            None,
            Some(setup_body(&first, "admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // The switch to multi mode leaves the platform tenant without a user, so the setup is
    // required again. The second token must be refused: the single-mode setup marked it used.
    let multi = app_with_kek(db.database());
    let (status, _, problem) = send(
        &multi,
        request(
            "POST",
            "/api/v1/setup",
            None,
            Some(setup_body(&second, "root")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        problem.get("code").and_then(Value::as_str),
        Some("invalid_credentials")
    );

    db.cleanup().await;
}

#[tokio::test]
async fn single_mode_platform_credentials_are_unauthenticated() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = app_single_with_kek(db.database());
    let cookie = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let bearer = seed_platform_api_token(db.database()).await;

    for path in ["/api/v1/users", "/api/v1/audit", "/api/v1/auth/me"] {
        let (status, _, _) = send(&app, request("GET", path, Some(&cookie), None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "cookie {path}");
        let (status, _, _) = send(&app, common::bearer_request("GET", path, &bearer, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "bearer {path}");
    }

    db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_06_single_mode_setup_with_a_small_pool_does_not_exhaust_it() {
    let Some(db) = common::test_db().await else {
        return;
    };
    // Complete the setup on the usual pool.
    let setup_app = app_single_with_kek(db.database());
    let token = insert_setup_token(db.database()).await;
    let (status, _, _) = send(
        &setup_app,
        request(
            "POST",
            "/api/v1/setup",
            None,
            Some(setup_body(&token, "admin")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // A pool of two connections. A repeated setup must not hold one connection while it waits
    // for another, or the call stalls until the acquire timeout.
    let app = small_pool_single_app(&db, 2).await;
    let body = setup_body("root", "admin2");
    let calls = async {
        tokio::join!(
            send(
                &app,
                request("POST", "/api/v1/setup", None, Some(body.clone()))
            ),
            send(
                &app,
                request("POST", "/api/v1/setup", None, Some(body.clone()))
            ),
            send(
                &app,
                request("POST", "/api/v1/setup", None, Some(body.clone()))
            ),
            send(&app, request("GET", "/readyz", None, None))
        )
    };
    let (first, second, third, ready) =
        tokio::time::timeout(std::time::Duration::from_secs(10), calls)
            .await
            .expect("the repeated setup must not wait for a connection");
    for result in [first, second, third] {
        assert_eq!(result.0, StatusCode::CONFLICT, "a repeated setup");
    }
    assert_eq!(ready.0, StatusCode::OK, "the readiness probe");

    db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_06_single_mode_wrong_tokens_do_not_block_the_setup() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let app = small_pool_single_app(&db, 2).await;
    let token = insert_setup_token(db.database()).await;
    let wrong = |username: &str| setup_body("not-a-real-token", username);
    let calls = async {
        tokio::join!(
            send(
                &app,
                request("POST", "/api/v1/setup", None, Some(wrong("a")))
            ),
            send(
                &app,
                request("POST", "/api/v1/setup", None, Some(wrong("b")))
            ),
            send(
                &app,
                request("POST", "/api/v1/setup", None, Some(wrong("c")))
            ),
            send(
                &app,
                request(
                    "POST",
                    "/api/v1/setup",
                    None,
                    Some(setup_body(&token, "admin"))
                )
            )
        )
    };
    let (first, second, third, correct) =
        tokio::time::timeout(std::time::Duration::from_secs(10), calls)
            .await
            .expect("a wrong token must not block the setup");
    let statuses = [first.0, second.0, third.0, correct.0];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "{statuses:?}"
    );
    for status in statuses {
        assert!(
            matches!(status, StatusCode::CREATED) || status.is_client_error(),
            "{status}"
        );
    }
    let tenant = tenant_by_name(db.database(), "default").await;
    assert_eq!(count(db.database(), tenant, "users").await, 1);

    db.cleanup().await;
}
