//! Audit log tests: the append-only rule, the platform scope and the filters.

use super::*;
use crate::PLATFORM_TENANT_ID;
use crate::audit;
use crate::audit::{AuditCursor, AuditFilter, AuditPage};
use crate::tenants;
use crate::test_util::test_database;
use uuid::Uuid;

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
async fn db_insert_platform_for_clears_the_tenant_guc() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    let mut tx = db.database().begin_platform().await?;
    let event = AuditEvent::new(AuditAction::TenantSuspend, "tenant");
    audit::insert_platform_for(&mut tx, tenant_a, &event).await?;

    // The record is in tenant A.
    let page = audit::list_platform(
        &mut tx,
        &AuditFilter {
            action: Some(AuditAction::TenantSuspend),
            ..AuditFilter::default()
        },
        &AuditPage::default(),
    )
    .await?;
    assert_eq!(page.records.len(), 1);

    // The function cleared the GUC: the transaction reads no row of tenant A (issue #20).
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM users")
        .fetch_one(tx.as_conn())
        .await?;
    assert_eq!(count.0, 0, "the transaction must hold no tenant scope");

    // The transaction cannot write a row of tenant A either.
    let write = sqlx::query(
        "INSERT INTO users (id, tenant_id, username, password_hash, display_name) \
         VALUES ($1, $2, 'mallory', 'phc', 'Mallory')",
    )
    .bind(UserId::new())
    .bind(tenant_a)
    .execute(tx.as_conn())
    .await;
    assert_code(write, "42501");
    drop(tx);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_platform_audit_writes_an_existing_tenant_and_counts_users() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;
    seed_tenant(db.database(), tenant_a).await?;

    // Seed the platform tenant, so the visibility check below is meaningful.
    let mut tx = db.database().begin_tenant(PLATFORM_TENANT_ID).await?;
    users::create(
        &mut tx,
        &NewUser {
            username: "root",
            password_hash: "phc",
            display_name: "Root",
        },
    )
    .await?;
    sqlx::query(
        "INSERT INTO kek_check (tenant_id, kek_id, nonce, ciphertext) VALUES ($1, 'kek-1', $2, $3)",
    )
    .bind(PLATFORM_TENANT_ID)
    .bind(vec![1u8; 12])
    .bind(vec![2u8; 32])
    .execute(tx.as_conn())
    .await?;
    sqlx::query(
        "INSERT INTO setup_tokens (id, tenant_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, now() + interval '1 hour')",
    )
    .bind(SetupTokenId::new())
    .bind(PLATFORM_TENANT_ID)
    .bind(vec![3u8; 32])
    .execute(tx.as_conn())
    .await?;
    tx.commit().await?;

    // The user count function returns the count in a platform transaction.
    let mut tx = db.database().begin_platform().await?;
    let summary = tenants::get_platform(&mut tx, tenant_a).await?;
    tx.commit().await?;
    assert_eq!(summary.map(|tenant| tenant.user_count), Some(1));

    // The count function returns no row in a tenant transaction (issue #20).
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let rows: Vec<(Uuid, i64)> =
        sqlx::query_as("SELECT tenant_id, user_count FROM tenant_user_counts()")
            .fetch_all(tx.as_conn())
            .await?;
    tx.commit().await?;
    assert!(
        rows.is_empty(),
        "a tenant transaction must get no count row"
    );

    // A platform operation writes the audit record of an existing tenant and keeps no tenant scope.
    let mut tx = db.database().begin_platform().await?;
    let event = AuditEvent::new(AuditAction::TenantSuspend, "tenant");
    let transition =
        tenants::set_status_from(&mut tx, tenant_a, "active", "suspended", &event, &event).await?;
    assert_eq!(transition, tenants::Transition::Changed);

    // After the audit write, the transaction reads and writes no row of tenant A, and it sees no
    // row of the platform tenant (issue #20).
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM users")
        .fetch_one(tx.as_conn())
        .await?;
    assert_eq!(count.0, 0, "the transaction must hold no tenant scope");
    let platform_users: (i64,) = sqlx::query_as("SELECT count(*) FROM users")
        .fetch_one(tx.as_conn())
        .await?;
    assert_eq!(platform_users.0, 0);
    let platform_keys: (i64,) = sqlx::query_as("SELECT count(*) FROM kek_check")
        .fetch_one(tx.as_conn())
        .await?;
    assert_eq!(platform_keys.0, 0);
    let platform_setup: (i64,) = sqlx::query_as("SELECT count(*) FROM setup_tokens")
        .fetch_one(tx.as_conn())
        .await?;
    assert_eq!(platform_setup.0, 0);
    tx.commit().await?;

    // The audit record is in the affected tenant.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let records = audit::list(
        &mut tx,
        &AuditFilter {
            action: Some(AuditAction::TenantSuspend),
            ..AuditFilter::default()
        },
        &AuditPage::default(),
    )
    .await?
    .records;
    tx.commit().await?;
    assert_eq!(records.len(), 1);

    // A repeated call is idempotent, and a wrong source status is refused.
    let mut tx = db.database().begin_platform().await?;
    let transition =
        tenants::set_status_from(&mut tx, tenant_a, "active", "suspended", &event, &event).await?;
    assert_eq!(transition, tenants::Transition::AlreadyInTarget);
    let transition =
        tenants::set_status_from(&mut tx, tenant_a, "deleted", "active", &event, &event).await?;
    assert_eq!(transition, tenants::Transition::Refused);
    tx.commit().await?;

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
        after = records.last().map(|record| AuditCursor {
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
