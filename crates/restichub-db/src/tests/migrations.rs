use std::borrow::Cow;

use sqlx::AssertSqlSafe;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;

use super::{TestResult, admin_pool, admin_query};
use crate::MIGRATOR;
use crate::test_util::test_database;

#[tokio::test]
async fn db_verify_migrations_detects_a_newer_schema() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };

    db.database().verify_migrations().await?;

    // Ahead: a newer applied version.
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

/// AC-AU-03..07 support: a username that exists in two tenants stops the account migration
/// (ADR-0025). The message names the two tenants and the username.
#[tokio::test]
async fn db_the_account_migration_refuses_a_duplicate_username() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let admin = admin_pool(&db).await?;
    let name = format!("restichub_mig_{}", uuid::Uuid::now_v7().simple());
    admin_query(&admin, &format!("CREATE DATABASE \"{name}\"")).await?;

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(db.admin_options().database(name.as_str()))
        .await?;

    // Apply the migrations that precede the account model.
    let before: Vec<_> = MIGRATOR
        .iter()
        .filter(|migration| migration.version <= 15)
        .cloned()
        .collect();
    let up_to_15 = Migrator {
        migrations: Cow::Owned(before),
        ignore_missing: false,
        locking: true,
        no_tx: false,
        table_name: Cow::Borrowed("_sqlx_migrations"),
        create_schemas: Cow::Owned(Vec::new()),
    };
    up_to_15.run(&pool).await?;

    // Two tenants hold the same username, without the letter case.
    let tenant_a = uuid::Uuid::now_v7();
    let tenant_b = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tenants (id, tenant_id, name) VALUES ($1, $1, 'dup-a'), ($2, $2, 'dup-b')",
    )
    .bind(tenant_a)
    .bind(tenant_b)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO users (id, tenant_id, username, password_hash, display_name) \
         VALUES ($1, $3, 'dup', 'phc', 'Dup'), ($2, $4, 'DUP', 'phc', 'Dup')",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(uuid::Uuid::now_v7())
    .bind(tenant_a)
    .bind(tenant_b)
    .execute(&pool)
    .await?;

    // The account migration stops and names the two tenants and the username.
    let sql = MIGRATOR
        .iter()
        .find(|migration| migration.version == 16)
        .expect("the account migration is present")
        .sql
        .as_str()
        .to_owned();
    let result = sqlx::raw_sql(AssertSqlSafe(sql)).execute(&pool).await;
    let error = result.expect_err("a duplicate username must stop the migration");
    let message = error.to_string();
    assert!(
        message.contains("dup") && message.contains("dup-a") && message.contains("dup-b"),
        "the message must name the username and the two tenants: {message}"
    );

    pool.close().await;
    admin_query(
        &admin,
        &format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"),
    )
    .await?;
    admin.close().await;
    db.cleanup().await;
    Ok(())
}

/// AC-AU-18: the migration moves a `platform_admin` membership to the account right (ADR-0029).
#[tokio::test]
async fn db_the_platform_admin_migration_moves_the_membership_to_a_right() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let admin = admin_pool(&db).await?;
    let name = format!("restichub_mig_{}", uuid::Uuid::now_v7().simple());
    admin_query(&admin, &format!("CREATE DATABASE \"{name}\"")).await?;

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(db.admin_options().database(name.as_str()))
        .await?;

    // Apply the migrations that precede the right.
    let before: Vec<_> = MIGRATOR
        .iter()
        .filter(|migration| migration.version <= 23)
        .cloned()
        .collect();
    let up_to_20 = Migrator {
        migrations: Cow::Owned(before),
        ignore_missing: false,
        locking: true,
        no_tx: false,
        table_name: Cow::Borrowed("_sqlx_migrations"),
        create_schemas: Cow::Owned(Vec::new()),
    };
    up_to_20.run(&pool).await?;

    // A platform tenant account with a `platform_admin` membership.
    let user = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO users (id, tenant_id, username, password_hash, display_name) \
         VALUES ($1, '00000000-0000-7000-8000-000000000000', 'root', 'phc', 'Root')",
    )
    .bind(user)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO memberships (tenant_id, user_id, role) \
         VALUES ('00000000-0000-7000-8000-000000000000', $1, 'platform_admin')",
    )
    .bind(user)
    .execute(&pool)
    .await?;

    // Apply the right migration.
    let sql = MIGRATOR
        .iter()
        .find(|migration| migration.version == 24)
        .expect("the platform admin right migration is present")
        .sql
        .as_str()
        .to_owned();
    sqlx::raw_sql(AssertSqlSafe(sql)).execute(&pool).await?;

    let (flag,): (bool,) = sqlx::query_as("SELECT platform_admin FROM users WHERE id = $1")
        .bind(user)
        .fetch_one(&pool)
        .await?;
    assert!(flag, "the account must hold the Platform Admin right");
    let (memberships,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM memberships WHERE user_id = $1")
            .bind(user)
            .fetch_one(&pool)
            .await?;
    assert_eq!(memberships, 0, "the platform_admin membership must be gone");

    pool.close().await;
    admin_query(
        &admin,
        &format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"),
    )
    .await?;
    admin.close().await;
    db.cleanup().await;
    Ok(())
}

