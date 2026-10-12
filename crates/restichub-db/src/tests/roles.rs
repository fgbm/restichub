//! Runtime role tests: the safety check, the pooled connection reset and escalation.

use super::*;
use crate::DatabaseConfig;
use crate::test_util::test_database;
use uuid::Uuid;

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
    let config = DatabaseConfig {
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
