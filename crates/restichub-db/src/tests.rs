//! Database tests for the tenant isolation (issue #16, AC-TN-01 at the database layer).
//!
//! The tests read `RESTICHUB_TEST_DATABASE_URL`. If the variable is not set, the helper prints a
//! message and the tests do nothing.
//!
//! The tests are inside the crate, because the connection methods are crate-internal (H1). The
//! isolation checks read through the runtime login. The owner-level checks use the owner pool.

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use super::audit::{Actor, AuditAction, AuditEvent, AuditFilter, AuditPage};
use super::ids::{ApiTokenId, AuditLogId, SecretId, SessionId, SetupTokenId, UserId};
use super::test_util::{TestDatabase, test_database};
use super::users::NewUser;
use super::{Database, PLATFORM_TENANT_ID, TenantId, audit, tenants, users};

/// The error type of the tests.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The SECURITY DEFINER functions that the schema allows (N9).
const SECURITY_DEFINER_ALLOWED: [&str; 3] = [
    "auth_find_tenant",
    "auth_find_session",
    "auth_find_api_token",
];

/// The tenant tables with the general tenant policy.
const TENANT_TABLES: [&str; 8] = [
    "users",
    "memberships",
    "sessions",
    "api_tokens",
    "tenant_keys",
    "secrets",
    "audit_log",
    "setup_tokens",
];

/// The identifiers of one seeded tenant.
struct Seed {
    user: UserId,
    session_id: SessionId,
    session_hash: Vec<u8>,
    api_token_id: ApiTokenId,
    api_hash: Vec<u8>,
}

/// Returns the SQLSTATE of a database error.
fn code(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .map(|code| code.to_string())
}

/// Asserts that a statement fails with the given SQLSTATE.
fn assert_code(result: Result<sqlx::postgres::PgQueryResult, sqlx::Error>, expected: &str) {
    match result {
        Ok(_) => panic!("the statement must fail"),
        Err(error) => assert_eq!(code(&error).as_deref(), Some(expected), "{error}"),
    }
}

/// Creates two tenants through a platform transaction.
async fn two_tenants(db: &Database) -> Result<(TenantId, TenantId), Box<dyn std::error::Error>> {
    let mut tx = db.begin_platform().await?;
    let tenant_a = tenants::create(&mut tx, "tenant-a").await?;
    let tenant_b = tenants::create(&mut tx, "tenant-b").await?;
    tx.commit().await?;
    Ok((tenant_a, tenant_b))
}

/// Inserts one row of each tenant table for the tenant `tenant`.
async fn seed_tenant(db: &Database, tenant: TenantId) -> Result<Seed, Box<dyn std::error::Error>> {
    let mut session_hash = tenant.as_uuid().as_bytes().to_vec();
    session_hash[15] ^= 0x01;
    let mut api_hash = tenant.as_uuid().as_bytes().to_vec();
    api_hash[15] ^= 0x02;

    let mut tx = db.begin_tenant(tenant).await?;
    let user = users::create(
        &mut tx,
        &NewUser {
            username: "alice",
            password_hash: "phc$secret",
            display_name: "Alice",
        },
    )
    .await?;

    sqlx::query("INSERT INTO memberships (tenant_id, user_id, role) VALUES ($1, $2, 'admin')")
        .bind(tenant)
        .bind(user)
        .execute(tx.as_conn())
        .await?;

    let session_id = SessionId::new();
    sqlx::query(
        "INSERT INTO sessions (id, tenant_id, user_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, now() + interval '1 hour')",
    )
    .bind(session_id)
    .bind(tenant)
    .bind(user)
    .bind(&session_hash)
    .execute(tx.as_conn())
    .await?;

    let api_token_id = ApiTokenId::new();
    sqlx::query(
        "INSERT INTO api_tokens \
         (id, tenant_id, name, token_hash, display_prefix, role, created_by) \
         VALUES ($1, $2, 'token', $3, 'rh_abcd', 'admin', $4)",
    )
    .bind(api_token_id)
    .bind(tenant)
    .bind(&api_hash)
    .bind(user)
    .execute(tx.as_conn())
    .await?;

    sqlx::query(
        "INSERT INTO tenant_keys (tenant_id, key_version, kek_id, wrapped_dek, nonce) \
         VALUES ($1, 1, 'kek-1', $2, $3)",
    )
    .bind(tenant)
    .bind(vec![3u8; 48])
    .bind(vec![4u8; 12])
    .execute(tx.as_conn())
    .await?;

    sqlx::query(
        "INSERT INTO secrets \
         (id, tenant_id, name, purpose, key_version, nonce, ciphertext) \
         VALUES ($1, $2, 'secret', 'repository_password', 1, $3, $4)",
    )
    .bind(SecretId::new())
    .bind(tenant)
    .bind(vec![5u8; 12])
    .bind(vec![6u8; 32])
    .execute(tx.as_conn())
    .await?;

    sqlx::query(
        "INSERT INTO audit_log (id, tenant_id, actor_type, action, object_type) \
         VALUES ($1, $2, 'system', 'user_create', 'user')",
    )
    .bind(AuditLogId::new())
    .bind(tenant)
    .execute(tx.as_conn())
    .await?;

    let mut setup_hash = tenant.as_uuid().as_bytes().to_vec();
    setup_hash[15] ^= 0x03;
    sqlx::query(
        "INSERT INTO setup_tokens (id, tenant_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, now() + interval '1 hour')",
    )
    .bind(SetupTokenId::new())
    .bind(tenant)
    .bind(&setup_hash)
    .execute(tx.as_conn())
    .await?;

    tx.commit().await?;
    Ok(Seed {
        user,
        session_id,
        session_hash,
        api_token_id,
        api_hash,
    })
}

