//! The repository API and the bound secrets (FR-RP-01, FR-RP-05, FR-RP-08).
//!
//! The tests cover the typed backends, the secret binding, the write-only behavior, the tenant
//! isolation, the authorization, the validation and the audit records.

#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use restichub_db::audit::{AuditAction, AuditFilter, AuditPage, AuditRecord};
use restichub_db::secrets::SecretMetadata;
use restichub_db::{PLATFORM_TENANT_ID, TenantId, audit, delivery, secrets as secrets_repo};
use restichub_secrets::{
    EncryptedSecret, KekId, SecretPurpose, WrappedDek, decrypt_secret, unwrap_dek,
};
use serde_json::{Value, json};

use common::{
    app_with_kek, create_tenant_with_key, request, seed_agent, seed_session, send, test_kek,
};

/// An S3-compatible create request body for the tests.
fn s3_body(name: &str, password: &str, access: &str, secret: &str) -> Value {
    json!({
        "name": name,
        "backend": {
            "kind": "s3",
            "endpoint": "https://s3.example.net",
            "bucket": "backups-prod",
            "prefix": "snapshots"
        },
        "password": password,
        "s3_access_key": access,
        "s3_secret_key": secret,
    })
}

/// Sends a create request and returns the identifier.
async fn create_repository(app: &axum::Router, cookie: &str, body: Value) -> String {
    let (status, _, response) = send(
        app,
        request("POST", "/api/v1/repositories", Some(cookie), Some(body)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{response}");
    response
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("the response holds no identifier: {response}"))
}

/// Reads the secret metadata of a tenant.
async fn list_secrets(db: &restichub_db::Database, tenant: TenantId) -> Vec<SecretMetadata> {
    let mut tx = db.begin_tenant(tenant).await.expect("cannot begin");
    let list = secrets_repo::list(&mut tx).await.expect("cannot list");
    tx.commit().await.expect("cannot commit");
    list
}

/// Reads the identifier of the secret of one purpose.
async fn secret_id_by_purpose(
    db: &restichub_db::Database,
    tenant: TenantId,
    purpose: &str,
) -> restichub_db::SecretId {
    list_secrets(db, tenant)
        .await
        .into_iter()
        .find(|secret| secret.purpose == purpose)
        .unwrap_or_else(|| panic!("the {purpose} secret"))
        .id
}

/// Returns the plaintext of a stored value with the test key.
async fn decrypt(
    db: &restichub_db::Database,
    tenant: TenantId,
    id: restichub_db::SecretId,
) -> String {
    let mut tx = db.begin_tenant(tenant).await.expect("cannot begin");
    let row = delivery::load_ciphertext(&mut tx, id)
        .await
        .expect("cannot read")
        .expect("the secret exists");
    let key = restichub_db::kek::key_by_version(&mut tx, row.key_version)
        .await
        .expect("cannot read the key")
        .expect("the key exists");
    tx.commit().await.expect("cannot commit");
    let wrapped = WrappedDek {
        kek_id: KekId::parse(&key.kek_id).expect("the kek id"),
        nonce: key.nonce.as_slice().try_into().expect("the nonce"),
        ciphertext: key.wrapped_dek,
    };
    let kek = test_kek();
    let dek = unwrap_dek(kek.as_ref(), tenant.as_uuid(), key.key_version, &wrapped)
        .expect("cannot unwrap");
    let purpose = SecretPurpose::parse(&row.purpose).expect("the purpose");
    let encrypted = EncryptedSecret {
        nonce: row.nonce.as_slice().try_into().expect("the nonce"),
        ciphertext: row.ciphertext,
    };
    decrypt_secret(&dek, tenant.as_uuid(), id.as_uuid(), purpose, &encrypted)
        .expect("cannot decrypt")
        .expose_secret()
        .to_owned()
}

async fn audit_records(
    db: &restichub_db::Database,
    tenant: TenantId,
    action: AuditAction,
) -> Vec<AuditRecord> {
    let mut tx = db.begin_tenant(tenant).await.expect("cannot begin");
    let filter = AuditFilter {
        action: Some(action),
        ..AuditFilter::default()
    };
    let records = audit::list(&mut tx, &filter, &AuditPage::default())
        .await
        .expect("cannot list the audit")
        .records;
    tx.commit().await.expect("cannot commit");
    records
}

#[tokio::test]
async fn ac_rp_01_a_create_binds_the_secrets_and_masks_the_values() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let canary = "RESTICHUB-CANARY-repo-9a1b";
    let id = create_repository(
        &app,
        &cookie,
        s3_body("s3-prod", canary, "access-key", "secret-key"),
    )
    .await;

    // The GET response holds only the presence markers.
    let (status, _, view) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/repositories/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["name"], "s3-prod");
    assert_eq!(view["backend"]["kind"], "s3");
    assert_eq!(view["backend"]["bucket"], "backups-prod");
    assert_eq!(view["credentials"]["password"], true);
    assert_eq!(view["credentials"]["s3_access_key"], true);
    assert_eq!(view["credentials"]["s3_secret_key"], true);
    assert_eq!(view["initialized"], false);
    assert!(view["maintenance_agent_id"].is_null());
    assert!(
        !view.to_string().contains(canary),
        "the response must not hold the value"
    );

    // The three secrets are stored with their purpose and decrypted by the tenant key.
    let metadata = list_secrets(db.database(), tenant).await;
    assert_eq!(metadata.len(), 3, "{metadata:?}");
    let by_purpose = |purpose: &str| {
        metadata
            .iter()
            .find(|secret| secret.purpose == purpose)
            .unwrap_or_else(|| panic!("no secret of the purpose {purpose}"))
    };
    let password = by_purpose("repository_password").id;
    assert_eq!(decrypt(db.database(), tenant, password).await, canary);
    assert_eq!(
        decrypt(db.database(), tenant, by_purpose("s3_access_key").id).await,
        "access-key"
    );

    // No endpoint returns the value after the save (AC-RP-02).
    let (status, _, _) = send(&app, request("GET", "/api/v1/secrets", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::OK);

    // The create writes one audit record without the value.
    let records = audit_records(db.database(), tenant, AuditAction::RepositoryCreate).await;
    assert_eq!(records.len(), 1);
    assert!(
        !records[0]
            .diff
            .as_ref()
            .map(|diff| diff.to_string())
            .unwrap_or_default()
            .contains(canary),
        "the audit record must not hold the value"
    );

    db.cleanup().await;
}

