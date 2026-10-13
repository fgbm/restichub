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
