//! The write-only secret API and the encrypted storage (FR-RP-03, FR-RP-05).
//!
//! The tests cover the metadata API, the encryption round trip, the isolation between the tenants,
//! the authorization, the audit records and the canary checks of AC-RP-02 and AC-RP-03.

mod common;

use std::io;
use std::sync::{Arc, Mutex, Once, OnceLock};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use restichub_cp::delivery::SecretReader;
use restichub_cp::server::SECRET_BODY_LIMIT;
use restichub_db::audit::{AuditAction, AuditFilter, AuditPage, AuditRecord};
use restichub_db::test_util::TestDatabase;
use restichub_db::{Database, PLATFORM_TENANT_ID, SecretId, TenantId, audit, delivery, kek};
use restichub_secrets::{
    EncryptedSecret, KekId, SecretPurpose, WrappedDek, decrypt_secret, generate_dek, unwrap_dek,
    wrap_dek,
};
use serde_json::{Value, json};
use uuid::Uuid;

use common::{
    app_with_kek, create_tenant, create_tenant_with_key, create_user, raw_request, request,
    seed_session, send, send_response, session_cookie, test_kek,
};

/// The canary value. It must never appear in a response, a log or a database dump.
const CANARY: &str = "RESTICHUB-CANARY-7f3a91c2d4e5";
const PURPOSE: &str = "repository_password";

/// Builds a create-secret body.
fn secret_body(name: &str, value: &str) -> Value {
    json!({"name": name, "purpose": PURPOSE, "value": value})
}

