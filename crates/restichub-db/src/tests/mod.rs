//! Database tests for tenant isolation and the schema (AC-TN-01 at the database layer).
//!
//! The tests read `RESTICHUB_TEST_DATABASE_URL`. If the variable is not set, the helper prints a
//! message and the tests do nothing.
//!
//! The tests are inside the crate, because the connection methods are crate-internal. The
//! isolation checks read through the runtime login. The owner-level checks use the owner pool.

mod audit;
mod auth;
mod enrollment_tokens;
mod migrations;
mod rls;
mod roles;
mod secrets;
mod tenants;

use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool};

use crate::audit::{Actor, AuditAction, AuditEvent};
use crate::ids::{ApiTokenId, AuditLogId, SecretId, SessionId, SetupTokenId, UserId};
use crate::test_util::TestDatabase;
use crate::users::NewUser;
use crate::{Database, TenantId, TenantTx, users};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The list of SECURITY DEFINER functions in the schema.
const SECURITY_DEFINER_ALLOWED: [&str; 31] = [
    "auth_find_tenant",
    "auth_find_session",
    "auth_find_api_token",
    "tenant_user_counts",
    "tenant_admin_counts",
    "auth_find_account",
    "auth_find_account_by_id",
    "auth_self_password_hash",
    "auth_list_memberships",
    "auth_tenant_members",
    "auth_tenant_member",
    "auth_create_member",
    "auth_admin_set_password",
    "auth_admin_set_display_name",
    "auth_self_set_password",
    "auth_set_account_disabled",
    "auth_delete_account_sessions",
    "auth_delete_session",
    "auth_set_last_tenant",
    "auth_add_membership",
    "auth_platform_accounts",
    "auth_platform_memberships",
    "auth_platform_create_account",
    "auth_platform_set_membership_role",
    "auth_platform_remove_membership",
    "auth_platform_set_email",
    "auth_create_platform_admin",
    "auth_platform_admin_count",
    "auth_is_platform_admin",
    "auth_set_platform_admin",
    "check_platform_admin_membership",
];

const TENANT_TABLES: [&str; 10] = [
    "users",
    "memberships",
    "sessions",
    "api_tokens",
    "tenant_keys",
    "secrets",
    "audit_log",
    "setup_tokens",
    "agents",
    "enrollment_tokens",
];

struct Seed {
    user: UserId,
    session_id: SessionId,
    session_hash: Vec<u8>,
    api_token_id: ApiTokenId,
    api_hash: Vec<u8>,
}

fn code(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .map(|code| code.to_string())
}

fn assert_code(result: Result<sqlx::postgres::PgQueryResult, sqlx::Error>, expected: &str) {
    match result {
        Ok(_) => panic!("the statement must fail"),
        Err(error) => assert_eq!(code(&error).as_deref(), Some(expected), "{error}"),
    }
}

async fn two_tenants(db: &Database) -> Result<(TenantId, TenantId), Box<dyn std::error::Error>> {
    let mut tx = db.begin_platform().await?;
    let tenant_a = crate::tenants::create(&mut tx, "tenant-a").await?;
    let tenant_b = crate::tenants::create(&mut tx, "tenant-b").await?;
    tx.commit().await?;
    Ok((tenant_a, tenant_b))
}

async fn seed_tenant(db: &Database, tenant: TenantId) -> Result<Seed, Box<dyn std::error::Error>> {
    let mut session_hash = tenant.as_uuid().as_bytes().to_vec();
    session_hash[15] ^= 0x01;
    let mut api_hash = tenant.as_uuid().as_bytes().to_vec();
    api_hash[15] ^= 0x02;

    let mut tx = db.begin_tenant(tenant).await?;
    let username = format!("alice-{tenant}");
    let user = users::create_member(
        &mut tx,
        &NewUser {
            username: &username,
            password_hash: "phc$secret",
            display_name: "Alice",
        },
        crate::Role::Admin,
    )
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

    let mut serial = tenant.as_uuid().as_bytes().to_vec();
    serial[0] ^= 0x04;
    sqlx::query(
        "INSERT INTO agents \
         (id, tenant_id, hostname, os, arch, agent_version, certificate_serial, \
          certificate_expires_at, status) \
         VALUES ($1, $2, 'host', 'linux', 'amd64', '0.1.0', $3, now() + interval '30 days', \
                 'active')",
    )
    .bind(crate::AgentId::new())
    .bind(tenant)
    .bind(&serial)
    .execute(tx.as_conn())
    .await?;

    let mut token_hash = tenant.as_uuid().as_bytes().to_vec();
    token_hash[15] ^= 0x05;
    sqlx::query(
        "INSERT INTO enrollment_tokens \
         (id, tenant_id, token_hash, display_prefix, labels, expires_at, created_by_user_id) \
         VALUES ($1, $2, $3, 'ab12cd34', ARRAY['host'], now() + interval '15 minutes', $4)",
    )
    .bind(crate::EnrollmentTokenId::new())
    .bind(tenant)
    .bind(&token_hash)
    .bind(user)
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

async fn count_rows(
    tx: &mut TenantTx<'_>,
    table: &'static str,
) -> Result<i64, Box<dyn std::error::Error>> {
    let query = format!("SELECT count(*) FROM {table}");
    let count: (i64,) = sqlx::query_as(AssertSqlSafe(query))
        .fetch_one(tx.as_conn())
        .await?;
    Ok(count.0)
}

/// Asserts that the runtime role cannot read the table directly. The table name comes from a fixed
/// list in the test only.
async fn assert_tenant_table_read_refused(
    db: &Database,
    tenant: TenantId,
    table: &'static str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut tx = db.begin_tenant(tenant).await?;
    let query = format!("SELECT count(*) FROM {table}");
    let result = sqlx::query(AssertSqlSafe(query))
        .execute(tx.as_conn())
        .await;
    match result {
        Ok(_) => panic!("the read of {table} must fail"),
        Err(error) => assert_eq!(code(&error).as_deref(), Some("42501"), "{error}"),
    }
    Ok(())
}

fn bootstrap(name: &str) -> crate::tenants::BootstrapTenant<'_> {
    crate::tenants::BootstrapTenant {
        id: TenantId::new(),
        name,
        key_version: 1,
        kek_id: "kek-1",
        wrapped_dek: &[3u8; 48],
        nonce: &[4u8; 12],
        actor: Actor::System,
        source_ip: Some(
            "203.0.113.7"
                .parse()
                .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        ),
        admin: Some(crate::tenants::BootstrapAdmin {
            username: name,
            password_hash: "phc$admin",
            display_name: "Admin",
        }),
    }
}

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

async fn admin_pool(db: &TestDatabase) -> Result<PgPool, Box<dyn std::error::Error>> {
    Ok(PgPoolOptions::new()
        .max_connections(1)
        .connect_with(db.admin_options())
        .await?)
}

async fn admin_query(admin: &PgPool, statement: &str) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(AssertSqlSafe(statement.to_owned()))
        .execute(admin)
        .await?;
    Ok(())
}

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

async fn insert_audit(
    tx: &mut TenantTx<'_>,
    action: AuditAction,
) -> Result<AuditLogId, Box<dyn std::error::Error>> {
    let mut event = AuditEvent::new(action, "user");
    event.source_ip = Some("203.0.113.7".parse()?);
    Ok(crate::audit::insert(tx, &event).await?)
}
