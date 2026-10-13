use super::*;
use crate::PLATFORM_TENANT_ID;
use crate::test_util::test_database;

#[tokio::test]
async fn ac_tn_01_db_tenant_a_cannot_read_tenant_b_rows() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_b).await?;

    let mut tx_b = db.database().begin_tenant(tenant_b).await?;
    for table in TENANT_TABLES {
        if table == "users" {
            continue;
        }
        assert_eq!(count_rows(&mut tx_b, table).await?, 1, "tenant B: {table}");
    }
    tx_b.commit().await?;
    // An account lives in the platform tenant, and the runtime role reads no account row directly
    // (ADR-0025).
    assert_tenant_table_read_refused(db.database(), tenant_b, "users").await?;

    let mut tx_a = db.database().begin_tenant(tenant_a).await?;
    for table in TENANT_TABLES {
        if table == "users" {
            continue;
        }
        assert_eq!(count_rows(&mut tx_a, table).await?, 0, "tenant A: {table}");
    }
    tx_a.commit().await?;
    assert_tenant_table_read_refused(db.database(), tenant_a, "users").await?;

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

    // The runtime role cannot write an account row directly (ADR-0025): the account table is
    // reached through the definer functions only. Each negative write runs in its own transaction,
    // so an aborted transaction does not hide the next result.
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

    // A tenant transaction cannot add an account to a tenant: a membership insert runs through the
    // definer functions only (ADR-0025).
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let reference =
        sqlx::query("INSERT INTO memberships (tenant_id, user_id, role) VALUES ($1, $2, 'viewer')")
            .bind(tenant_a)
            .bind(seed_b.user)
            .execute(tx.as_conn())
            .await;
    assert_code(reference, "42501");
    drop(tx);

    // The runtime role changes the role and the disabled flag of a membership only: it cannot move
    // the membership to another account (42501, ADR-0025).
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let moved = sqlx::query("UPDATE memberships SET user_id = $1")
        .bind(seed_b.user)
        .execute(tx.as_conn())
        .await;
    assert_code(moved, "42501");
    drop(tx);

    // The role update of the own membership still works.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let role = sqlx::query("UPDATE memberships SET role = 'viewer' WHERE user_id = $1")
        .bind(seed_a.user)
        .execute(tx.as_conn())
        .await?;
    assert_eq!(role.rows_affected(), 1);
    tx.commit().await?;

    // The database function refuses the foreign account to a tenant scope too.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let (added,): (bool,) = sqlx::query_as("SELECT auth_add_membership($1, $2, $3)")
        .bind(seed_b.user)
        .bind(tenant_a)
        .bind("viewer")
        .fetch_one(tx.as_conn())
        .await?;
    assert!(!added, "a tenant scope cannot add a membership");
    drop(tx);

    // A platform transaction cannot read a password hash (ADR-0025).
    let mut tx = db.database().begin_platform().await?;
    let hash = sqlx::query("SELECT password_hash FROM users")
        .execute(tx.as_conn())
        .await;
    assert_code(hash, "42501");
    drop(tx);

    // A session of tenant A may reference the same installation-wide account.
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
    assert!(session_ref.is_ok(), "{session_ref:?}");
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
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM sessions")
        .fetch_one(&mut *tx)
        .await?;
    drop(tx);

    assert_eq!(count.0, 0);
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

    // A tenant-scoped row cannot move to another tenant: the `WITH CHECK` clause of the policy
    // rejects the new tenant id (42501).
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let move_row = sqlx::query("UPDATE sessions SET tenant_id = $1")
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
async fn db_the_platform_scope_sees_no_other_tenant_rows() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (_, tenant_b) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_b).await?;

    let mut tx = db.database().begin_platform().await?;
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM sessions")
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
                    // `information_schema` hides a column the runtime role cannot read, so the
                    // PostgreSQL catalog answers instead.
                    "SELECT count(*) \
                     FROM pg_attribute a \
                     JOIN pg_class c ON c.oid = a.attrelid \
                     JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE n.nspname = $1 AND c.relname = $2 \
                       AND a.attname = 'tenant_id' AND a.attnum > 0 AND NOT a.attisdropped",
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

    // Every SECURITY DEFINER function is in the allow-list and sets the search path.
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

