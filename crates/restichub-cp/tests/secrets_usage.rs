//! The usage list of the secrets API (FR-RP-05).

mod common;

use axum::http::StatusCode;
use serde_json::Value;

use common::{
    app_with_kek, create_tenant_with_key, create_user, request, seed_repository, seed_secret, send,
    session_cookie,
};

async fn list(app: &axum::Router, cookie: &str) -> Vec<Value> {
    let (status, _, body) = send(app, request("GET", "/api/v1/secrets", Some(cookie), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["items"].as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn fr_rp_05_the_list_names_the_users_of_a_secret_and_delete_is_refused()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let tenant = create_tenant_with_key(db.database(), "secrets-usage").await;
    let admin = create_user(db.database(), tenant, "admin", "phc", "admin").await;
    let cookie = session_cookie(db.database(), tenant, admin).await;
    let app = app_with_kek(db.database());

    let used = seed_secret(db.database(), tenant, "used-password").await;
    let free = seed_secret(db.database(), tenant, "free-password").await;
    let repository = seed_repository(db.database(), tenant, "main-repo", used).await;

    let items = list(&app, &cookie).await;
    let find = |name: &str| {
        items
            .iter()
            .find(|item| item["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("the list holds no secret {name}"))
    };
    let used_by = find("used-password")["used_by"].clone();
    assert_eq!(used_by.as_array().map(Vec::len), Some(1), "{used_by}");
    assert_eq!(used_by[0]["kind"], "repository");
    assert_eq!(used_by[0]["name"], "main-repo");
    assert_eq!(used_by[0]["id"], repository.to_string());
    assert_eq!(find("free-password")["used_by"], serde_json::json!([]));

    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/secrets/{used}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/secrets/{free}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    Ok(())
}

#[tokio::test]
async fn fr_rp_05_the_usage_of_another_tenant_is_invisible()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let tenant_a = create_tenant_with_key(db.database(), "usage-a").await;
    let tenant_b = create_tenant_with_key(db.database(), "usage-b").await;
    let admin = create_user(db.database(), tenant_b, "admin", "phc", "admin").await;
    let cookie = session_cookie(db.database(), tenant_b, admin).await;
    let app = app_with_kek(db.database());

    let secret = seed_secret(db.database(), tenant_a, "other-password").await;
    seed_repository(db.database(), tenant_a, "other-repo", secret).await;
    assert!(list(&app, &cookie).await.is_empty());
    Ok(())
}
