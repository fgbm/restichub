use std::borrow::Cow;

use sqlx::AssertSqlSafe;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;

use super::{TestResult, admin_pool, admin_query, assert_code};
use crate::MIGRATOR;
use crate::test_util::test_database;

#[tokio::test]
async fn ac_nfr_15_verify_migrations_detects_a_newer_schema() -> TestResult {
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

/// ADR-0047: the migration 0056 moves the repository from the profile to the plan.
///
/// Each plan keeps the repository of its former profile in every tenant, the profile loses the
/// column, and the new key refuses a repository of another tenant. The migration runs as the owner
/// login of the installation, which owns the tables and has no BYPASSRLS (ADR-0023).
#[tokio::test]
async fn db_the_plan_repository_migration_keeps_the_repository_of_each_plan() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let admin = admin_pool(&db).await?;
    let name = format!("restichub_mig_{}", uuid::Uuid::now_v7().simple());
    admin_query(
        &admin,
        &format!("CREATE DATABASE \"{name}\" OWNER \"{}\"", db.owner_role()),
    )
    .await?;

    // The tables belong to the owner login, so the migrations run as that login, as `migrate` does.
    let owner = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(db.owner_options().database(name.as_str()))
        .await?;

    let before: Vec<_> = MIGRATOR
        .iter()
        .filter(|migration| migration.version <= 55)
        .cloned()
        .collect();
    let up_to_55 = Migrator {
        migrations: Cow::Owned(before),
        ignore_missing: false,
        locking: true,
        no_tx: false,
        table_name: Cow::Borrowed("_sqlx_migrations"),
        create_schemas: Cow::Owned(Vec::new()),
    };
    up_to_55.run(&owner).await?;

    // The seeded rows hold only the columns of this check, so the foreign keys stay off.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(db.admin_options().database(name.as_str()))
        .await?;
    let tenant = uuid::Uuid::now_v7();
    let other_tenant = uuid::Uuid::now_v7();
    let first_repository = uuid::Uuid::now_v7();
    let second_repository = uuid::Uuid::now_v7();
    let other_repository = uuid::Uuid::now_v7();
    let first_profile = uuid::Uuid::now_v7();
    let second_profile = uuid::Uuid::now_v7();
    let other_profile = uuid::Uuid::now_v7();
    let first_plan = uuid::Uuid::now_v7();
    let second_plan = uuid::Uuid::now_v7();
    let other_plan = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tenants (id, tenant_id, name) VALUES ($1, $1, 'plans'), ($2, $2, 'other')",
    )
    .bind(tenant)
    .bind(other_tenant)
    .execute(&pool)
    .await?;
    sqlx::query("SET session_replication_role = replica")
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO repositories (id, tenant_id, name, backend, local_path, password_secret_id) \
         VALUES ($1, $4, 'first', 'local', '/srv/first', $6), \
                ($2, $4, 'second', 'local', '/srv/second', $6), \
                ($3, $5, 'other', 'local', '/srv/other', $6)",
    )
    .bind(first_repository)
    .bind(second_repository)
    .bind(other_repository)
    .bind(tenant)
    .bind(other_tenant)
    .bind(uuid::Uuid::now_v7())
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO profiles (id, tenant_id, name, repository_id) \
         VALUES ($1, $4, 'first', $6), ($2, $4, 'second', $7), ($3, $5, 'other', $8)",
    )
    .bind(first_profile)
    .bind(second_profile)
    .bind(other_profile)
    .bind(tenant)
    .bind(other_tenant)
    .bind(first_repository)
    .bind(second_repository)
    .bind(other_repository)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO plans \
         (id, tenant_id, name, profile_id, agent_id, cron, time_zone, missed_run_policy) \
         VALUES ($1, $4, 'a-first', $6, $9, '0 2 * * *', 'UTC', 'run_all'), \
                ($2, $4, 'b-second', $7, $9, '0 3 * * *', 'UTC', 'run_all'), \
                ($3, $5, 'c-other', $8, $9, '0 4 * * *', 'UTC', 'run_all')",
    )
    .bind(first_plan)
    .bind(second_plan)
    .bind(other_plan)
    .bind(tenant)
    .bind(other_tenant)
    .bind(first_profile)
    .bind(second_profile)
    .bind(other_profile)
    .bind(uuid::Uuid::now_v7())
    .execute(&pool)
    .await?;
    sqlx::query("SET session_replication_role = DEFAULT")
        .execute(&pool)
        .await?;

    let sql = MIGRATOR
        .iter()
        .find(|migration| migration.version == 56)
        .expect("the plan repository migration is present")
        .sql
        .as_str()
        .to_owned();
    sqlx::raw_sql(AssertSqlSafe(sql)).execute(&owner).await?;

    let rows: Vec<(uuid::Uuid, uuid::Uuid)> =
        sqlx::query_as("SELECT id, repository_id FROM plans ORDER BY name")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        rows,
        vec![
            (first_plan, first_repository),
            (second_plan, second_repository),
            (other_plan, other_repository),
        ],
        "each plan keeps the repository of its former profile"
    );

    let column: Option<(String,)> = sqlx::query_as(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_name = 'profiles' AND column_name = 'repository_id'",
    )
    .fetch_optional(&pool)
    .await?;
    assert!(column.is_none(), "the profile keeps no repository");

    // The new key refuses a repository of another tenant.
    let foreign = sqlx::query("UPDATE plans SET repository_id = $2 WHERE id = $1")
        .bind(first_plan)
        .bind(other_repository)
        .execute(&pool)
        .await;
    assert_code(foreign, "23503");

    pool.close().await;
    owner.close().await;
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
        .filter(|migration| migration.version <= 44)
        .cloned()
        .collect();
    let up_to_44 = Migrator {
        migrations: Cow::Owned(before),
        ignore_missing: false,
        locking: true,
        no_tx: false,
        table_name: Cow::Borrowed("_sqlx_migrations"),
        create_schemas: Cow::Owned(Vec::new()),
    };
    up_to_44.run(&pool).await?;

    let tenant = uuid::Uuid::now_v7();
    let with_hooks = uuid::Uuid::now_v7();
    let without_hooks = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO tenants (id, tenant_id, name) VALUES ($1, $1, 'hooks')")
        .bind(tenant)
        .execute(&pool)
        .await?;
    // The repository and the tenant are not part of this check, so the foreign keys stay off for
    // the inserts. The profile of this version still holds its repository (ADR-0047).
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
        .find(|migration| migration.version == 45)
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