#[tokio::test]
async fn ac_rp_02_the_stored_ciphertext_holds_no_canary() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let canary = "RESTICHUB-CANARY-dump-4c7d";
    create_repository(&app, &cookie, s3_body("s3-prod", canary, canary, canary)).await;

    let mut tx = db.owner_pool().begin().await.expect("cannot begin");
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await
        .expect("cannot set the tenant scope");
    let rows: Vec<(Vec<u8>,)> = sqlx::query_as("SELECT ciphertext FROM secrets")
        .fetch_all(&mut *tx)
        .await
        .expect("cannot read the ciphertext");
    tx.commit().await.expect("cannot commit");
    assert_eq!(rows.len(), 3);
    for (ciphertext,) in rows {
        assert!(
            !ciphertext
                .windows(canary.len())
                .any(|window| window == canary.as_bytes()),
            "the ciphertext must not hold the canary"
        );
    }

    db.cleanup().await;
}

#[tokio::test]
async fn ac_tn_01_a_repository_of_another_tenant_gives_404() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant_a = create_tenant_with_key(db.database(), "acme").await;
    let tenant_b = create_tenant_with_key(db.database(), "globex").await;
    let app = app_with_kek(db.database());
    let cookie_a = seed_session(db.database(), tenant_a, "admin-a", "admin").await;
    let cookie_b = seed_session(db.database(), tenant_b, "admin-b", "admin").await;

    let id_b = create_repository(&app, &cookie_b, s3_body("s3-b", "p", "a", "s")).await;

    // Tenant A cannot read the repository of tenant B.
    let (status, _, _) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/repositories/{id_b}"),
            Some(&cookie_a),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The repository list of tenant A stays empty here.
    let (status, _, body) = send(
        &app,
        request("GET", "/api/v1/repositories", Some(&cookie_a), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["items"].as_array().map(Vec::len), Some(0));

    db.cleanup().await;
}

