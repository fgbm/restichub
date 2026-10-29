//! AC-SC-12: the API token of a member whose membership an installation administrator removed
//! keeps working and stays revocable. The token belongs to the organization (threat T33).

#![allow(clippy::expect_used)]

mod common;
mod schedule_failure_support;

use axum::http::StatusCode;
use restichub_db::Role;
use restichub_db::api_tokens::{NewApiToken, create};
use serde_json::Value;

use common::{
    app_with_kek, bearer_request, create_tenant_with_key, find_account_id, seed_admin_token_actor,
    seed_api_token, seed_memberless_account, send,
};
use schedule_failure_support::owner_exec;

/// AC-SC-12: the token of a removed creator authenticates, writes its last use, and an Admin
/// revokes it. A token for a creator without a membership is still refused on insert.
#[tokio::test]
async fn ac_sc_12_the_token_of_a_removed_creator_keeps_working_and_stays_revocable() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let database = db.database();
    let tenant = create_tenant_with_key(database, "removed-creator-token").await;
    let value = seed_api_token(database, tenant, "operator").await;
    let admin = seed_admin_token_actor(database, tenant).await;
    let creator = find_account_id(database, &format!("creator-operator-{tenant}")).await;
    owner_exec(
        &db,
        tenant,
        "DELETE FROM memberships WHERE user_id = $1",
        &[creator.as_uuid()],
    )
    .await;
    let app = app_with_kek(database);

    let (status, _, me) = send(&app, bearer_request("GET", "/api/v1/auth/me", &value, None)).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me.get("role").and_then(Value::as_str), Some("operator"));

    let (status, _, page) = send(
        &app,
        bearer_request("GET", "/api/v1/api-tokens", &admin, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .expect("the list has items");
    let token = items
        .iter()
        .find(|item| item.get("role").and_then(Value::as_str) == Some("operator"))
        .expect("the operator token is listed");
    assert!(
        token
            .get("last_used_at")
            .is_some_and(|used| !used.is_null()),
        "the use of the token is recorded: {token}"
    );
    let id = token.get("id").and_then(Value::as_str).expect("token id");

    let (status, _, _) = send(
        &app,
        bearer_request("DELETE", &format!("/api/v1/api-tokens/{id}"), &admin, None),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = send(&app, bearer_request("GET", "/api/v1/auth/me", &value, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let outsider = seed_memberless_account(&db, "token-outsider", "phc").await;
    let mut tx = database.begin_tenant(tenant).await.expect("begin");
    let error = create(
        &mut tx,
        &NewApiToken {
            name: "outsider-token",
            token_hash: &[7_u8; 32],
            display_prefix: "rh_outsider",
            role: Role::Viewer,
            created_by: outsider,
            expires_at: None,
        },
    )
    .await
    .expect_err("a token for a creator without a membership is refused");
    assert_eq!(error.sqlstate().as_deref(), Some("23503"), "{error}");
    drop(tx);
    db.cleanup().await;
}