/// The migration 0046 writes a `secret_ref` value in the canonical form and leaves every other
/// value as it is.
#[tokio::test]
async fn db_the_canonical_secret_ref_migration_rewrites_the_references() -> TestResult {
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

    let before: Vec<_> = MIGRATOR
        .iter()
        .filter(|migration| migration.version < 46)
        .cloned()
        .collect();
    Migrator {
        migrations: Cow::Owned(before),
        ignore_missing: false,
        locking: true,
        no_tx: false,
        table_name: Cow::Borrowed("_sqlx_migrations"),
        create_schemas: Cow::Owned(Vec::new()),
    }
    .run(&pool)
    .await?;

    let tenant = "00000000-0000-7000-8000-000000000000";
    let upper = "550E8400-E29B-41D4-A716-446655440000";
    let canonical = "550e8400-e29b-41d4-a716-446655440000";
    let scripts = serde_json::json!([{
        "name": "dump",
        "body": format!("echo {upper}"),
        "env": [
            {"name": "A", "kind": "secret_ref", "value": upper},
            {"name": "B", "kind": "secret_ref", "value": "{550e8400e29b41d4a716446655440000}"},
            {"name": "C", "kind": "string", "value": upper},
            {"name": "D", "kind": "secret_ref", "value": ""},
        ],
    }]);
    sqlx::query(
        "INSERT INTO job_templates (id, tenant_id, name, scripts) VALUES ($1, $2, 'template', $3)",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(tenant.parse::<uuid::Uuid>()?)
    .bind(&scripts)
    .execute(&pool)
    .await?;

    let sql = MIGRATOR
        .iter()
        .find(|migration| migration.version == 46)
        .expect("the canonical reference migration is present")
        .sql
        .as_str()
        .to_owned();
    sqlx::raw_sql(AssertSqlSafe(sql)).execute(&pool).await?;

    let (saved,): (serde_json::Value,) = sqlx::query_as("SELECT scripts FROM job_templates")
        .fetch_one(&pool)
        .await?;
    let env = &saved[0]["env"];
    assert_eq!(env[0]["value"], canonical);
    assert_eq!(env[1]["value"], canonical);
    assert_eq!(env[2]["value"], upper);
    assert_eq!(env[3]["value"], "");
    assert_eq!(saved[0]["body"], format!("echo {upper}"));

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

/// The migration 0050 gives a delivery of an earlier version the time of its last attempt, so the
/// channel list shows it after the upgrade.
#[tokio::test]
async fn db_the_list_fields_migration_backfills_the_last_delivery_attempt() -> TestResult {
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

    let before: Vec<_> = MIGRATOR
        .iter()
        .filter(|migration| migration.version < 50)
        .cloned()
        .collect();
    Migrator {
        migrations: Cow::Owned(before),
        ignore_missing: false,
        locking: true,
        no_tx: false,
        table_name: Cow::Borrowed("_sqlx_migrations"),
        create_schemas: Cow::Owned(Vec::new()),
    }
    .run(&pool)
    .await?;

    let tenant: uuid::Uuid = "00000000-0000-7000-8000-000000000000".parse()?;
    let channel = uuid::Uuid::now_v7();
    let alert = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO notification_channels (id, tenant_id, name, kind, recipients) \
         VALUES ($1, $2, 'ops', 'email', ARRAY['ops@example.com'])",
    )
    .bind(channel)
    .bind(tenant)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO alert_state (id, tenant_id, kind, dedupe_key, status, opened_at) \
         VALUES ($1, $2, 'job_failed', 'key', 'open', now())",
    )
    .bind(alert)
    .bind(tenant)
    .execute(&pool)
    .await?;
    let deliveries = [
        (
            "sent",
            1,
            Some("2026-10-01T10:05:00Z"),
            "2026-10-01T10:05:00Z",
        ),
        ("failed", 5, None, "2026-10-02T10:00:00Z"),
        ("pending", 2, None, "2026-10-03T10:00:00Z"),
        ("pending", 0, None, "2026-10-04T10:00:00Z"),
    ];
    for (status, attempts, sent_at, created_at) in deliveries {
        sqlx::query(
            "INSERT INTO alert_deliveries \
             (id, tenant_id, alert_id, channel_id, event, status, attempts, next_attempt_at, \
              created_at, sent_at) \
             VALUES ($1, $2, $3, $4, 'opened', $5, $6, now(), $7::timestamptz, \
                     $8::timestamptz)",
        )
        .bind(uuid::Uuid::now_v7())
        .bind(tenant)
        .bind(alert)
        .bind(channel)
        .bind(status)
        .bind(attempts)
        .bind(created_at)
        .bind(sent_at)
        .execute(&pool)
        .await?;
    }

    let sql = MIGRATOR
        .iter()
        .find(|migration| migration.version == 50)
        .expect("the list fields migration is present")
        .sql
        .as_str()
        .to_owned();
    sqlx::raw_sql(AssertSqlSafe(sql)).execute(&pool).await?;

    let rows: Vec<(String, i32, Option<String>)> = sqlx::query_as(
        "SELECT status, attempts, to_char(attempted_at AT TIME ZONE 'UTC', 'MM-DD HH24:MI') \
         FROM alert_deliveries ORDER BY created_at",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        rows,
        [
            ("sent".to_owned(), 1, Some("10-01 10:05".to_owned())),
            ("failed".to_owned(), 5, Some("10-02 10:00".to_owned())),
            ("pending".to_owned(), 2, Some("10-03 10:00".to_owned())),
            ("pending".to_owned(), 0, None),
        ]
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

/// The migration 0052 turns a blank email into NULL, so the unique email index lets any number of
/// accounts have no email.
#[tokio::test]
async fn db_the_blank_email_migration_writes_null() -> TestResult {
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

    let before: Vec<_> = MIGRATOR
        .iter()
        .filter(|migration| migration.version < 52)
        .cloned()
        .collect();
    Migrator {
        migrations: Cow::Owned(before),
        ignore_missing: false,
        locking: true,
        no_tx: false,
        table_name: Cow::Borrowed("_sqlx_migrations"),
        create_schemas: Cow::Owned(Vec::new()),
    }
    .run(&pool)
    .await?;

    sqlx::query(
        "INSERT INTO users (id, tenant_id, username, password_hash, display_name, email) \
         VALUES ($1, '00000000-0000-7000-8000-000000000000', 'blank', 'phc', 'Blank', ' '), \
                ($2, '00000000-0000-7000-8000-000000000000', 'mail', 'phc', 'Mail', 'a@b.example')",
    )
    .bind(uuid::Uuid::now_v7())
    .bind(uuid::Uuid::now_v7())
    .execute(&pool)
    .await?;

    let sql = MIGRATOR
        .iter()
        .find(|migration| migration.version == 52)
        .expect("the blank email migration is present")
        .sql
        .as_str()
        .to_owned();
    sqlx::raw_sql(AssertSqlSafe(sql)).execute(&pool).await?;

    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT username, email FROM users ORDER BY username")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        rows,
        [
            ("blank".to_owned(), None),
            ("mail".to_owned(), Some("a@b.example".to_owned())),
        ]
    );
    let blank = sqlx::query("UPDATE users SET email = '' WHERE username = 'blank'")
        .execute(&pool)
        .await;
    assert!(blank.is_err(), "the check must refuse a blank email");

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