/// Each account function carries its own scope at the database level (ADR-0025). A tenant A
/// transaction cannot reach an account that is a member only of tenant B, nor a Platform Admin
/// account.
#[tokio::test]
async fn db_the_account_functions_are_scoped_to_the_current_tenant() -> TestResult {
    use crate::enums::Role;
    use crate::{auth, users};

    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;

    let local = {
        let mut tx = db.database().begin_tenant(tenant_a).await?;
        let user = users::create_member(
            &mut tx,
            &NewUser {
                username: "local",
                password_hash: "phc",
                display_name: "Local",
            },
            Role::Admin,
        )
        .await?;
        tx.commit().await?;
        user
    };
    let foreign = {
        let mut tx = db.database().begin_tenant(tenant_b).await?;
        let user = users::create_member(
            &mut tx,
            &NewUser {
                username: "foreign",
                password_hash: "phc",
                display_name: "Foreign",
            },
            Role::Viewer,
        )
        .await?;
        tx.commit().await?;
        user
    };
    let platform = {
        let mut tx = db.database().begin_tenant(PLATFORM_TENANT_ID).await?;
        let user = users::create_member(
            &mut tx,
            &NewUser {
                username: "root",
                password_hash: "phc",
                display_name: "Root",
            },
            Role::PlatformAdmin,
        )
        .await?;
        tx.commit().await?;
        user
    };

    let mut tx = db.database().begin_tenant(tenant_a).await?;

    // The member reads return only the members of tenant A.
    assert!(users::get_member(&mut tx, foreign).await?.is_none());
    let members = users::list_members(&mut tx).await?;
    assert!(members.iter().any(|entry| entry.id == local));
    assert!(members.iter().all(|entry| entry.id != foreign));

    // The administrator writes need a membership in the current tenant.
    assert_eq!(
        users::admin_set_password(&mut tx, foreign, "phc").await?,
        users::WriteStatus::NotFound
    );
    assert_eq!(
        users::admin_set_display_name(&mut tx, foreign, "F").await?,
        users::WriteStatus::NotFound
    );
    assert_eq!(
        users::admin_set_password(&mut tx, platform, "phc").await?,
        users::WriteStatus::NotFound
    );

    // The own password change is bound to the caller's own session.
    assert!(!users::self_set_password(&mut tx, foreign, SessionId::new(), "phc").await?);
    assert!(
        users::self_password_hash(&mut tx, foreign, SessionId::new())
            .await?
            .is_none()
    );

    // Only the platform tenant disables the account itself.
    assert_eq!(
        users::set_disabled(&mut tx, foreign, true).await?,
        users::WriteStatus::Forbidden
    );

    // The last used tenant is written for the caller's own session only.
    assert!(!users::set_last_tenant(&mut tx, foreign, SessionId::new(), tenant_a).await?);

    // The session removal reaches the current tenant only.
    assert_eq!(
        auth::delete_account_sessions(&mut tx, foreign, None).await?,
        0
    );

    // The membership insert needs the platform scope: the same call from a tenant transaction is
    // refused by the database function.
    let (added,): (bool,) = sqlx::query_as("SELECT auth_add_membership($1, $2, $3)")
        .bind(foreign)
        .bind(tenant_a)
        .bind("viewer")
        .fetch_one(tx.as_conn())
        .await?;
    assert!(!added);

    tx.commit().await?;

    // A tenant transaction reads no account row directly (ADR-0025).
    assert_tenant_table_read_refused(db.database(), tenant_a, "users").await?;

    db.cleanup().await;
    Ok(())
}

/// A call with no tenant and no scope setting fails closed (ADR-0025). A `NULL` setting must not
/// skip a scope guard.
#[tokio::test]
async fn db_an_account_function_without_the_settings_fails_closed() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    let seed = seed_tenant(db.database(), tenant_a).await?;

    // The runtime login without `SET ROLE` and without the settings.
    let pool = db.pool();
    let (added,): (bool,) = sqlx::query_as("SELECT auth_add_membership($1, $2, $3)")
        .bind(seed.user)
        .bind(tenant_a)
        .bind("viewer")
        .fetch_one(pool)
        .await?;
    assert!(!added, "an unset scope must refuse the membership");

    let (status,): (String,) = sqlx::query_as("SELECT auth_set_account_disabled($1, true)")
        .bind(seed.user)
        .fetch_one(pool)
        .await?;
    assert_eq!(status, "forbidden");

    let (created,): (bool,) = sqlx::query_as("SELECT auth_create_member($1, $2, $3, $4, $5, $6)")
        .bind(UserId::new())
        .bind("no-guc")
        .bind("phc")
        .bind("No Guc")
        .bind(tenant_a)
        .bind("viewer")
        .fetch_one(pool)
        .await?;
    assert!(!created, "an unset tenant must refuse the create");

    db.cleanup().await;
    Ok(())
}