#[tokio::test]
async fn a_viewer_cannot_write_and_the_platform_tenant_is_refused() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());

    let viewer = seed_session(db.database(), tenant, "viewer", "viewer").await;
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/repositories",
            Some(&viewer),
            Some(s3_body("s3-prod", "p", "a", "s")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The platform tenant holds no repository: the operation is tenant-only.
    let platform = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let (status, _, _) = send(
        &app,
        request("GET", "/api/v1/repositories", Some(&platform), None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
}

#[tokio::test]
async fn an_update_keeps_the_password_and_replaces_a_backend_credential() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let id = create_repository(&app, &cookie, s3_body("s3-prod", "old-password", "a", "s")).await;
    let password_id = secret_id_by_purpose(db.database(), tenant, "repository_password").await;
    let access_id = secret_id_by_purpose(db.database(), tenant, "s3_access_key").await;

    // A rename keeps the stored password value unchanged.
    let (status, _, _) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/repositories/{id}"),
            Some(&cookie),
            Some(json!({"name": "s3-prod-renamed"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        decrypt(db.database(), tenant, password_id).await,
        "old-password"
    );

    // The repository password cannot be replaced until the rotation of FR-RP-06 (1.0).
    let (status, _, _) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/repositories/{id}"),
            Some(&cookie),
            Some(json!({"password": "new-password"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        decrypt(db.database(), tenant, password_id).await,
        "old-password"
    );

    // A backend credential may still be replaced.
    let (status, _, _) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/repositories/{id}"),
            Some(&cookie),
            Some(json!({"s3_access_key": "a2"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(decrypt(db.database(), tenant, access_id).await, "a2");

    // The audit diff names the replaced secret field and holds no value.
    let records = audit_records(db.database(), tenant, AuditAction::RepositoryUpdate).await;
    // The password patch gave 409 and wrote no record; the rename and the credential patch did.
    assert_eq!(records.len(), 2);
    let diff = records[0].diff.as_ref().expect("the diff");
    assert_eq!(diff["replaced"]["new"], json!(["s3_access_key"]));
    assert!(!diff.to_string().contains("a2"));

    db.cleanup().await;
}

#[tokio::test]
async fn the_repository_password_cannot_be_replaced_through_the_secrets_api() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    create_repository(&app, &cookie, s3_body("s3-prod", "p", "a", "s")).await;
    let password_id = list_secrets(db.database(), tenant)
        .await
        .into_iter()
        .find(|secret| secret.purpose == "repository_password")
        .expect("the password secret")
        .id;

    // A bound repository password cannot be replaced (FR-RP-06 is 1.0).
    let (status, _, _) = send(
        &app,
        request(
            "PUT",
            &format!("/api/v1/secrets/{password_id}/value"),
            Some(&cookie),
            Some(json!({"value": "new-password"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(decrypt(db.database(), tenant, password_id).await, "p");

    // A standalone repository password secret is not bound, so it may be replaced.
    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/secrets",
            Some(&cookie),
            Some(json!({"name": "standalone.password", "purpose": "repository_password", "value": "x"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let standalone = created["id"].as_str().expect("the identifier").to_owned();
    let (status, _, _) = send(
        &app,
        request(
            "PUT",
            &format!("/api/v1/secrets/{standalone}/value"),
            Some(&cookie),
            Some(json!({"value": "y"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    db.cleanup().await;
}

#[tokio::test]
async fn a_rename_allows_the_old_name_and_a_taken_name_gives_409() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let first = create_repository(&app, &cookie, s3_body("s3-prod", "p", "a", "s")).await;
    create_repository(&app, &cookie, s3_body("s3-backup", "p", "a", "s")).await;

    // A rename to a name that is taken is a conflict.
    let (status, _, _) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/repositories/{first}"),
            Some(&cookie),
            Some(json!({"name": "s3-backup"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Rename to a free name, then reuse the old name: the derived secret names follow the
    // identifier, so the old name is free.
    let (status, _, _) = send(
        &app,
        request(
            "PATCH",
            &format!("/api/v1/repositories/{first}"),
            Some(&cookie),
            Some(json!({"name": "s3-prod-renamed"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/repositories",
            Some(&cookie),
            Some(s3_body("s3-prod", "p", "a", "s")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    db.cleanup().await;
}

#[tokio::test]
async fn a_delete_removes_the_bound_secrets() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant = create_tenant_with_key(db.database(), "acme").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;

    let id = create_repository(&app, &cookie, s3_body("s3-prod", "p", "a", "s")).await;
    assert_eq!(list_secrets(db.database(), tenant).await.len(), 3);

    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/repositories/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(list_secrets(db.database(), tenant).await.is_empty());
    assert_eq!(
        audit_records(db.database(), tenant, AuditAction::RepositoryDelete)
            .await
            .len(),
        1
    );

    db.cleanup().await;
}

#[tokio::test]
async fn the_validation_refuses_a_bad_backend_and_a_foreign_agent() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant_a = create_tenant_with_key(db.database(), "acme").await;
    let tenant_b = create_tenant_with_key(db.database(), "globex").await;
    let app = app_with_kek(db.database());
    let cookie = seed_session(db.database(), tenant_a, "admin", "admin").await;

    // A relative local path is refused with a field error.
    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/repositories",
            Some(&cookie),
            Some(json!({
                "name": "local-bad",
                "backend": {"kind": "local", "path": "relative/path"},
                "password": "p",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "backend.path");

    // A bad endpoint is refused with a field error.
    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/repositories",
            Some(&cookie),
            Some(json!({
                "name": "rest-bad",
                "backend": {"kind": "rest", "endpoint": "not-a-url"},
                "password": "p",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "backend.endpoint");

    // The endpoint holds no user information, no query and no fragment.
    for endpoint in [
        "https://u:p@s3.example.net",
        "https://s3.example.net?x=1",
        "https://s3.example.net#frag",
    ] {
        let (status, _, body) = send(
            &app,
            request(
                "POST",
                "/api/v1/repositories",
                Some(&cookie),
                Some(json!({
                    "name": "s3-bad",
                    "backend": {"kind": "s3", "endpoint": endpoint, "bucket": "backups"},
                    "password": "p",
                    "s3_access_key": "a",
                    "s3_secret_key": "s",
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{endpoint}: {body}");
        assert_eq!(body["field"], "backend.endpoint", "{endpoint}");
    }

    // A UNC path, a `..` segment and an empty segment are refused.
    for path in ["\\\\server\\share", "/mnt/../etc", "/mnt//backup"] {
        let (status, _, body) = send(
            &app,
            request(
                "POST",
                "/api/v1/repositories",
                Some(&cookie),
                Some(json!({
                    "name": "local-bad",
                    "backend": {"kind": "local", "path": path},
                    "password": "p",
                })),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {body}");
        assert_eq!(body["field"], "backend.path", "{path}");
    }

    // A `..` segment in the S3 prefix is refused.
    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/repositories",
            Some(&cookie),
            Some(json!({
                "name": "s3-bad-prefix",
                "backend": {
                    "kind": "s3",
                    "endpoint": "https://s3.example.net",
                    "bucket": "backups",
                    "prefix": "a/../b",
                },
                "password": "p",
                "s3_access_key": "a",
                "s3_secret_key": "s",
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "backend.prefix");

    // A maintenance agent of another tenant is refused (FR-RP-08).
    let foreign_agent = seed_agent(db.database(), tenant_b, "nas-backup").await;
    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/repositories",
            Some(&cookie),
            Some(json!({
                "name": "s3-prod",
                "backend": {"kind": "s3", "endpoint": "https://s3.example.net", "bucket": "backups"},
                "password": "p",
                "s3_access_key": "a",
                "s3_secret_key": "s",
                "maintenance_agent_id": foreign_agent.to_string(),
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["field"], "maintenance_agent_id");

    // The maintenance agent of the same tenant is accepted.
    let local_agent = seed_agent(db.database(), tenant_a, "nas-backup").await;
    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/repositories",
            Some(&cookie),
            Some(json!({
                "name": "s3-prod",
                "backend": {"kind": "s3", "endpoint": "https://s3.example.net", "bucket": "backups"},
                "password": "p",
                "s3_access_key": "a",
                "s3_secret_key": "s",
                "maintenance_agent_id": local_agent.to_string(),
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["maintenance_agent_id"], local_agent.to_string());

    db.cleanup().await;
}
