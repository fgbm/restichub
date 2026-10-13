use super::*;
use crate::test_util::test_database;
use uuid::Uuid;

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

use crate::PLATFORM_TENANT_ID;
use crate::enums::{Role, TenantStatus};
use crate::ids::{SessionId, UserId};
use crate::{auth, memberships, users};

/// Creates an account with a membership in the tenant and returns the account id.
async fn member(
    db: &Database,
    tenant: TenantId,
    name: &str,
    role: Role,
) -> Result<UserId, Box<dyn std::error::Error>> {
    let mut tx = db.begin_tenant(tenant).await?;
    let user = users::create(
        &mut tx,
        &NewUser {
            username: name,
            password_hash: "phc",
            display_name: name,
        },
    )
    .await?;
    memberships::set_role(&mut tx, user, role).await?;
    tx.commit().await?;
    Ok(user)
}

async fn set_status(
    db: &Database,
    tenant: TenantId,
    status: TenantStatus,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut tx = db.begin_platform().await?;
    crate::tenants::set_status(&mut tx, tenant, status).await?;
    tx.commit().await?;
    Ok(())
}

/// A membership in a suspended tenant and a disabled membership still share the account
/// (ADR-0025, threat T14). A membership in a deleted tenant does not.
#[tokio::test]
async fn db_a_shared_account_counts_a_suspended_tenant_and_a_disabled_membership() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    let user = member(db.database(), tenant_a, "shared", Role::Viewer).await?;
    let mut tx = db.database().begin_tenant(tenant_b).await?;
    memberships::set_role(&mut tx, user, Role::Viewer).await?;
    tx.commit().await?;

    // The account is a member of tenant A and tenant B, so it is shared.
    assert!(auth::account_is_shared(db.database(), user, tenant_a).await?);
    set_status(db.database(), tenant_b, TenantStatus::Suspended).await?;
    assert!(auth::account_is_shared(db.database(), user, tenant_a).await?);

    set_status(db.database(), tenant_b, TenantStatus::Active).await?;
    let mut tx = db.database().begin_tenant(tenant_b).await?;
    memberships::set_disabled(&mut tx, user, true).await?;
    tx.commit().await?;
    assert!(auth::account_is_shared(db.database(), user, tenant_a).await?);

    set_status(db.database(), tenant_b, TenantStatus::Deleted).await?;
    assert!(!auth::account_is_shared(db.database(), user, tenant_a).await?);

    db.cleanup().await;
    Ok(())
}

/// The membership insert and the administrator write take the same account row lock, so a
/// membership that commits first makes the write refuse the account (ADR-0025, threat T14).
#[tokio::test]
async fn db_the_membership_insert_takes_the_account_row_lock() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    let user = member(db.database(), tenant_a, "locked", Role::Viewer).await?;

    // Hold the account row lock.
    let mut tx_a = db.database().begin_tenant(tenant_a).await?;
    assert!(users::lock_account(&mut tx_a, user).await?);

    // The platform membership insert waits for the lock.
    let blocked = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        let mut tx = db.database().begin_platform().await?;
        let added = auth::add_membership(&mut tx, user, tenant_b, Role::Viewer).await?;
        tx.commit().await?;
        Ok::<_, crate::DbError>(added)
    })
    .await;
    assert!(
        blocked.is_err(),
        "the membership insert must wait for the account row lock"
    );
    drop(tx_a);

    // The insert commits the membership first, so the administrator write is refused.
    {
        let mut tx = db.database().begin_platform().await?;
        assert!(auth::add_membership(&mut tx, user, tenant_b, Role::Viewer).await?);
        tx.commit().await?;
    }
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    assert_eq!(
        users::admin_set_password(&mut tx, user, "phc").await?,
        users::WriteStatus::Shared
    );
    drop(tx);

    db.cleanup().await;
    Ok(())
}

/// Each account function carries its own scope at the database level (ADR-0025). A tenant A
/// transaction cannot reach an account that is a member only of tenant B, nor a Platform Admin
/// account.
#[tokio::test]
async fn db_the_account_functions_are_scoped_to_the_current_tenant() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    let local = member(db.database(), tenant_a, "local", Role::Admin).await?;
    let foreign = member(db.database(), tenant_b, "foreign", Role::Viewer).await?;
    let platform = member(
        db.database(),
        PLATFORM_TENANT_ID,
        "root",
        Role::PlatformAdmin,
    )
    .await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;

    // A tenant transaction reads no account row directly.
    let (direct,): (i64,) = sqlx::query_as("SELECT count(*) FROM users")
        .fetch_one(tx.as_conn())
        .await?;
    assert_eq!(direct, 0);

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
    db.cleanup().await;
    Ok(())
}

/// The creator of an API token and of a secret must be a member of the row's tenant (ADR-0025).
#[tokio::test]
async fn db_the_creator_of_a_token_and_a_secret_must_be_a_member() -> TestResult {
    let Some(db) = test_database().await else {
        return Ok(());
    };
    let (tenant_a, tenant_b) = two_tenants(db.database()).await?;
    let seed_a = seed_tenant(db.database(), tenant_a).await?;
    let seed_b = seed_tenant(db.database(), tenant_b).await?;

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let token = sqlx::query(
        "INSERT INTO api_tokens \
         (id, tenant_id, name, token_hash, display_prefix, role, created_by) \
         VALUES ($1, $2, 't', $3, 'rh_x', 'viewer', $4)",
    )
    .bind(ApiTokenId::new())
    .bind(tenant_a)
    .bind(vec![7u8; 32])
    .bind(seed_b.user)
    .execute(tx.as_conn())
    .await;
    assert_code(token, "23503");
    drop(tx);

    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let secret = sqlx::query(
        "INSERT INTO secrets \
         (id, tenant_id, name, purpose, key_version, nonce, ciphertext, created_by) \
         VALUES ($1, $2, 's', 'repository_password', 1, $3, $4, $5)",
    )
    .bind(SecretId::new())
    .bind(tenant_a)
    .bind(vec![5u8; 12])
    .bind(vec![6u8; 32])
    .bind(seed_b.user)
    .execute(tx.as_conn())
    .await;
    assert_code(secret, "23503");
    drop(tx);

    // A creator that is a member of the tenant is accepted.
    let mut tx = db.database().begin_tenant(tenant_a).await?;
    let accepted = sqlx::query(
        "INSERT INTO api_tokens \
         (id, tenant_id, name, token_hash, display_prefix, role, created_by) \
         VALUES ($1, $2, 'ok', $3, 'rh_y', 'viewer', $4)",
    )
    .bind(ApiTokenId::new())
    .bind(tenant_a)
    .bind(vec![8u8; 32])
    .bind(seed_a.user)
    .execute(tx.as_conn())
    .await;
    assert!(accepted.is_ok(), "{accepted:?}");
    tx.commit().await?;

    db.cleanup().await;
    Ok(())
}