/// Creates a secret through the API and returns its identifier.
async fn create_secret(app: &Router, cookie: &str, name: &str, value: &str) -> String {
    let (status, _, body) = send(
        app,
        request(
            "POST",
            "/api/v1/secrets",
            Some(cookie),
            Some(secret_body(name, value)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body.get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("the response holds no identifier: {body}"))
}

/// Sends a request and returns the status, the content type and the JSON body.
async fn send_problem(app: &Router, request: Request<Body>) -> (StatusCode, Option<String>, Value) {
    let response = send_response(app, request).await;
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = common::body_json(response).await;
    (status, content_type, body)
}

/// Reads the nonce and the ciphertext of a stored secret.
async fn stored_value(
    db: &Database,
    tenant: TenantId,
    id: SecretId,
) -> Result<(Vec<u8>, Vec<u8>), Box<dyn std::error::Error>> {
    let mut tx = db.begin_tenant(tenant).await?;
    let row = delivery::load_ciphertext(&mut tx, id)
        .await?
        .ok_or("the secret is missing")?;
    tx.commit().await?;
    Ok((row.nonce, row.ciphertext))
}

/// Decrypts a stored secret through `delivery::load_ciphertext` and the tenant key.
async fn decrypt_stored(
    db: &Database,
    tenant: TenantId,
    id: SecretId,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut tx = db.begin_tenant(tenant).await?;
    let row = delivery::load_ciphertext(&mut tx, id)
        .await?
        .ok_or("the secret is missing")?;
    let key = kek::key_by_version(&mut tx, row.key_version)
        .await?
        .ok_or("the key is missing")?;
    tx.commit().await?;
    let wrapped = WrappedDek {
        kek_id: KekId::parse(&key.kek_id)?,
        nonce: key.nonce.as_slice().try_into()?,
        ciphertext: key.wrapped_dek,
    };
    let kek = test_kek();
    let dek = unwrap_dek(kek.as_ref(), tenant.as_uuid(), key.key_version, &wrapped)?;
    let purpose = SecretPurpose::parse(&row.purpose).ok_or("the purpose is not known")?;
    let encrypted = EncryptedSecret {
        nonce: row.nonce.as_slice().try_into()?,
        ciphertext: row.ciphertext,
    };
    let plaintext = decrypt_secret(&dek, tenant.as_uuid(), id.as_uuid(), purpose, &encrypted)?;
    Ok(plaintext.expose_secret().to_owned())
}

/// Returns the audit records of one action in a tenant.
async fn audit_records(
    db: &Database,
    tenant: TenantId,
    action: AuditAction,
) -> Result<Vec<AuditRecord>, Box<dyn std::error::Error>> {
    let mut tx = db.begin_tenant(tenant).await?;
    let filter = AuditFilter {
        action: Some(action),
        ..AuditFilter::default()
    };
    let records = audit::list(&mut tx, &filter, &AuditPage::default())
        .await?
        .records;
    tx.commit().await?;
    Ok(records)
}

/// Builds the hexadecimal form of a byte slice.
fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

/// Adds a key version of a tenant and makes it the active one.
///
/// The function uses the owner pool, because the runtime repository has no function to rotate a key
/// in the MVP. It deactivates the current key before it inserts the new one, because the table
/// allows one active key for each tenant.
async fn add_key_version(
    db: &TestDatabase,
    tenant: TenantId,
    version: i32,
) -> Result<(), Box<dyn std::error::Error>> {
    let kek = test_kek();
    let dek = generate_dek()?;
    let wrapped = wrap_dek(kek.current(), tenant.as_uuid(), version, &dek)?;
    let mut tx = db.owner_pool().begin().await?;
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE tenant_keys SET active = false WHERE tenant_id = $1 AND active")
        .bind(tenant.as_uuid())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO tenant_keys (tenant_id, key_version, kek_id, wrapped_dek, nonce, active) \
         VALUES ($1, $2, $3, $4, $5, true)",
    )
    .bind(tenant.as_uuid())
    .bind(version)
    .bind(wrapped.kek_id.as_str())
    .bind(&wrapped.ciphertext)
    .bind(wrapped.nonce.as_slice())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Dumps every row of every public table as text through the owner pool.
///
/// The function sets the tenant scope and the platform scope, so Row-Level Security shows the rows
/// of the tenant and the platform rows. The owner login is not the runtime login, but
/// `FORCE ROW LEVEL SECURITY` still applies to it (ADR-0023).
async fn dump_all(
    db: &TestDatabase,
    tenant: TenantId,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut tx = db.owner_pool().begin().await?;
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant.to_string())
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT set_config('restichub.scope', 'platform', true)")
        .execute(&mut *tx)
        .await?;
    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_type = 'BASE TABLE' ORDER BY table_name",
    )
    .fetch_all(&mut *tx)
    .await?;
    let mut dump = String::new();
    for (table,) in tables {
        let query = format!(
            "SELECT coalesce(string_agg(row_to_json(t)::text, E'\\n'), '') FROM \"{table}\" t"
        );
        let (text,): (String,) = sqlx::query_as(sqlx::AssertSqlSafe(query))
            .fetch_one(&mut *tx)
            .await?;
        dump.push_str(&table);
        dump.push('|');
        dump.push_str(&text);
        dump.push('\n');
    }
    tx.commit().await?;
    Ok(dump)
}

/// The buffer that captures the tracing output of the test process.
static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();

fn logs() -> &'static Arc<Mutex<Vec<u8>>> {
    LOGS.get_or_init(|| Arc::new(Mutex::new(Vec::new())))
}

/// A writer that appends the tracing output to the shared buffer.
struct BufferWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for BufferWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut guard = self
            .0
            .lock()
            .map_err(|_| io::Error::other("the log buffer is poisoned"))?;
        guard.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufferWriter {
    type Writer = BufferWriter;

    fn make_writer(&'a self) -> Self::Writer {
        BufferWriter(self.0.clone())
    }
}

/// Installs a tracing subscriber that writes to the shared buffer at TRACE level.
fn install_log_capture() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(BufferWriter(logs().clone()))
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    });
}

fn captured_logs() -> String {
    let guard = logs().lock().unwrap_or_else(|error| error.into_inner());
    String::from_utf8_lossy(&guard).into_owned()
}