/// Counts the rows of one tenant table.
async fn count_rows(
    tx: &mut super::TenantTx<'_>,
    table: &'static str,
) -> Result<i64, Box<dyn std::error::Error>> {
    let query = format!("SELECT count(*) FROM {table}");
    let count: (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(query))
        .fetch_one(tx.as_conn())
        .await?;
    Ok(count.0)
}

#[tokio::test]
async fn ac_tn_01_db_tenant_a_cannot_read_tenant_b_rows() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_b).await?;

    let mut tx_b = db.database().begin_tenant(tenant_b).await?;
    for table in TENANT_TABLES {
        assert_eq!(count_rows(&mut tx_b, table).await?, 1, "tenant B: {table}");
    }
    tx_b.commit().await?;

    let mut tx_a = db.database().begin_tenant(tenant_a).await?;
    for table in TENANT_TABLES {
        assert_eq!(count_rows(&mut tx_a, table).await?, 0, "tenant A: {table}");
    }
    tx_a.commit().await?;

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_a_foreign_tenant_id_write_is_rejected() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    let seed_b = seed_tenant(db.database(), tenant_b).await?;
    let seed_a = seed_tenant(db.database(), tenant_a).await?;

    // The `WITH CHECK` clause rejects a row of tenant B (42501). Each negative write runs in its
    // own transaction, so an aborted transaction does not hide the next result.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let write = sqlx::query(
        "INSERT INTO users (id, tenant_id, username, password_hash, display_name) \
         VALUES ($1, $2, 'mallory', 'phc', 'Mallory')",
    )
    .bind(UserId::new())
    .bind(tenant_b)
    .execute(tx.as_conn())
    .await;
    assert_code(write, "42501");
    drop(tx);

    // The composite foreign key rejects a reference to a user of tenant B (23503).
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let reference =
        sqlx::query("INSERT INTO memberships (tenant_id, user_id, role) VALUES ($1, $2, 'viewer')")
            .bind(tenant_a)
            .bind(seed_b.user)
            .execute(tx.as_conn())
            .await;
    assert_code(reference, "23503");
    drop(tx);

    // The session of tenant B is not visible, so the foreign key fails.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let session_ref = sqlx::query(
        "INSERT INTO sessions (id, tenant_id, user_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, now() + interval '1 hour')",
    )
    .bind(SessionId::new())
    .bind(tenant_a)
    .bind(seed_b.user)
    .bind(vec![9u8; 32])
    .execute(tx.as_conn())
    .await;
    assert_code(session_ref, "23503");
    drop(tx);

    // The token of tenant B is not visible, so the foreign key fails.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let token_ref = sqlx::query(
        "INSERT INTO api_tokens \
         (id, tenant_id, name, token_hash, display_prefix, role, created_by) \
         VALUES ($1, $2, 't', $3, 'rh_x', 'admin', $4)",
    )
    .bind(ApiTokenId::new())
    .bind(tenant_a)
    .bind(vec![9u8; 32])
    .bind(seed_b.user)
    .execute(tx.as_conn())
    .await;
    assert_code(token_ref, "23503");
    drop(tx);

    // The secret references a user of tenant B, so the foreign key fails.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let secret_ref = sqlx::query(
        "INSERT INTO secrets \
         (id, tenant_id, name, purpose, key_version, nonce, ciphertext, created_by) \
         VALUES ($1, $2, 's', 'p', 1, $3, $4, $5)",
    )
    .bind(SecretId::new())
    .bind(tenant_a)
    .bind(vec![5u8; 12])
    .bind(vec![6u8; 32])
    .bind(seed_b.user)
    .execute(tx.as_conn())
    .await;
    assert_code(secret_ref, "23503");
    drop(tx);

    // The seed of tenant A is complete, so the references above are the only difference.
    assert_ne!(seed_a.user, seed_b.user);
    assert_ne!(seed_a.session_id, seed_b.session_id);
    assert_ne!(seed_a.api_token_id, seed_b.api_token_id);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_a_transaction_without_the_setting_sees_zero_rows() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (_, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_b).await?;

    // The runtime pool holds the runtime login. The role `restichub_app` has the policies.
    let mut tx = db.pool().begin().await?;
    sqlx::query("SET LOCAL ROLE restichub_app")
        .execute(&mut *tx)
        .await?;
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    drop(tx);

    assert_eq!(count.0, 0);
    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_the_runtime_role_cannot_bypass_rls() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };

    let (runtime_login, is_superuser, has_bypassrls): (String, bool, bool) = sqlx::query_as(
        "SELECT current_user, role.rolsuper, role.rolbypassrls \
         FROM pg_roles role WHERE role.rolname = current_user",
    )
    .fetch_one(db.pool())
    .await?;
    assert!(!is_superuser, "the runtime login must not be a superuser");
    assert!(!has_bypassrls, "the runtime login must not bypass RLS");

    let is_member: (bool,) =
        sqlx::query_as("SELECT pg_has_role(current_user, 'restichub_app', 'member')")
            .fetch_one(db.pool())
            .await?;
    assert!(
        is_member.0,
        "the runtime login must be a member of restichub_app"
    );

    let bypass: Option<(bool,)> =
        sqlx::query_as("SELECT rolbypassrls FROM pg_roles WHERE rolname = 'restichub_app'")
            .fetch_optional(db.pool())
            .await?;
    assert_eq!(
        bypass,
        Some((false,)),
        "restichub_app must have no BYPASSRLS"
    );

    let owner: (String,) =
        sqlx::query_as("SELECT pg_get_userbyid(relowner) FROM pg_class WHERE relname = 'users'")
            .fetch_one(db.pool())
            .await?;
    assert_ne!(
        owner.0, "restichub_app",
        "restichub_app must not own a table"
    );
    assert_ne!(
        owner.0, runtime_login,
        "the runtime login must not own a table"
    );

    let security: (bool, bool) = sqlx::query_as(
        "SELECT relrowsecurity, relforcerowsecurity FROM pg_class WHERE relname = 'users'",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(security, (true, true));

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_the_runtime_role_is_safe_and_the_owner_is_refused() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };

    // The runtime login passes the check.
    db.database().verify_runtime_role().await?;

    // The owner login owns a relation, so the check refuses it.
    let owner_database = Database {
        pool: db.owner_pool().clone(),
    };
    assert!(owner_database.verify_runtime_role().await.is_err());

    // A superuser is refused.
    let config = super::DatabaseConfig {
        url: std::env::var("RESTICHUB_TEST_DATABASE_URL")?,
        max_connections: 1,
        acquire_timeout: std::time::Duration::from_secs(5),
    };
    let admin = Database::connect(&config, None).await?;
    assert!(admin.verify_runtime_role().await.is_err());
    admin.close().await;

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_the_session_setting_does_not_leak_on_a_pooled_connection() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;

    let pool = db.runtime_pool_with_max(1).await?;

    let mut connection = pool.acquire().await?;
    sqlx::query("SET restichub.scope = 'platform'")
        .execute(&mut *connection)
        .await?;
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, false)")
        .bind(tenant_a.to_string())
        .execute(&mut *connection)
        .await?;
    sqlx::query("SET ROLE restichub_app")
        .execute(&mut *connection)
        .await?;
    let role_before: (String,) = sqlx::query_as("SELECT current_user")
        .fetch_one(&mut *connection)
        .await?;
    drop(connection);
    assert_eq!(role_before.0, "restichub_app");

    // The pool holds one connection. The `RESET ROLE` and `RESET ALL` hook must clear the state
    // (M2, N7).
    let mut connection = pool.acquire().await?;
    let (scope, tenant, current): (Option<String>, Option<String>, String) = sqlx::query_as(
        "SELECT current_setting('restichub.scope', true), \
                current_setting('restichub.tenant_id', true), \
                current_user",
    )
    .fetch_one(&mut *connection)
    .await?;
    drop(connection);
    pool.close().await;

    assert_ne!(current, "restichub_app");
    assert_ne!(scope.as_deref(), Some("platform"));
    assert!(
        scope.is_none() || scope.as_deref() == Some(""),
        "the scope must be cleared: {scope:?}"
    );
    assert!(
        tenant.is_none() || tenant.as_deref() == Some(""),
        "the tenant id must be cleared: {tenant:?}"
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_the_audit_log_is_append_only() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (_, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_b).await?;

    // The runtime role has no UPDATE and no DELETE privilege on the audit log (42501).
    let mut tx = db.database().begin_tenant(tenant_b).await?;
    let update = sqlx::query("UPDATE audit_log SET action = 'x'")
        .execute(tx.as_conn())
        .await;
    assert_code(update, "42501");
    drop(tx);

    // The owner cannot change, remove or truncate a row. The trigger rejects the operation.
    let mut tx = db.owner_pool().begin().await?;
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant_b.to_string())
        .execute(&mut *tx)
        .await?;
    let owner_update = sqlx::query("UPDATE audit_log SET action = 'x'")
        .execute(&mut *tx)
        .await;
    assert_code(owner_update, "P0001");
    drop(tx);

    let mut tx = db.owner_pool().begin().await?;
    sqlx::query("SELECT set_config('restichub.tenant_id', $1, true)")
        .bind(tenant_b.to_string())
        .execute(&mut *tx)
        .await?;
    let owner_delete = sqlx::query("DELETE FROM audit_log").execute(&mut *tx).await;
    assert_code(owner_delete, "P0001");
    drop(tx);

    let owner_truncate = sqlx::query("TRUNCATE audit_log")
        .execute(db.owner_pool())
        .await;
    assert_code(owner_truncate, "P0001");

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_the_platform_scope_manages_tenants() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;

    let mut tx = db.database().begin_platform().await?;
    let created = tenants::create(&mut tx, "platform-created").await?;
    let list = tenants::list(&mut tx).await?;
    assert!(list.iter().any(|tenant| tenant.id == created));
    assert!(list.iter().any(|tenant| tenant.id == PLATFORM_TENANT_ID));
    tx.commit().await?;

    // A tenant scope sees its own row only and cannot write `tenants`.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let rows: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM tenants")
        .fetch_all(tx.as_conn())
        .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows.first().map(|row| row.0), Some(tenant_a.as_uuid()));
    drop(tx);

    // A tenant scope cannot insert a row (no permissive INSERT policy, 42501).
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let insert = sqlx::query("INSERT INTO tenants (id, tenant_id, name) VALUES ($1, $2, 'x')")
        .bind(TenantId::new().as_uuid())
        .bind(tenant_a)
        .execute(tx.as_conn())
        .await;
    assert_code(insert, "42501");
    drop(tx);

    // A tenant scope cannot change its status. The update touches no row and reports no error.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let status = sqlx::query("UPDATE tenants SET status = 'suspended' WHERE id = $1")
        .bind(tenant_a)
        .execute(tx.as_conn())
        .await?;
    assert_eq!(status.rows_affected(), 0);
    tx.commit().await?;

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_the_platform_scope_sees_no_user_rows() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (_, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_b).await?;

    let mut tx = db.database().begin_platform().await?;
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM users")
        .fetch_one(tx.as_conn())
        .await?;
    tx.commit().await?;
    assert_eq!(count.0, 0);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_kek_check_is_visible_only_in_the_platform_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;

    let mut tx = db.database().begin_tenant(PLATFORM_TENANT_ID).await?;
    sqlx::query(
        "INSERT INTO kek_check (tenant_id, kek_id, nonce, ciphertext) VALUES ($1, 'kek-1', $2, $3)",
    )
    .bind(PLATFORM_TENANT_ID)
    .bind(vec![7u8; 12])
    .bind(vec![8u8; 32])
    .execute(tx.as_conn())
    .await?;
    let platform_count: (i64,) = sqlx::query_as("SELECT count(*) FROM kek_check")
        .fetch_one(tx.as_conn())
        .await?;
    tx.commit().await?;
    assert_eq!(platform_count.0, 1);

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let other_count: (i64,) = sqlx::query_as("SELECT count(*) FROM kek_check")
        .fetch_one(tx.as_conn())
        .await?;
    tx.commit().await?;
    assert_eq!(other_count.0, 0);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_an_update_cannot_move_a_row_to_another_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let move_row = sqlx::query("UPDATE users SET tenant_id = $1")
        .bind(tenant_b)
        .execute(tx.as_conn())
        .await;
    assert_code(move_row, "42501");
    drop(tx);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_a_delete_leaves_the_rows_of_the_other_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;
    seed_tenant(db.database(), tenant_b).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let deleted = sqlx::query("DELETE FROM sessions")
        .execute(tx.as_conn())
        .await?;
    tx.commit().await?;
    assert_eq!(deleted.rows_affected(), 1);

    let mut tx = db.database().begin_tenant(tenant_b).await?;
    let remaining: (i64,) = sqlx::query_as("SELECT count(*) FROM sessions")
        .fetch_one(tx.as_conn())
        .await?;
    tx.commit().await?;
    assert_eq!(remaining.0, 1);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_the_runtime_login_cannot_escalate() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;

    // The runtime login is not the table owner, so it cannot disable the forced Row-Level Security.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    sqlx::query("RESET ROLE").execute(tx.as_conn()).await?;
    let no_force = sqlx::query("ALTER TABLE users NO FORCE ROW LEVEL SECURITY")
        .execute(tx.as_conn())
        .await;
    assert_code(no_force, "42501");
    drop(tx);

    // The runtime login cannot become another role.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let set_role = sqlx::query("SET ROLE restichub_auth")
        .execute(tx.as_conn())
        .await;
    assert_code(set_role, "42501");
    drop(tx);

    // The runtime login cannot become the owner login either.
    let owner_role = db.owner_role().to_owned();
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let set_owner = sqlx::query(sqlx::AssertSqlSafe(format!("SET ROLE \"{owner_role}\"")))
        .execute(tx.as_conn())
        .await;
    assert_code(set_owner, "42501");
    drop(tx);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_a_tenant_creation_commits_atomically() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (_, _) = two_tenants(db.database()).await?;

    // A created tenant, its key row, its first admin and its audit record commit together (N2/D3).
    let mut tx = db.database().begin_platform().await?;
    let committed = tenants::create_with_bootstrap(&mut tx, &bootstrap("committed")).await?;
    tx.commit().await?;
    assert!(tenant_exists(db.database(), committed).await?);
    assert_eq!(
        count_in_tenant(db.database(), committed, "tenant_keys").await?,
        1
    );
    assert_eq!(count_in_tenant(db.database(), committed, "users").await?, 1);
    assert_eq!(
        count_in_tenant(db.database(), committed, "memberships").await?,
        1
    );
    assert_eq!(
        count_in_tenant(db.database(), committed, "audit_log").await?,
        1
    );

    // A dropped transaction leaves nothing (D3).
    let mut tx = db.database().begin_platform().await?;
    let crashed = tenants::create_with_bootstrap(&mut tx, &bootstrap("crashed")).await?;
    drop(tx);
    assert!(!tenant_exists(db.database(), crashed).await?);

    db.cleanup().await;
    Ok(())
}

