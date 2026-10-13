//! AC-AU-01: the audit records hold no secret value.
//!
//! The test drives every authentication and user operation and then dumps the whole audit log. The
//! dump must hold none of the canary password, the API token value and the setup token value.

mod common;

use axum::http::StatusCode;
use restichub_cp::password;
use restichub_cp::token;
use restichub_db::{PLATFORM_TENANT_ID, setup};
use serde_json::{Value, json};
use time::OffsetDateTime;

use common::{app, count, create_tenant, request, seed_session, send};

/// The password must never appear in the audit log.
const CANARY: &str = "CANARY-secret-0001";

/// Performs a sign-in and gives the status and the cookie.
async fn sign_in(
    app: &axum::Router,
    tenant: &str,
    username: &str,
    password_value: &str,
) -> (StatusCode, Option<String>) {
    let body = json!({"tenant": tenant, "username": username, "password": password_value});
    let (status, cookie, _) = send(
        app,
        request("POST", "/api/v1/auth/sign-in", None, Some(body)),
    )
    .await;
    (status, cookie)
}

#[tokio::test]
async fn ac_au_01_audit_records_hold_no_secret() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant(db.database(), "canary").await;
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;

    let hash = password::hash_password(CANARY).unwrap_or_default();
    common::create_user(db.database(), tenant, "alice", &hash, "viewer").await;

    let app = app(db.database());

    // sign_in, sign_in_failed (known and unknown user), sign_out.
    let (status, alice_cookie) = sign_in(&app, "canary", "alice", CANARY).await;
    assert_eq!(status, StatusCode::OK);
    let _ = sign_in(&app, "canary", "alice", "wrong password value").await;
    let _ = sign_in(&app, "canary", "nobody", "wrong password value").await;
    let alice_cookie = alice_cookie.unwrap_or_default();
    let (status, _, _) = send(
        &app,
        request("POST", "/api/v1/auth/sign-out", Some(&alice_cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // user_create, user_update, password reset.
    let body = json!({
        "username": "bob",
        "password": CANARY,
        "display_name": "Bob",
        "role": "viewer",
    });
    let (status, _, created) = send(
        &app,
        request("POST", "/api/v1/users", Some(&admin), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let bob = created
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let (status, _, _) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/users/{bob}"),
            Some(&admin),
            Some(json!({"display_name": "Bobby"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            &format!("/api/v1/users/{bob}/password"),
            Some(&admin),
            Some(json!({"new_password": CANARY})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/users/{bob}"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, alice_cookie) = sign_in(&app, "canary", "alice", CANARY).await;
    assert_eq!(status, StatusCode::OK);
    let alice_cookie = alice_cookie.unwrap_or_default();
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/auth/password",
            Some(&alice_cookie),
            Some(json!({"current_password": CANARY, "new_password": CANARY})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // api_token_create and api_token_revoke.
    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/api-tokens",
            Some(&admin),
            Some(json!({"name": "ci", "role": "viewer"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let token_value = created
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let token_id = created
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/api-tokens/{token_id}"),
            Some(&admin),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The setup token value must not appear either.
    let setup_token = token::random_token().unwrap_or_default();
    let setup_hash = token::hash_token(&setup_token);
    let mut tx = db
        .database()
        .begin_tenant(PLATFORM_TENANT_ID)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    setup::create(
        &mut tx,
        &setup_hash,
        OffsetDateTime::now_utc() + time::Duration::hours(1),
    )
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
    let body = json!({
        "setup_token": setup_token,
        "username": "root",
        "password": CANARY,
        "display_name": "Root",
    });
    let (status, _, _) = send(&app, request("POST", "/api/v1/setup", None, Some(body))).await;
    assert_eq!(status, StatusCode::CREATED);

    // The audit log holds a record for each operation.
    let actions = audit_actions(&db).await;
    for expected in [
        "sign_in",
        "sign_in_failed",
        "sign_out",
        "user_create",
        "user_update",
        "user_delete",
        "api_token_create",
        "api_token_revoke",
    ] {
        assert!(
            actions.get(expected).copied().unwrap_or(0) > 0,
            "the audit log holds no {expected} record: {actions:?}"
        );
    }

    // Dump holds none of the secrets.
    let dump = audit_dump(&db).await;
    assert!(
        !dump.contains(CANARY),
        "the audit log holds the canary password"
    );
    assert!(
        !dump.contains(&token_value),
        "the audit log holds the token value"
    );
    assert!(
        !dump.contains(&setup_token),
        "the audit log holds the setup token"
    );
    assert!(
        !dump.contains("alice"),
        "the audit log holds a known username"
    );
    assert!(
        !dump.contains("nobody"),
        "the audit log holds an attempted username"
    );

    // The account lives in the platform tenant; the platform Admin holds one membership there.
    assert_eq!(
        count(db.database(), PLATFORM_TENANT_ID, "memberships").await,
        1
    );

    db.cleanup().await;
}

/// Counts records for each action.
async fn audit_actions(
    db: &restichub_db::test_util::TestDatabase,
) -> std::collections::HashMap<String, i64> {
    let mut tx = db
        .owner_pool()
        .begin()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    sqlx::query("SELECT set_config('restichub.scope', 'platform', true)")
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT action, count(*) FROM audit_log GROUP BY action")
            .fetch_all(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
    rows.into_iter().collect()
}

async fn audit_dump(db: &restichub_db::test_util::TestDatabase) -> String {
    let mut tx = db
        .owner_pool()
        .begin()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    sqlx::query("SELECT set_config('restichub.scope', 'platform', true)")
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let (dump,): (String,) = sqlx::query_as(
        "SELECT coalesce(string_agg( \
             coalesce(action, '') || '|' || coalesce(object_type, '') || '|' || \
             coalesce(object_id::text, '') || '|' || coalesce(actor_type, '') || '|' || \
             coalesce(actor_id::text, '') || '|' || coalesce(source_ip::text, '') || '|' || \
             coalesce(diff::text, ''), E'\\n'), '') \
         FROM audit_log",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));
    dump
}
