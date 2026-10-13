use super::*;
use crate::PLATFORM_TENANT_ID;
use crate::enums::TenantStatus;
use crate::tenants;
use crate::test_util::test_database;
use uuid::Uuid;

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
async fn db_a_tenant_creation_commits_atomically() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (_, _) = two_tenants(db.database()).await?;

    // A created tenant, its key row, its first admin and its audit record commit together.
    let mut tx = db.database().begin_platform().await?;
    let committed = tenants::create_with_bootstrap(&mut tx, &bootstrap("committed")).await?;
    tx.commit().await?;
    assert!(tenant_exists(db.database(), committed).await?);
    assert_eq!(
        count_in_tenant(db.database(), committed, "tenant_keys").await?,
        1
    );
    // The account lives in the platform tenant, and the runtime role reads no account row directly
    // (ADR-0025).
    assert_tenant_table_read_refused(db.database(), committed, "users").await?;
    assert_eq!(
        count_in_tenant(db.database(), committed, "memberships").await?,
        1
    );
    assert_eq!(
        count_in_tenant(db.database(), committed, "audit_log").await?,
        1
    );

    // Dropping the transaction leaves nothing.
    let mut tx = db.database().begin_platform().await?;
    let crashed = tenants::create_with_bootstrap(&mut tx, &bootstrap("crashed")).await?;
    drop(tx);
    assert!(!tenant_exists(db.database(), crashed).await?);

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_a_tenant_creation_rolls_back_on_a_bad_audit_actor() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (_, _) = two_tenants(db.database()).await?;

    // A failure inside `create_with_bootstrap`: the audit insert rejects an unknown actor type
    // (23514). The tenant row, the key row, the first admin and the audit records must not persist.
    let mut input = bootstrap("bad-audit");
    input.actor = Actor::Unknown {
        actor_type: "bogus".to_owned(),
        actor_id: None,
    };
    let tenant = input.id;
    let mut tx = db.database().begin_platform().await?;
    let result = tenants::create_with_bootstrap(&mut tx, &input).await;
    assert!(
        result
            .as_ref()
            .is_err_and(|error| error.is_check_violation()),
        "{result:?}"
    );
    drop(tx);

    assert!(!tenant_exists(db.database(), tenant).await?);
    assert_eq!(
        count_in_tenant(db.database(), tenant, "tenant_keys").await?,
        0
    );
    assert_tenant_table_read_refused(db.database(), tenant, "users").await?;
    assert_eq!(
        count_in_tenant(db.database(), tenant, "audit_log").await?,
        0
    );

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_a_concurrent_delete_and_resume_never_end_active() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, _) = two_tenants(db.database()).await?;

    let mut tx = db.database().begin_platform().await?;
    let suspend = AuditEvent::new(AuditAction::TenantSuspend, "tenant");
    assert_eq!(
        tenants::set_status_from(
            &mut tx,
            tenant_a,
            TenantStatus::Active,
            TenantStatus::Suspended,
            &suspend,
            &suspend,
        )
        .await?,
        tenants::Transition::Changed
    );
    tx.commit().await?;

    // A delete (suspended -> deleted) races a resume (suspended -> active). The update of the
    // loser touches no row.
    let database = db.database();
    let delete = async {
        let mut tx = database.begin_platform().await?;
        let event = AuditEvent::new(AuditAction::TenantDelete, "tenant");
        let result = tenants::set_status_from(
            &mut tx,
            tenant_a,
            TenantStatus::Suspended,
            TenantStatus::Deleted,
            &event,
            &event,
        )
        .await?;
        tx.commit().await?;
        Ok::<_, crate::DbError>(result)
    };
    let resume = async {
        let mut tx = database.begin_platform().await?;
        let event = AuditEvent::new(AuditAction::TenantResume, "tenant");
        let result = tenants::set_status_from(
            &mut tx,
            tenant_a,
            TenantStatus::Suspended,
            TenantStatus::Active,
            &event,
            &event,
        )
        .await?;
        tx.commit().await?;
        Ok::<_, crate::DbError>(result)
    };
    let (delete_result, resume_result) = tokio::join!(delete, resume);
    let delete_result = delete_result?;
    let resume_result = resume_result?;

    // Exactly one transition changes the status. The tenant never ends active after a delete.
    assert_ne!(
        delete_result == tenants::Transition::Changed,
        resume_result == tenants::Transition::Changed,
        "exactly one transition must change the status"
    );
    let mut tx = db.database().begin_platform().await?;
    let status = tenants::get_platform(&mut tx, tenant_a)
        .await?
        .map(|tenant| tenant.status)
        .unwrap_or(TenantStatus::Active);
    tx.commit().await?;
    if delete_result == tenants::Transition::Changed {
        assert_eq!(
            status,
            TenantStatus::Deleted,
            "the tenant must not end active after a delete"
        );
    } else {
        assert_eq!(status, TenantStatus::Active);
    }

    db.cleanup().await;
    Ok(())
}

#[tokio::test]
async fn db_enter_tenant_refuses_a_foreign_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (_, tenant_b) = two_tenants(db.database()).await?;

    // The tenant exists, but this transaction did not create it.
    let mut tx = db.database().begin_platform().await?;
    assert!(tx.enter_tenant(tenant_b).await.is_err());
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

    // The tenant name is unique and ignores the letter case.
    let mut tx = db.database().begin_platform().await?;
    let duplicate_tenant = tenants::create(&mut tx, "TENANT-A").await;
    assert!(duplicate_tenant.is_err(), "the name must be unique");
    drop(tx);

    // The username is unique for the tenant and ignores the letter case.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    users::create_member(
        &mut tx,
        &NewUser {
            username: "Alice",
            password_hash: "phc",
            display_name: "Alice",
        },
        crate::Role::Admin,
    )
    .await?;
    let duplicate_user = users::create_member(
        &mut tx,
        &NewUser {
            username: "alice",
            password_hash: "phc",
            display_name: "Alice",
        },
        crate::Role::Admin,
    )
    .await;
    assert!(duplicate_user.is_err(), "the username must be unique");
    drop(tx);

    db.cleanup().await;
    Ok(())
}