/// Returns a bootstrap input for a test tenant (N2).
fn bootstrap(name: &str) -> tenants::BootstrapTenant<'_> {
    tenants::BootstrapTenant {
        name,
        key_version: 1,
        kek_id: "kek-1",
        wrapped_dek: &[3u8; 48],
        nonce: &[4u8; 12],
        admin: Some(tenants::BootstrapAdmin {
            username: "admin",
            password_hash: "phc$admin",
            display_name: "Admin",
        }),
    }
}

/// Counts the rows of one tenant table in a tenant transaction.
async fn count_in_tenant(
    db: &Database,
    tenant: TenantId,
    table: &'static str,
) -> Result<i64, Box<dyn std::error::Error>> {
    let mut tx = db.begin_tenant(tenant).await?;
    let count = count_rows(&mut tx, table).await?;
    tx.commit().await?;
    Ok(count)
}

#[tokio::test]
async fn db_enter_tenant_refuses_a_foreign_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (_, tenant_b) = two_tenants(db.database()).await?;

    // The tenant exists, but this transaction did not create it (N2).
    let mut tx = db.database().begin_platform().await?;
    assert!(tx.enter_tenant(tenant_b).await.is_err());
    drop(tx);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_a_platform_audit_insert_is_limited_to_the_entered_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (_, tenant_b) = two_tenants(db.database()).await?;

    let mut tx = db.database().begin_platform().await?;
    let created = tenants::create(&mut tx, "audit-target").await?;
    tx.enter_tenant(created).await?;

    // The entered tenant is accepted.
    let entered = sqlx::query(
        "INSERT INTO audit_log (id, tenant_id, actor_type, action, object_type) \
         VALUES ($1, $2, 'system', 'x', 'tenant')",
    )
    .bind(AuditLogId::new())
    .bind(created)
    .execute(tx.as_conn())
    .await;
    assert!(entered.is_ok());

    // The platform tenant is accepted.
    let platform = sqlx::query(
        "INSERT INTO audit_log (id, tenant_id, actor_type, action, object_type) \
         VALUES ($1, $2, 'system', 'x', 'tenant')",
    )
    .bind(AuditLogId::new())
    .bind(PLATFORM_TENANT_ID)
    .execute(tx.as_conn())
    .await;
    assert!(platform.is_ok());

    // Another existing tenant is refused (42501).
    let foreign = sqlx::query(
        "INSERT INTO audit_log (id, tenant_id, actor_type, action, object_type) \
         VALUES ($1, $2, 'system', 'x', 'tenant')",
    )
    .bind(AuditLogId::new())
    .bind(tenant_b)
    .execute(tx.as_conn())
    .await;
    assert_code(foreign, "42501");
    drop(tx);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_verify_migrations_detects_a_newer_schema() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };

    // The database matches the embedded migrations.
    db.database().verify_migrations().await?;

    // A newer applied version is reported as ahead (N4).
    let fake = 9_999_999_i64;
    sqlx::query(
        "INSERT INTO _sqlx_migrations \
         (version, description, installed_on, success, checksum, execution_time) \
         VALUES ($1, 'fake', now(), true, $2, 0)",
    )
    .bind(fake)
    .bind(vec![0u8; 32])
    .execute(db.owner_pool())
    .await?;
    assert!(db.database().verify_migrations().await.is_err());

    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = $1")
        .bind(fake)
        .execute(db.owner_pool())
        .await?;
    db.database().verify_migrations().await?;

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_the_runtime_role_check_refuses_unsafe_setups() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let admin = admin_pool(&db).await?;
    let suffix = Uuid::now_v7().simple().to_string();

    // The runtime login owns the current database.
    let scratch = format!("restichub_scratch_{suffix}");
    admin_query(
        &admin,
        &format!(
            "CREATE DATABASE \"{scratch}\" OWNER \"{}\"",
            db.runtime_role()
        ),
    )
    .await?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(db.runtime_options().database(scratch.as_str()))
        .await?;
    let scratch_database = Database { pool };
    assert!(scratch_database.verify_runtime_role().await.is_err());
    scratch_database.close().await;
    admin_query(
        &admin,
        &format!("DROP DATABASE IF EXISTS \"{scratch}\" WITH (FORCE)"),
    )
    .await?;

    // A substitute group role owns BYPASSRLS. The group role is cluster-wide, so a test does not
    // change `restichub_app` itself (N1).
    let probe_app = format!("restichub_app_probe_{suffix}");
    let probe_login = format!("restichub_probe_login_{suffix}");
    let probe_password = Uuid::now_v7().simple().to_string();
    admin_query(
        &admin,
        &format!(
            "CREATE ROLE \"{probe_app}\" NOLOGIN NOSUPERUSER BYPASSRLS NOCREATEROLE NOCREATEDB"
        ),
    )
    .await?;
    admin_query(
        &admin,
        &format!(
            "CREATE ROLE \"{probe_login}\" LOGIN NOSUPERUSER NOBYPASSRLS NOCREATEROLE NOCREATEDB \
             PASSWORD '{probe_password}'"
        ),
    )
    .await?;
    admin_query(
        &admin,
        &format!("GRANT \"{probe_app}\" TO \"{probe_login}\""),
    )
    .await?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(
            db.runtime_options()
                .username(probe_login.as_str())
                .password(probe_password.as_str()),
        )
        .await?;
    let probe_database = Database { pool };
    assert!(
        probe_database
            .verify_runtime_role_for(&probe_app)
            .await
            .is_err()
    );
    probe_database.close().await;

    // The runtime login is a member of restichub_auth (BYPASSRLS).
    admin_query(
        &admin,
        &format!("GRANT restichub_auth TO \"{}\"", db.runtime_role()),
    )
    .await?;
    assert!(db.database().verify_runtime_role().await.is_err());

    // Clean up the substitute roles.
    for name in [&probe_login, &probe_app] {
        admin_query(&admin, &format!("DROP ROLE IF EXISTS \"{name}\"")).await?;
    }
    admin.close().await;

    db.cleanup().await;
    Ok(())
}