/// A disabled `platform_admin` membership gives no right, and every platform API token is revoked
/// (ADR-0029).
#[tokio::test]
async fn db_the_right_migration_skips_a_revoked_membership_and_revokes_the_tokens() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let admin = admin_pool(&db).await?;
    let name = format!("restichub_mig_{}", uuid::Uuid::now_v7().simple());
    admin_query(&admin, &format!("CREATE DATABASE \"{name}\"")).await?;

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(db.admin_options().database(name.as_str()))
        .await?;

    // Apply the migrations that precede the right.
    let before: Vec<_> = MIGRATOR
        .iter()
        .filter(|migration| migration.version <= 23)
        .cloned()
        .collect();
    let up_to_21 = Migrator {
        migrations: Cow::Owned(before),
        ignore_missing: false,
        locking: true,
        no_tx: false,
        table_name: Cow::Borrowed("_sqlx_migrations"),
        create_schemas: Cow::Owned(Vec::new()),
    };
    up_to_21.run(&pool).await?;

    // One account with an enabled membership and one with a disabled (revoked) membership.
    let enabled = uuid::Uuid::now_v7();
    let revoked = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO users (id, tenant_id, username, password_hash, display_name) \
         VALUES ($1, '00000000-0000-7000-8000-000000000000', 'enabled', 'phc', 'Enabled'), \
                ($2, '00000000-0000-7000-8000-000000000000', 'revoked', 'phc', 'Revoked')",
    )
    .bind(enabled)
    .bind(revoked)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO memberships (tenant_id, user_id, role, disabled) \
         VALUES ('00000000-0000-7000-8000-000000000000', $1, 'platform_admin', false), \
                ('00000000-0000-7000-8000-000000000000', $2, 'platform_admin', true)",
    )
    .bind(enabled)
    .bind(revoked)
    .execute(&pool)
    .await?;

    // A live platform-tenant platform_admin token.
    sqlx::query(
        "INSERT INTO api_tokens \
         (id, tenant_id, name, token_hash, display_prefix, role, created_by, revoked_at) \
         VALUES ($1, '00000000-0000-7000-8000-000000000000', 'old', $2, 'rh_deadbeef', \
                 'platform_admin', $3, NULL)",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(vec![9u8; 32])
    .bind(enabled)
    .execute(&pool)
    .await?;

    // Apply the right migration.
    let sql = MIGRATOR
        .iter()
        .find(|migration| migration.version == 24)
        .expect("the platform admin right migration is present")
        .sql
        .as_str()
        .to_owned();
    sqlx::raw_sql(AssertSqlSafe(sql)).execute(&pool).await?;

    let (enabled_flag,): (bool,) = sqlx::query_as("SELECT platform_admin FROM users WHERE id = $1")
        .bind(enabled)
        .fetch_one(&pool)
        .await?;
    assert!(enabled_flag, "the enabled membership gives the right");
    let (revoked_flag,): (bool,) = sqlx::query_as("SELECT platform_admin FROM users WHERE id = $1")
        .bind(revoked)
        .fetch_one(&pool)
        .await?;
    assert!(!revoked_flag, "the disabled membership gives no right");

    let (memberships,): (i64,) = sqlx::query_as("SELECT count(*) FROM memberships")
        .fetch_one(&pool)
        .await?;
    assert_eq!(memberships, 0, "the platform_admin memberships are gone");

    let (live,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM api_tokens WHERE revoked_at IS NULL")
            .fetch_one(&pool)
            .await?;
    assert_eq!(live, 0, "the platform token is revoked");
    let (kept,): (i64,) = sqlx::query_as("SELECT count(*) FROM api_tokens")
        .fetch_one(&pool)
        .await?;
    assert_eq!(kept, 1, "the token row stays");

    pool.close().await;
    admin_query(
        &admin,
        &format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"),
    )
    .await?;
    admin.close().await;
    db.cleanup().await;
    Ok(())
}

/// ADR-0040: the migration writes one audit record for each profile that held a hook, before it
/// drops the column.
#[tokio::test]
async fn db_the_hook_removal_migration_audits_each_profile_with_hooks() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let admin = admin_pool(&db).await?;
    let name = format!("restichub_mig_{}", uuid::Uuid::now_v7().simple());
    admin_query(&admin, &format!("CREATE DATABASE \"{name}\"")).await?;

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(db.admin_options().database(name.as_str()))
        .await?;

    let before: Vec<_> = MIGRATOR
        .iter()
        .filter(|migration| migration.version <= 43)
        .cloned()
        .collect();
    let up_to_43 = Migrator {
        migrations: Cow::Owned(before),
        ignore_missing: false,
        locking: true,
        no_tx: false,
        table_name: Cow::Borrowed("_sqlx_migrations"),
        create_schemas: Cow::Owned(Vec::new()),
    };
    up_to_43.run(&pool).await?;

    let tenant = uuid::Uuid::now_v7();
    let with_hooks = uuid::Uuid::now_v7();
    let without_hooks = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO tenants (id, tenant_id, name) VALUES ($1, $1, 'hooks')")
        .bind(tenant)
        .execute(&pool)
        .await?;
    // The repository is not part of this check, so the foreign keys stay off for the inserts.
    sqlx::query("SET session_replication_role = replica")
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO profiles (id, tenant_id, name, repository_id, hooks) VALUES \
         ($1, $3, 'with', $4, '[{\"hook_id\": \"pg-dump\", \"phase\": \"before\", \
         \"parameters\": [{\"name\": \"password\", \"value\": \"s3cret\"}]}]'::jsonb), \
         ($2, $3, 'without', $4, '[]'::jsonb)",
    )
    .bind(with_hooks)
    .bind(without_hooks)
    .bind(tenant)
    .bind(uuid::Uuid::now_v7())
    .execute(&pool)
    .await?;
    sqlx::query("SET session_replication_role = DEFAULT")
        .execute(&pool)
        .await?;

    let sql = MIGRATOR
        .iter()
        .find(|migration| migration.version == 44)
        .expect("the hook removal migration is present")
        .sql
        .as_str()
        .to_owned();
    sqlx::raw_sql(AssertSqlSafe(sql)).execute(&pool).await?;

    let rows: Vec<(uuid::Uuid, uuid::Uuid, String, String, serde_json::Value)> = sqlx::query_as(
        "SELECT object_id, tenant_id, actor_type, action, diff FROM audit_log \
         WHERE action = 'profile_hooks_removed'",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(rows.len(), 1, "only the profile with a hook gets a record");
    let (object, row_tenant, actor, action, diff) = &rows[0];
    assert_eq!(*object, with_hooks);
    assert_eq!(*row_tenant, tenant);
    assert_eq!(actor, "system");
    assert_eq!(action, "profile_hooks_removed");
    assert_eq!(diff["hooks"][0]["hook_id"], "pg-dump");
    assert_eq!(diff["hooks"][0]["phase"], "before");
    assert!(
        !diff.to_string().contains("s3cret"),
        "the record holds no parameter value"
    );

    pool.close().await;
    admin_query(
        &admin,
        &format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"),
    )
    .await?;
    admin.close().await;
    db.cleanup().await;
    Ok(())
}