#[tokio::test]
async fn ac_rp_02_no_endpoint_returns_a_secret_value() -> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let tenant = create_tenant_with_key(db.database(), "secrets-read").await;
    let admin = create_user(db.database(), tenant, "admin", "phc", "admin").await;
    let cookie = session_cookie(db.database(), tenant, admin).await;
    let app = app_with_kek(db.database());

    // The create response holds the metadata only, and it names the creator.
    let (status, _, created) = send(
        &app,
        request(
            "POST",
            "/api/v1/secrets",
            Some(&cookie),
            Some(secret_body("repo-password", CANARY)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created
        .get("id")
        .and_then(Value::as_str)
        .ok_or("the response holds no identifier")?
        .to_owned();
    assert_eq!(
        created.get("created_by").and_then(Value::as_str),
        Some(admin.to_string().as_str())
    );
    assert!(!created.to_string().contains(CANARY));

    // Every GET route of the OpenAPI document is called as an Admin.
    let (status, _, document) =
        send(&app, request("GET", "/api/v1/openapi.json", None, None)).await;
    assert_eq!(status, StatusCode::OK);
    let paths = document
        .get("paths")
        .and_then(Value::as_object)
        .ok_or("the document holds no paths")?;
    let mut checked = 0;
    for (path, item) in paths {
        let Some(methods) = item.as_object() else {
            continue;
        };
        if !methods.contains_key("get") {
            continue;
        }
        let uri = path.replace("{id}", &id);
        let (_, _, body) = send(&app, request("GET", &uri, Some(&cookie), None)).await;
        let text = body.to_string();
        assert!(
            !text.contains(CANARY),
            "GET {uri} returned the secret value: {text}"
        );
        checked += 1;
    }
    assert!(checked > 0, "the document holds no GET route");

    // The single read shows the metadata and the marker `set`.
    let (status, _, one) = send(
        &app,
        request("GET", &format!("/api/v1/secrets/{id}"), Some(&cookie), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(one.get("value").and_then(Value::as_str), Some("set"));
    assert_eq!(
        one.get("name").and_then(Value::as_str),
        Some("repo-password")
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_02_a_database_dump_does_not_contain_the_canary()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let tenant = create_tenant_with_key(db.database(), "secrets-dump").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app_with_kek(db.database());
    let _id = create_secret(&app, &cookie, "dump", CANARY).await;

    let dump = dump_all(&db, tenant).await?;
    assert!(!dump.contains(CANARY), "the dump holds the canary");
    assert!(
        !dump.contains(&hex(CANARY.as_bytes())),
        "the dump holds the canary in hexadecimal"
    );
    let encoded = base64::engine::general_purpose::STANDARD.encode(CANARY.as_bytes());
    assert!(
        !dump.contains(&encoded),
        "the dump holds the canary in base64"
    );
    // The dump reaches the ciphertext of the secret and the audit record, so the check is not
    // vacuous. The platform scope reads every audit row (migration 0006).
    assert!(
        dump.contains("ciphertext"),
        "the dump must reach the secrets table"
    );
    assert!(
        dump.contains("secret_create"),
        "the dump must reach the audit record"
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_02_the_value_round_trips_through_the_delivery_loader()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let tenant = create_tenant_with_key(db.database(), "secrets-round").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app_with_kek(db.database());
    let id = create_secret(&app, &cookie, "round", CANARY).await;
    let secret_id = id.parse::<SecretId>()?;

    let plaintext = decrypt_stored(db.database(), tenant, secret_id).await?;
    assert_eq!(plaintext, CANARY);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_02_replace_changes_the_nonce_and_the_ciphertext()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let tenant = create_tenant_with_key(db.database(), "secrets-replace").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app_with_kek(db.database());
    let id = create_secret(&app, &cookie, "rotate", CANARY).await;
    let secret_id = id.parse::<SecretId>()?;

    let before = stored_value(db.database(), tenant, secret_id).await?;
    let (status, _, _) = send(
        &app,
        request(
            "PUT",
            &format!("/api/v1/secrets/{id}/value"),
            Some(&cookie),
            Some(json!({"value": "the new value"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let after = stored_value(db.database(), tenant, secret_id).await?;
    assert_ne!(before.0, after.0, "the nonce must change");
    assert_ne!(before.1, after.1, "the ciphertext must change");
    assert_eq!(
        decrypt_stored(db.database(), tenant, secret_id).await?,
        "the new value"
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_02_a_secret_of_another_tenant_is_not_found() -> Result<(), Box<dyn std::error::Error>>
{
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let tenant_a = create_tenant_with_key(db.database(), "secrets-a").await;
    let tenant_b = create_tenant_with_key(db.database(), "secrets-b").await;
    let cookie_a = seed_session(db.database(), tenant_a, "admin-a", "admin").await;
    let cookie_b = seed_session(db.database(), tenant_b, "admin-b", "admin").await;
    let app = app_with_kek(db.database());
    let id = create_secret(&app, &cookie_a, "private", CANARY).await;

    // The Admin of tenant B cannot see, replace or remove the secret of tenant A.
    let (status, _, _) = send(
        &app,
        request(
            "GET",
            &format!("/api/v1/secrets/{id}"),
            Some(&cookie_b),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send(
        &app,
        request(
            "PUT",
            &format!("/api/v1/secrets/{id}/value"),
            Some(&cookie_b),
            Some(json!({"value": "stolen"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/secrets/{id}"),
            Some(&cookie_b),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The list of tenant B is empty and the secret of tenant A is unchanged.
    let (_, _, list) = send(
        &app,
        request("GET", "/api/v1/secrets", Some(&cookie_b), None),
    )
    .await;
    assert_eq!(
        list.get("items").and_then(Value::as_array).map(Vec::len),
        Some(0)
    );
    assert_eq!(
        decrypt_stored(db.database(), tenant_a, id.parse()?).await?,
        CANARY
    );
    // A replace and a delete of a missing secret write no audit record.
    assert!(
        audit_records(db.database(), tenant_b, AuditAction::SecretReplace)
            .await?
            .is_empty()
    );
    assert!(
        audit_records(db.database(), tenant_b, AuditAction::SecretDelete)
            .await?
            .is_empty()
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_02_a_viewer_and_an_operator_cannot_write_a_secret()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let tenant = create_tenant_with_key(db.database(), "secrets-roles").await;
    let admin = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app_with_kek(db.database());
    let id = create_secret(&app, &admin, "guarded", CANARY).await;

    for role in ["viewer", "operator"] {
        let cookie = seed_session(db.database(), tenant, &format!("user-{role}"), role).await;
        // A Viewer reads the metadata, an Operator reads it too. Both cannot write.
        let (status, _, _) =
            send(&app, request("GET", "/api/v1/secrets", Some(&cookie), None)).await;
        assert_eq!(status, StatusCode::OK, "{role} must read the metadata");
        let (status, _, _) = send(
            &app,
            request(
                "POST",
                "/api/v1/secrets",
                Some(&cookie),
                Some(secret_body("blocked", "value")),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{role} must not create");
        let (status, _, _) = send(
            &app,
            request(
                "PUT",
                &format!("/api/v1/secrets/{id}/value"),
                Some(&cookie),
                Some(json!({"value": "value"})),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{role} must not replace");
        let (status, _, _) = send(
            &app,
            request(
                "DELETE",
                &format!("/api/v1/secrets/{id}"),
                Some(&cookie),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{role} must not delete");
    }

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_02_the_platform_tenant_refuses_a_secret_operation()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let cookie = seed_session(db.database(), PLATFORM_TENANT_ID, "root", "platform_admin").await;
    let app = app_with_kek(db.database());

    let (status, _, _) = send(&app, request("GET", "/api/v1/secrets", Some(&cookie), None)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = send(
        &app,
        request(
            "POST",
            "/api/v1/secrets",
            Some(&cookie),
            Some(secret_body("platform", "value")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_02_a_tenant_without_a_key_row_gives_tenant_key_missing()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    // The tenant holds no key row: the helper `create_tenant` writes the tenant only.
    let tenant = create_tenant(db.database(), "secrets-no-key").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app_with_kek(db.database());

    let (status, _, body) = send(
        &app,
        request(
            "POST",
            "/api/v1/secrets",
            Some(&cookie),
            Some(secret_body("no-key", "value")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("tenant_key_missing")
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_02_invalid_bodies_are_problem_responses_without_the_canary()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let tenant = create_tenant_with_key(db.database(), "secrets-invalid").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app_with_kek(db.database());

    let (status, content_type, body) = send_problem(
        &app,
        request(
            "POST",
            "/api/v1/secrets",
            Some(&cookie),
            Some(json!({"name": "num", "purpose": PURPOSE, "value": 12345678})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(content_type.as_deref(), Some("application/problem+json"));
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("invalid_argument")
    );
    assert!(!body.to_string().contains("12345678"), "{body}");

    let (status, content_type, body) = send_problem(
        &app,
        request(
            "POST",
            "/api/v1/secrets",
            Some(&cookie),
            Some(json!({"name": "bool", "purpose": PURPOSE, "value": true})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(content_type.as_deref(), Some("application/problem+json"));
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("invalid_argument")
    );

    // The body is malformed and it holds the canary.
    let malformed =
        format!("{{\"name\":\"broken\",\"purpose\":\"{PURPOSE}\",\"value\":\"{CANARY}\"");
    let (status, content_type, body) = send_problem(
        &app,
        raw_request("POST", "/api/v1/secrets", &cookie, malformed),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(content_type.as_deref(), Some("application/problem+json"));
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("invalid_argument")
    );
    assert!(!body.to_string().contains(CANARY), "{body}");

    // The body is larger than the limit of the route.
    let big = "x".repeat(SECRET_BODY_LIMIT);
    let (status, content_type, body) = send_problem(
        &app,
        request(
            "POST",
            "/api/v1/secrets",
            Some(&cookie),
            Some(json!({"name": "big", "purpose": PURPOSE, "value": big})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(content_type.as_deref(), Some("application/problem+json"));
    assert_eq!(
        body.get("code").and_then(Value::as_str),
        Some("payload_too_large")
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_02_the_logs_hold_no_canary() -> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    install_log_capture();
    let tenant = create_tenant_with_key(db.database(), "secrets-logs").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app_with_kek(db.database());

    // A create with the canary value, and an invalid body that holds the canary.
    let _ = create_secret(&app, &cookie, "logged", CANARY).await;
    let malformed =
        format!("{{\"name\":\"broken\",\"purpose\":\"{PURPOSE}\",\"value\":\"{CANARY}\"");
    let (status, _, _) = send_problem(
        &app,
        raw_request("POST", "/api/v1/secrets", &cookie, malformed),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let logs = captured_logs();
    assert!(!logs.is_empty(), "the capture must hold some output");
    assert!(
        !logs.contains(CANARY),
        "the tracing output holds the canary: {logs}"
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_02_the_audit_records_hold_no_canary() -> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let tenant = create_tenant_with_key(db.database(), "secrets-audit").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app_with_kek(db.database());
    let id = create_secret(&app, &cookie, "audited", CANARY).await;
    let (status, _, _) = send(
        &app,
        request(
            "PUT",
            &format!("/api/v1/secrets/{id}/value"),
            Some(&cookie),
            Some(json!({"value": CANARY})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = send(
        &app,
        request(
            "DELETE",
            &format!("/api/v1/secrets/{id}"),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    for action in [
        AuditAction::SecretCreate,
        AuditAction::SecretReplace,
        AuditAction::SecretDelete,
    ] {
        let records = audit_records(db.database(), tenant, action.clone()).await?;
        assert_eq!(records.len(), 1, "the {action:?} record is missing");
        let diff = records
            .first()
            .and_then(|record| record.diff.as_ref())
            .map(|diff| diff.to_string())
            .unwrap_or_default();
        assert!(!diff.contains(CANARY), "the diff holds the canary: {diff}");
        assert!(
            diff.contains("<redacted>"),
            "the diff is not redacted: {diff}"
        );
    }

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_03_secret_delivery_is_audited() -> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let tenant = create_tenant_with_key(db.database(), "secrets-delivery").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app_with_kek(db.database());
    let id = create_secret(&app, &cookie, "delivery", CANARY).await;
    let secret_id = id.parse::<SecretId>()?;

    let agent_id = Uuid::now_v7();
    let job_id = Uuid::now_v7();
    let repository_id = Uuid::now_v7();

    // The reader returns the plaintext and writes the audit record in the same transaction.
    let kek = test_kek();
    let reader = SecretReader::new(kek.as_ref());
    let mut tx = db.database().begin_tenant(tenant).await?;
    let plaintext = reader
        .read_for_delivery(&mut tx, secret_id, agent_id, job_id, repository_id)
        .await?;
    assert_eq!(plaintext.expose_secret(), CANARY);
    tx.commit().await?;

    // The audit log holds the record with the tenant, the agent, the job and the repository.
    let records = audit_records(db.database(), tenant, AuditAction::SecretDelivered).await?;
    assert_eq!(records.len(), 1, "the secret_delivered record is missing");
    let record = records.first().ok_or("the record is missing")?;
    assert_eq!(record.tenant_id, tenant);
    assert_eq!(record.object_id, Some(secret_id.as_uuid()));
    let diff = record
        .diff
        .as_ref()
        .map(|diff| diff.to_string())
        .unwrap_or_default();
    assert!(diff.contains(&agent_id.to_string()), "{diff}");
    assert!(diff.contains(&job_id.to_string()), "{diff}");
    assert!(diff.contains(&repository_id.to_string()), "{diff}");
    assert!(!diff.contains(CANARY), "{diff}");

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_03_a_rollback_leaves_no_delivery_record() -> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let tenant = create_tenant_with_key(db.database(), "secrets-rollback").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app_with_kek(db.database());
    let id = create_secret(&app, &cookie, "rollback", CANARY).await;
    let secret_id = id.parse::<SecretId>()?;

    let kek = test_kek();
    let reader = SecretReader::new(kek.as_ref());
    let mut tx = db.database().begin_tenant(tenant).await?;
    let plaintext = reader
        .read_for_delivery(
            &mut tx,
            secret_id,
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        )
        .await?;
    assert_eq!(plaintext.expose_secret(), CANARY);
    // The caller does not commit. The drop rolls the transaction back, so the delivery does not
    // happen: the record and the value are both gone (AC-RP-03).
    drop(tx);

    let records = audit_records(db.database(), tenant, AuditAction::SecretDelivered).await?;
    assert!(
        records.is_empty(),
        "a rollback must leave no delivery record"
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn ac_rp_03_the_delivery_decrypts_with_the_stored_key_version()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(db) = common::test_db().await else {
        return Ok(());
    };
    let tenant = create_tenant_with_key(db.database(), "secrets-version").await;
    let cookie = seed_session(db.database(), tenant, "admin", "admin").await;
    let app = app_with_kek(db.database());
    // Encrypt the secret under key version 1.
    let id = create_secret(&app, &cookie, "version-1", CANARY).await;
    let secret_id = id.parse::<SecretId>()?;

    // The tenant rotates the key: version 1 becomes inactive and version 2 becomes active.
    add_key_version(&db, tenant, 2).await?;

    // The delivery still decrypts with version 1, the version stored on the secret row.
    let kek = test_kek();
    let reader = SecretReader::new(kek.as_ref());
    let mut tx = db.database().begin_tenant(tenant).await?;
    let plaintext = reader
        .read_for_delivery(
            &mut tx,
            secret_id,
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
        )
        .await?;
    assert_eq!(plaintext.expose_secret(), CANARY);
    tx.commit().await?;

    db.cleanup().await;
    Ok(())
}