/// Returns an administrator (superuser) pool for a test.
async fn admin_pool(db: &TestDatabase) -> Result<PgPool, Box<dyn std::error::Error>> {
    Ok(PgPoolOptions::new()
        .max_connections(1)
        .connect_with(db.admin_options())
        .await?)
}

/// Runs one statement as the administrator.
async fn admin_query(admin: &PgPool, statement: &str) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
        .execute(admin)
        .await?;
    Ok(())
}

/// Checks the tenant row through a platform transaction.
async fn tenant_exists(
    db: &Database,
    tenant: TenantId,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut tx = db.begin_platform().await?;
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM tenants WHERE id = $1")
        .bind(tenant)
        .fetch_one(tx.as_conn())
        .await?;
    tx.commit().await?;
    Ok(count.0 == 1)
}

#[tokio::test]
async fn db_secrets_require_a_tenant_key_and_one_active_key() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;

    // A secret with no matching key row fails (23503).
    let unknown_key = sqlx::query(
        "INSERT INTO secrets \
         (id, tenant_id, name, purpose, key_version, nonce, ciphertext) \
         VALUES ($1, $2, 's2', 'p', 99, $3, $4)",
    )
    .bind(SecretId::new())
    .bind(tenant_a)
    .bind(vec![5u8; 12])
    .bind(vec![6u8; 32])
    .execute(tx.as_conn())
    .await;
    assert_code(unknown_key, "23503");
    drop(tx);

    // A second active key row for the same tenant fails (23505).
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let second_active = sqlx::query(
        "INSERT INTO tenant_keys (tenant_id, key_version, kek_id, wrapped_dek, nonce) \
         VALUES ($1, 2, 'kek-2', $2, $3)",
    )
    .bind(tenant_a)
    .bind(vec![3u8; 48])
    .bind(vec![4u8; 12])
    .execute(tx.as_conn())
    .await;
    assert_code(second_active, "23505");
    drop(tx);

    // The runtime role cannot delete a key row (42501).
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let delete_key = sqlx::query("DELETE FROM tenant_keys")
        .execute(tx.as_conn())
        .await;
    assert_code(delete_key, "42501");
    drop(tx);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_an_api_token_cannot_be_a_platform_admin_in_a_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    let seed = seed_tenant(db.database(), tenant_a).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let bad_role = sqlx::query(
        "INSERT INTO api_tokens \
         (id, tenant_id, name, token_hash, display_prefix, role, created_by) \
         VALUES ($1, $2, 't', $3, 'rh_x', 'platform_admin', $4)",
    )
    .bind(ApiTokenId::new())
    .bind(tenant_a)
    .bind(vec![11u8; 32])
    .bind(seed.user)
    .execute(tx.as_conn())
    .await;
    assert_code(bad_role, "23514");
    drop(tx);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_the_lower_case_unique_indexes_hold() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;

    // The tenant name is unique and ignores the letter case (D6).
    let mut tx = db.database().begin_platform().await?;
    let duplicate_tenant = tenants::create(&mut tx, "TENANT-A").await;
    assert!(duplicate_tenant.is_err(), "the name must be unique");
    drop(tx);

    // The username is unique for the tenant and ignores the letter case (D6).
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    users::create(
        &mut tx,
        &NewUser {
            username: "Alice",
            password_hash: "phc",
            display_name: "Alice",
        },
    )
    .await?;
    let duplicate_user = users::create(
        &mut tx,
        &NewUser {
            username: "alice",
            password_hash: "phc",
            display_name: "Alice",
        },
    )
    .await;
    assert!(duplicate_user.is_err(), "the username must be unique");
    drop(tx);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_the_auth_lookups_work_under_the_runtime_role() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    let seed = seed_tenant(db.database(), tenant_b).await?;

    // The lookups run in a tenant transaction of another tenant. The functions must find tenant B.
    let mut tx = db.database().begin_tenant(tenant_a).await?;

    let tenant: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id, status FROM auth_find_tenant($1)")
            .bind("tenant-b")
            .fetch_optional(tx.as_conn())
            .await?;
    assert_eq!(tenant.map(|row| row.0), Some(tenant_b.as_uuid()));

    let session: Option<(Uuid, Uuid, Uuid)> =
        sqlx::query_as("SELECT tenant_id, session_id, user_id FROM auth_find_session($1)")
            .bind(&seed.session_hash)
            .fetch_optional(tx.as_conn())
            .await?;
    assert_eq!(session.map(|row| row.0), Some(tenant_b.as_uuid()));
    assert_eq!(session.map(|row| row.1), Some(seed.session_id.as_uuid()));

    let token: Option<(Uuid, Uuid, String)> =
        sqlx::query_as("SELECT tenant_id, token_id, role FROM auth_find_api_token($1)")
            .bind(&seed.api_hash)
            .fetch_optional(tx.as_conn())
            .await?;
    assert_eq!(token.as_ref().map(|row| row.0), Some(tenant_b.as_uuid()));
    assert_eq!(
        token.as_ref().map(|row| row.2.clone()),
        Some("admin".to_owned())
    );

    // Unknown values return nothing.
    let unknown_session: Option<(Uuid,)> =
        sqlx::query_as("SELECT tenant_id FROM auth_find_session($1)")
            .bind(vec![0u8; 32])
            .fetch_optional(tx.as_conn())
            .await?;
    assert!(unknown_session.is_none());
    let unknown_tenant: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM auth_find_tenant($1)")
        .bind("does-not-exist")
        .fetch_optional(tx.as_conn())
        .await?;
    assert!(unknown_tenant.is_none());

    // The runtime role cannot read the other columns of tenant B directly.
    let cross: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE tenant_id = $1")
        .bind(tenant_b)
        .fetch_optional(tx.as_conn())
        .await?;
    assert!(cross.is_none());

    tx.commit().await?;
    db.cleanup().await;
    Ok(())
}

