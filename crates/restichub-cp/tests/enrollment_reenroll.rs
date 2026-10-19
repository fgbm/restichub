//! Re-registration, duplicate detection and the waiting state of a host (FR-ID-10, FR-ID-11,
//! FR-ID-04, FR-TN-01).

#![allow(clippy::expect_used)]

mod common;
mod reenroll_support;

use axum::http::StatusCode;
use serde_json::json;
use time::OffsetDateTime;

use common::{request, seed_plan, seed_profile, seed_repository, seed_secret, send};
use reenroll_support::{
    MACHINE, agent_id, agents, audit_count, enroll, enroll_new, env, find, issue_reenroll_token,
    serial_of,
};

#[tokio::test]
async fn ac_id_08_a_host_waits_for_its_first_connection() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "wait").await;
    let id = enroll_new(&db, &env, "waiting-host", None).await;

    let items = agents(&env).await;
    assert!(find(&items, id)["first_connected_at"].is_null());

    let (_, hash_serial) = serial_of(&db, &env, id).await;
    let mut tx = db
        .database()
        .begin_tenant(env.tenant)
        .await
        .expect("cannot begin");
    let touched = restichub_db::agents::touch_last_seen_guarded(
        &mut tx,
        id,
        &hash_serial,
        OffsetDateTime::now_utc(),
    )
    .await
    .expect("cannot touch");
    tx.commit().await.expect("cannot commit");
    assert!(touched);

    let items = agents(&env).await;
    assert!(find(&items, id)["first_connected_at"].is_string());
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_07_a_used_token_names_the_host_it_enrolled() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "named").await;
    let id = enroll_new(&db, &env, "named-host", None).await;

    let (status, _, page) = send(
        &env.api,
        request("GET", "/api/v1/enrollment-tokens", Some(&env.cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let tokens = page["items"].as_array().expect("a list");
    let used: Vec<_> = tokens
        .iter()
        .filter(|token| token["used_at"].is_string())
        .collect();
    assert_eq!(used.len(), 1);
    assert_eq!(used[0]["used_by_agent_id"], id.to_string());
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_07_a_bound_token_keeps_the_agent_and_replaces_the_identity() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "reenroll").await;
    let id = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let secret = seed_secret(db.database(), env.tenant, "plan.password").await;
    let repository = seed_repository(db.database(), env.tenant, "repo", secret).await;
    let profile = seed_profile(db.database(), env.tenant, "profile").await;
    let plan = seed_plan(db.database(), env.tenant, "plan", profile, id, repository).await;
    let (old_serial, old_hex) = serial_of(&db, &env, id).await;

    let (status, issued) = issue_reenroll_token(&env, id).await;
    assert_eq!(status, StatusCode::CREATED, "{issued}");
    assert_eq!(issued["replaces_agent_id"], id.to_string());
    let token = issued["token"].as_str().expect("the token");

    let (status, response) = enroll(&env, token, "MINDCORE", Some(MACHINE)).await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    assert_eq!(response["reenrolled"], true);
    assert_eq!(agent_id(&response), id);

    let items = agents(&env).await;
    assert_eq!(items.len(), 1, "no second record appears");
    let row = find(&items, id);
    assert_eq!(row["status"], "active");
    assert!(row["first_connected_at"].is_null());
    assert!(row["possible_duplicate_of"].is_null());
    assert_eq!(audit_count(&env, "agent_reenroll").await, 1);

    let (new_serial, new_hex) = serial_of(&db, &env, id).await;
    assert_ne!(old_serial, new_serial);
    let mut tx = db
        .database()
        .begin_tenant(env.tenant)
        .await
        .expect("cannot begin");
    let now = OffsetDateTime::now_utc();
    let old = restichub_db::agents::touch_last_seen_guarded(&mut tx, id, &old_hex, now)
        .await
        .expect("cannot touch");
    let new = restichub_db::agents::touch_last_seen_guarded(&mut tx, id, &new_hex, now)
        .await
        .expect("cannot touch");
    tx.commit().await.expect("cannot commit");
    assert!(!old, "the old certificate is refused at once");
    assert!(new);

    let (status, _, plan_view) = send(
        &env.api,
        request(
            "GET",
            &format!("/api/v1/plans/{plan}"),
            Some(&env.cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(plan_view["agent_id"], id.to_string());

    let (status, _) = enroll(&env, token, "MINDCORE", Some(MACHINE)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_07_a_bound_token_of_a_revoked_host_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "reenroll-revoked").await;
    let id = enroll_new(&db, &env, "gone", None).await;
    let (status, issued) = issue_reenroll_token(&env, id).await;
    assert_eq!(status, StatusCode::CREATED);
    let token = issued["token"].as_str().expect("the token").to_owned();

    let (status, _, _) = send(
        &env.api,
        request(
            "POST",
            &format!("/api/v1/agents/{id}/revoke"),
            Some(&env.cookie),
            Some(json!({})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = enroll(&env, &token, "gone", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = issue_reenroll_token(&env, id).await;
    assert_eq!(status, StatusCode::CONFLICT);
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_09_an_unbound_token_of_the_same_machine_flags_both_records() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "duplicate").await;
    let old = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let new = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let other = enroll_new(&db, &env, "OTHER", Some("ffffffffffffffffffffffffffffffff")).await;
    let bare = enroll_new(&db, &env, "BARE", None).await;
    assert_ne!(old, new);

    let items = agents(&env).await;
    assert_eq!(
        find(&items, new)["possible_duplicate_of"]["id"],
        old.to_string()
    );
    assert_eq!(
        find(&items, new)["possible_duplicate_of"]["hostname"],
        "MINDCORE"
    );
    assert_eq!(
        find(&items, old)["possible_duplicate_of"]["id"],
        new.to_string()
    );
    assert!(find(&items, other)["possible_duplicate_of"].is_null());
    assert!(find(&items, bare)["possible_duplicate_of"].is_null());
    db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_01_the_same_machine_in_two_tenants_never_matches() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let first = env(&db, "tenant-one").await;
    let second = env(&db, "tenant-two").await;
    let in_first = enroll_new(&db, &first, "MINDCORE", Some(MACHINE)).await;
    let in_second = enroll_new(&db, &second, "MINDCORE", Some(MACHINE)).await;

    assert!(find(&agents(&first).await, in_first)["possible_duplicate_of"].is_null());
    assert!(find(&agents(&second).await, in_second)["possible_duplicate_of"].is_null());

    // A replace across the tenants finds no pair.
    let (status, _, _) = send(
        &second.api,
        request(
            "POST",
            &format!("/api/v1/agents/{in_second}/replace"),
            Some(&second.cookie),
            Some(json!({"old_agent_id": in_first.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        find(&agents(&first).await, in_first)["status"],
        "active",
        "the other tenant is untouched"
    );
    db.cleanup().await;
}

#[tokio::test]
async fn ac_id_09_replace_moves_plans_and_maintenance_references_and_revokes_the_old_record() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "replace").await;
    let old = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let new = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let unrelated = enroll_new(&db, &env, "OTHER", Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")).await;

    let secret = seed_secret(db.database(), env.tenant, "plan.password").await;
    let repository = seed_repository(db.database(), env.tenant, "repo", secret).await;
    let profile = seed_profile(db.database(), env.tenant, "profile").await;
    let plan = seed_plan(db.database(), env.tenant, "plan", profile, old, repository).await;
    let maintenance_repository = {
        use restichub_db::repositories::{NewRepository, RepositorySecrets, RepositorySettings};
        let id = restichub_db::RepositoryId::new();
        let mut tx = db
            .database()
            .begin_tenant(env.tenant)
            .await
            .expect("cannot begin");
        restichub_db::repositories::create(
            &mut tx,
            &NewRepository {
                id,
                name: "maintained",
                backend: restichub_db::BackendKind::Local,
                settings: RepositorySettings {
                    local_path: Some("/mnt/backup/maintained"),
                    ..RepositorySettings::default()
                },
                secrets: RepositorySecrets {
                    password: secret,
                    s3_access_key: None,
                    s3_secret_key: None,
                    rest_password: None,
                },
                maintenance_agent_id: Some(old),
                created_by: None,
            },
        )
        .await
        .expect("cannot create the repository");
        // AC-RP-08: the host of an initialized local repository moves with the replace.
        restichub_db::repositories::set_initialized(&mut tx, id, true)
            .await
            .expect("cannot mark the repository initialized");
        tx.commit().await.expect("cannot commit");
        id
    };

    // A pair without a shared machine identifier is refused.
    let (status, _, _) = send(
        &env.api,
        request(
            "POST",
            &format!("/api/v1/agents/{new}/replace"),
            Some(&env.cookie),
            Some(json!({"old_agent_id": unrelated.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, moved) = send(
        &env.api,
        request(
            "POST",
            &format!("/api/v1/agents/{new}/replace"),
            Some(&env.cookie),
            Some(json!({"old_agent_id": old.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["plans_moved"], 1);
    assert_eq!(moved["repositories_moved"], 1);

    let items = agents(&env).await;
    assert_eq!(find(&items, old)["status"], "revoked");
    assert_eq!(find(&items, new)["status"], "active");
    assert!(find(&items, new)["possible_duplicate_of"].is_null());
    assert_eq!(audit_count(&env, "agent_replace").await, 1);

    let (_, _, plan_view) = send(
        &env.api,
        request(
            "GET",
            &format!("/api/v1/plans/{plan}"),
            Some(&env.cookie),
            None,
        ),
    )
    .await;
    assert_eq!(plan_view["agent_id"], new.to_string());
    let (_, _, repository_view) = send(
        &env.api,
        request(
            "GET",
            &format!("/api/v1/repositories/{maintenance_repository}"),
            Some(&env.cookie),
            None,
        ),
    )
    .await;
    assert_eq!(
        repository_view["maintenance_agent_id"],
        new.to_string(),
        "{repository_view}"
    );
    db.cleanup().await;
}

/// FR-ID-11: a replace moves the parameter values of the host with its plans (FR-PF-10).
#[tokio::test]
async fn fr_id_11_a_replace_moves_the_parameter_values_of_the_host() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "params").await;
    let old = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let new = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;

    let (status, _, _) = send(
        &env.api,
        request(
            "PUT",
            &format!("/api/v1/agents/{old}/parameters"),
            Some(&env.cookie),
            Some(json!({
                "parameters": [{"name": "pg_dir", "kind": "text", "value": "/srv/pg"}],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, moved) = send(
        &env.api,
        request(
            "POST",
            &format!("/api/v1/agents/{new}/replace"),
            Some(&env.cookie),
            Some(json!({"old_agent_id": old.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");

    let (status, _, table) = send(
        &env.api,
        request(
            "GET",
            &format!("/api/v1/agents/{new}/parameters"),
            Some(&env.cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{table}");
    let moved_row = table["parameters"]
        .as_array()
        .expect("the table")
        .iter()
        .find(|row| row["name"] == "pg_dir")
        .expect("the moved value");
    assert_eq!(moved_row["value"]["value"], "/srv/pg");
    assert_eq!(moved_row["unused"], true);

    let (status, _, old_table) = send(
        &env.api,
        request(
            "GET",
            &format!("/api/v1/agents/{old}/parameters"),
            Some(&env.cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{old_table}");
    assert_eq!(
        old_table["parameters"].as_array().expect("the table").len(),
        0,
        "{old_table}"
    );

    db.cleanup().await;
}

/// FR-ID-11: a value of the successor with another kind keeps its row, so a plan that moves loses a
/// required value and the replace is refused (FR-PF-11, ADR-0047).
#[tokio::test]
async fn fr_id_11_a_replace_refuses_a_kind_conflict_that_leaves_a_plan_empty() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let env = env(&db, "replace-kind").await;
    let old = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let new = enroll_new(&db, &env, "MINDCORE", Some(MACHINE)).await;
    let secret = seed_secret(db.database(), env.tenant, "plan.password").await;
    let repository = seed_repository(db.database(), env.tenant, "repo", secret).await;
    let profile = seed_profile(db.database(), env.tenant, "profile").await;
    let plan = seed_plan(db.database(), env.tenant, "plan", profile, old, repository).await;
    let (status, _, hook_secret) = send(
        &env.api,
        request(
            "POST",
            "/api/v1/secrets",
            Some(&env.cookie),
            Some(
                json!({"name": "pg-password", "purpose": "hook_secret", "value": "CANARY-hook-1"}),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{hook_secret}");
    let hook_secret = hook_secret["id"]
        .as_str()
        .expect("the identifier")
        .to_owned();
    let (status, _, _) = send(
        &env.api,
        request(
            "PUT",
            &format!("/api/v1/agents/{old}/parameters"),
            Some(&env.cookie),
            Some(json!({
                "parameters": [
                    {"name": "pg_password", "kind": "secret", "secret_id": hook_secret},
                ],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // The profile now requires a secret value that the old record already holds.
    let (status, _, body) = send(
        &env.api,
        request(
            "PATCH",
            &format!("/api/v1/profiles/{profile}"),
            Some(&env.cookie),
            Some(json!({
                "paths": {"all": ["/srv"]},
                "parameters": [{"name": "pg_password", "kind": "secret", "required": true}],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The new record holds a value of the other kind, so the moved value cannot replace it.
    let (status, _, _) = send(
        &env.api,
        request(
            "PUT",
            &format!("/api/v1/agents/{new}/parameters"),
            Some(&env.cookie),
            Some(json!({
                "parameters": [{"name": "pg_password", "kind": "text", "value": "/srv/pg"}],
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _, body) = send(
        &env.api,
        request(
            "POST",
            &format!("/api/v1/agents/{new}/replace"),
            Some(&env.cookie),
            Some(json!({"old_agent_id": old.to_string()})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["reason"], "parameter_missing_in_plans");
    assert_eq!(body["parameters"], json!(["pg_password"]));

    // Nothing moved: the plan still runs on the old record.
    let (status, _, view) = send(
        &env.api,
        request(
            "GET",
            &format!("/api/v1/plans/{plan}"),
            Some(&env.cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["agent_id"], old.to_string());

    db.cleanup().await;
}
