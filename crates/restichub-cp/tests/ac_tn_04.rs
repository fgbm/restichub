//! AC-TN-04: an API token of tenant A reaches no object of tenant B (FR-TN-05).
//!
//! The first test drives the API with a token of tenant A and checks the tenant B objects and the
//! audit records. The second test checks the second isolation layer below the application: Row-Level
//! Security on every table that holds `tenant_id` (ADR-0006, ADR-0023).

#![allow(clippy::expect_used)]

mod common;

use axum::http::StatusCode;
use restichub_db::audit;
use serde_json::Value;

use common::{app, create_tenant, create_user, seed_api_token, send};

#[tokio::test]
async fn ac_tn_04_api_token_of_tenant_a_cannot_access_tenant_b() {
    let Some(db) = common::test_db().await else {
        return;
    };
    let tenant_a = create_tenant(db.database(), "tenant-a").await;
    let tenant_b = create_tenant(db.database(), "tenant-b").await;

    // Seed a user and an audit record of tenant B.
    let user_b = create_user(db.database(), tenant_b, "bob", "phc", "admin").await;
    let mut tx = db
        .database()
        .begin_tenant(tenant_b)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut event = audit::AuditEvent::new(audit::AuditAction::UserCreate, "user");
    event.object_id = Some(user_b.as_uuid());
    audit::insert(&mut tx, &event)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));

    // Seed a user and an audit record of tenant A.
    let user_a = create_user(db.database(), tenant_a, "alice", "phc", "admin").await;
    let mut tx = db
        .database()
        .begin_tenant(tenant_a)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut event = audit::AuditEvent::new(audit::AuditAction::UserCreate, "user");
    event.object_id = Some(user_a.as_uuid());
    audit::insert(&mut tx, &event)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    tx.commit().await.unwrap_or_else(|e| panic!("{e}"));

    let token_a = seed_api_token(db.database(), tenant_a, "admin").await;
    let app = app(db.database());

    // The token of tenant A cannot read a user of tenant B (HTTP 404).
    let (status, _, _) = send(
        &app,
        common::bearer_request(
            "GET",
            &format!("/api/v1/users/{}", user_b.as_uuid()),
            &token_a,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // The audit list of tenant A holds no event of tenant B.
    let (status, _, page) = send(
        &app,
        common::bearer_request("GET", "/api/v1/audit", &token_a, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no items"));
    assert!(!items.is_empty(), "tenant A must see its own event");
    for item in items {
        assert_eq!(
            item.get("tenant_id").and_then(Value::as_str),
            Some(tenant_a.to_string().as_str()),
            "the audit list of tenant A holds an event of another tenant"
        );
    }

    // The token of tenant A cannot read a user of tenant B through the list either.
    let (status, _, page) = send(
        &app,
        common::bearer_request("GET", "/api/v1/users", &token_a, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let items = page
        .get("items")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no items"));
    for item in items {
        assert_ne!(
            item.get("id").and_then(Value::as_str),
            Some(user_b.to_string().as_str())
        );
    }

    db.cleanup().await;
}

/// The migration table is the only public table without `tenant_id` (ADR-0006).
const NON_TENANT_TABLE: &str = "_sqlx_migrations";

#[tokio::test]
async fn ac_tn_04_every_tenant_table_forces_row_level_security() {
    let Some(db) = common::test_db().await else {
        return;
    };

    // Every public table except the migration bookkeeping holds `tenant_id`.
    let non_tenant: Vec<(String,)> = sqlx::query_as(
        "SELECT c.relname FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'public' AND c.relkind = 'r' \
           AND NOT EXISTS ( \
               SELECT 1 FROM information_schema.columns col \
               WHERE col.table_schema = 'public' AND col.table_name = c.relname \
                 AND col.column_name = 'tenant_id') \
         ORDER BY c.relname",
    )
    .fetch_all(db.owner_pool())
    .await
    .expect("cannot read the catalog");
    let non_tenant: Vec<String> = non_tenant.into_iter().map(|(name,)| name).collect();
    assert_eq!(
        non_tenant,
        vec![NON_TENANT_TABLE.to_owned()],
        "a public table without tenant_id (ADR-0006)"
    );

    // Every table with `tenant_id` enables and forces Row-Level Security and has a policy that
    // reads the tenant from the setting `restichub.tenant_id`.
    let rows: Vec<(String, bool, bool, bool)> = sqlx::query_as(
        "SELECT c.relname, c.relrowsecurity, c.relforcerowsecurity, \
                coalesce(bool_or( \
                    coalesce(pg_get_expr(p.polqual, p.polrelid), '') LIKE '%restichub.tenant_id%' \
                    OR coalesce(pg_get_expr(p.polwithcheck, p.polrelid), '') \
                       LIKE '%restichub.tenant_id%'), false) \
         FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         LEFT JOIN pg_policy p ON p.polrelid = c.oid \
         WHERE n.nspname = 'public' AND c.relkind = 'r' \
           AND EXISTS ( \
               SELECT 1 FROM information_schema.columns col \
               WHERE col.table_schema = 'public' AND col.table_name = c.relname \
                 AND col.column_name = 'tenant_id') \
         GROUP BY c.relname, c.relrowsecurity, c.relforcerowsecurity \
         ORDER BY c.relname",
    )
    .fetch_all(db.owner_pool())
    .await
    .expect("cannot read the catalog");

    assert!(!rows.is_empty(), "no table with tenant_id is present");
    for (table, enabled, forced, has_policy) in &rows {
        assert!(
            *enabled,
            "{table}: ROW LEVEL SECURITY is not enabled (ADR-0006)"
        );
        assert!(
            *forced,
            "{table}: FORCE ROW LEVEL SECURITY is not enabled (ADR-0023)"
        );
        assert!(
            *has_policy,
            "{table}: no policy reads restichub.tenant_id (ADR-0023)"
        );
    }

    db.cleanup().await;
}