/// Inserts one audit record in a tenant transaction.
async fn insert_audit(
    tx: &mut super::TenantTx<'_>,
    action: AuditAction,
) -> Result<AuditLogId, Box<dyn std::error::Error>> {
    let mut event = AuditEvent::new(action, "user");
    event.source_ip = Some("203.0.113.7".parse()?);
    Ok(audit::insert(tx, &event).await?)
}

#[tokio::test]
async fn db_audit_insert_and_list_in_a_tenant_scope() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let first = insert_audit(&mut tx, AuditAction::UserCreate).await?;
    let second = insert_audit(&mut tx, AuditAction::SignIn).await?;
    tx.commit().await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let records = audit::list(&mut tx, &AuditFilter::default(), &AuditPage::default())
        .await?
        .records;
    tx.commit().await?;
    assert_eq!(records.len(), 2);
    let ids: Vec<AuditLogId> = records.iter().map(|record| record.id).collect();
    assert!(ids.contains(&first));
    assert!(ids.contains(&second));
    assert!(records.iter().all(|record| record.tenant_id == tenant_a
        && record.action == AuditAction::UserCreate
        || record.action == AuditAction::SignIn));
    assert!(records.iter().all(|record| record.source_ip.is_some()));

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_audit_list_does_not_show_another_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    insert_audit(&mut tx, AuditAction::ConfigCreate).await?;
    tx.commit().await?;

    let mut tx = db.database().begin_tenant(tenant_b).await?;
    let records = audit::list(&mut tx, &AuditFilter::default(), &AuditPage::default())
        .await?
        .records;
    tx.commit().await?;
    assert!(records.is_empty());

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_audit_platform_list_sees_every_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    insert_audit(&mut tx, AuditAction::JobRun).await?;
    tx.commit().await?;
    let mut tx = db.database().begin_tenant(tenant_b).await?;
    insert_audit(&mut tx, AuditAction::JobCancel).await?;
    tx.commit().await?;

    let mut tx = db.database().begin_platform().await?;
    let records = audit::list_platform(&mut tx, &AuditFilter::default(), &AuditPage::default())
        .await?
        .records;
    tx.commit().await?;
    let tenants: Vec<TenantId> = records.iter().map(|record| record.tenant_id).collect();
    assert!(tenants.contains(&tenant_a));
    assert!(tenants.contains(&tenant_b));

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_audit_platform_insert_follows_the_policy() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (_, tenant_b) = two_tenants(db.database()).await?;

    let mut tx = db.database().begin_platform().await?;
    let created = tenants::create(&mut tx, "audit-target").await?;
    tx.enter_tenant(created).await?;
    let event = AuditEvent::new(AuditAction::TenantCreate, "tenant");

    // The entered tenant is accepted.
    assert!(
        audit::insert_platform(&mut tx, created, &event)
            .await
            .is_ok()
    );
    // The platform tenant is accepted.
    assert!(
        audit::insert_platform(&mut tx, PLATFORM_TENANT_ID, &event)
            .await
            .is_ok()
    );
    // Another tenant is refused (42501).
    assert!(
        audit::insert_platform(&mut tx, tenant_b, &event)
            .await
            .is_err()
    );
    drop(tx);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_audit_filter_selects_action_and_object() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    insert_audit(&mut tx, AuditAction::SignIn).await?;
    insert_audit(&mut tx, AuditAction::UserCreate).await?;
    tx.commit().await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let filter = AuditFilter {
        action: Some(AuditAction::SignIn),
        actor: Some(Actor::System),
        ..AuditFilter::default()
    };
    let records = audit::list(&mut tx, &filter, &AuditPage::default())
        .await?
        .records;
    tx.commit().await?;
    assert_eq!(records.len(), 1);
    assert_eq!(
        records.first().map(|record| &record.action),
        Some(&AuditAction::SignIn)
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_audit_pagination_is_stable() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    for _ in 0..5 {
        insert_audit(&mut tx, AuditAction::JobRun).await?;
    }
    tx.commit().await?;

    let mut seen = Vec::new();
    let mut after = None;
    for _ in 0..3 {
        let page = AuditPage { limit: 2, after };
        let mut tx = db.database().begin_tenant(tenant_a).await?;
        let records = audit::list(&mut tx, &AuditFilter::default(), &page)
            .await?
            .records;
        tx.commit().await?;
        if records.len() < 2 {
            seen.extend(records.iter().map(|record| record.id));
            break;
        }
        after = records.last().map(|record| audit::AuditCursor {
            occurred_at: record.occurred_at,
            id: record.id,
        });
        seen.extend(records.iter().map(|record| record.id));
    }

    // Every record appears once.
    assert_eq!(seen.len(), 5);
    let mut unique = seen.clone();
    unique.sort_by_key(|id| id.as_uuid());
    unique.dedup();
    assert_eq!(unique.len(), 5, "the pages repeat a record");

    // The first page is stable.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let first = audit::list(
        &mut tx,
        &AuditFilter::default(),
        &AuditPage {
            limit: 2,
            after: None,
        },
    )
    .await?
    .records;
    tx.commit().await?;
    assert_eq!(first.len(), 2);
    assert_eq!(first.first().map(|record| record.id), seen.first().copied());

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_audit_list_keeps_an_unknown_action_and_actor() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;

    // A row of a newer schema. The list must not fail and must keep the row (M5). The actor type
    // is a known type with no id, and the action string is not known.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    sqlx::query(
        "INSERT INTO audit_log (id, tenant_id, actor_type, actor_id, action, object_type) \
         VALUES ($1, $2, 'user', NULL, 'future_x', 'thing')",
    )
    .bind(AuditLogId::new())
    .bind(tenant_a)
    .execute(tx.as_conn())
    .await?;
    tx.commit().await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let records = audit::list(&mut tx, &AuditFilter::default(), &AuditPage::default())
        .await?
        .records;
    tx.commit().await?;
    assert_eq!(records.len(), 1);
    assert_eq!(
        records.first().map(|record| record.action.as_str()),
        Some("future_x")
    );
    assert_eq!(
        records.first().map(|record| record.actor.actor_type()),
        Some("user")
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_audit_page_reports_has_more() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    for _ in 0..3 {
        insert_audit(&mut tx, AuditAction::JobRun).await?;
    }
    tx.commit().await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let first = audit::list(
        &mut tx,
        &AuditFilter::default(),
        &AuditPage {
            limit: 2,
            after: None,
        },
    )
    .await?;
    tx.commit().await?;
    assert_eq!(first.records.len(), 2);
    assert!(first.has_more);

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let last = audit::list(
        &mut tx,
        &AuditFilter::default(),
        &AuditPage {
            limit: 10,
            after: None,
        },
    )
    .await?;
    tx.commit().await?;
    assert_eq!(last.records.len(), 3);
    assert!(!last.has_more);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_every_relation_has_forced_rls_and_a_policy() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };

    let rows: Vec<(String, String, String, bool, bool, i64, String)> = sqlx::query_as(
        "SELECT n.nspname, c.relname, c.relkind::text, c.relrowsecurity, c.relforcerowsecurity, \
                (SELECT count(*) FROM pg_policy p WHERE p.polrelid = c.oid), \
                coalesce(array_to_string(c.reloptions, ','), '') \
         FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE c.relkind IN ('r', 'p', 'v', 'm') \
           AND n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
           AND c.relname <> '_sqlx_migrations' \
         ORDER BY n.nspname, c.relname",
    )
    .fetch_all(db.pool())
    .await?;
    assert!(!rows.is_empty());

    for (schema, name, kind, row_security, force_security, policies, options) in &rows {
        match kind.as_str() {
            "r" | "p" => {
                assert!(row_security, "{schema}.{name}: no row-level security");
                assert!(
                    force_security,
                    "{schema}.{name}: no forced row-level security"
                );
                assert!(*policies >= 1, "{schema}.{name}: no policy");
                let has_tenant: (i64,) = sqlx::query_as(
                    "SELECT count(*) FROM information_schema.columns \
                     WHERE table_schema = $1 AND table_name = $2 AND column_name = 'tenant_id'",
                )
                .bind(schema)
                .bind(name)
                .fetch_one(db.pool())
                .await?;
                assert_eq!(has_tenant.0, 1, "{schema}.{name}: no tenant_id column");
            }
            "v" | "m" => {
                assert!(
                    options.contains("security_invoker=true"),
                    "{schema}.{name}: no security_invoker"
                );
            }
            other => panic!("unexpected relation kind {other}"),
        }
    }

    let names: Vec<&str> = rows.iter().map(|row| row.1.as_str()).collect();
    for expected in TENANT_TABLES {
        assert!(names.contains(&expected), "the table {expected} is missing");
    }
    assert!(names.contains(&"tenants"), "the table tenants is missing");
    assert!(
        names.contains(&"kek_check"),
        "the table kek_check is missing"
    );

    // Every SECURITY DEFINER function is in the allow-list and sets the search path (N9).
    let functions: Vec<(String, Option<Vec<String>>)> = sqlx::query_as(
        "SELECT p.proname, p.proconfig \
         FROM pg_proc p \
         JOIN pg_namespace n ON n.oid = p.pronamespace \
         WHERE p.prosecdef AND n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
         ORDER BY p.proname",
    )
    .fetch_all(db.pool())
    .await?;
    assert_eq!(
        functions.len(),
        SECURITY_DEFINER_ALLOWED.len(),
        "unexpected SECURITY DEFINER functions: {functions:?}"
    );
    for (name, config) in &functions {
        assert!(
            SECURITY_DEFINER_ALLOWED.contains(&name.as_str()),
            "the SECURITY DEFINER function {name} is not in the allow-list"
        );
        let has_search_path = config.as_ref().is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.starts_with("search_path="))
        });
        assert!(has_search_path, "the function {name} has no search_path");
    }

    db.cleanup().await;
    Ok(())
}
