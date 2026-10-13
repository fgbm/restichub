//! Membership operations.
//!
//! A membership gives an account a role in a tenant (ADR-0025). The role of a request comes from
//! the membership of the actor. A membership can be disabled in its tenant. The table has the
//! general tenant policy.

use uuid::Uuid;

use crate::DbError;
use crate::enums::Role;
use crate::ids::{TenantId, UserId};
use crate::tx::{PlatformTx, TenantTx};
use crate::users::WriteStatus;

#[derive(Debug, Clone)]
pub struct Membership {
    pub user_id: UserId,
    pub role: Role,
    pub disabled: bool,
}

/// One membership of an account with the name of the organization (ADR-0028).
#[derive(Debug, Clone)]
pub struct PlatformMembership {
    pub user_id: UserId,
    pub tenant_id: TenantId,
    pub tenant_name: String,
    pub role: Role,
    pub disabled: bool,
}

pub async fn role_of(tx: &mut TenantTx<'_>, user_id: UserId) -> Result<Option<Role>, DbError> {
    let row = sqlx::query_as::<_, (Role,)>(
        "SELECT role FROM memberships WHERE user_id = $1 AND NOT disabled",
    )
    .bind(user_id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(|(role,)| role))
}

/// Changes the role of an existing membership in the tenant of the transaction. It leaves the
/// `disabled` flag unchanged: only the platform `add_membership` re-enables a membership
/// (ADR-0025). A new membership is inserted by `users::create_member`.
pub async fn set_role(tx: &mut TenantTx<'_>, user_id: UserId, role: Role) -> Result<(), DbError> {
    sqlx::query("UPDATE memberships SET role = $2 WHERE user_id = $1")
        .bind(user_id)
        .bind(role.as_str())
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(())
}

/// Disables or enables the membership in the tenant of the transaction.
pub async fn set_disabled(
    tx: &mut TenantTx<'_>,
    user_id: UserId,
    disabled: bool,
) -> Result<bool, DbError> {
    let result = sqlx::query("UPDATE memberships SET disabled = $2 WHERE user_id = $1")
        .bind(user_id)
        .bind(disabled)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

pub async fn list(tx: &mut TenantTx<'_>) -> Result<Vec<Membership>, DbError> {
    let rows =
        sqlx::query_as::<_, (Uuid, Role, bool)>("SELECT user_id, role, disabled FROM memberships")
            .fetch_all(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(|(user_id, role, disabled)| Membership {
            user_id: UserId::from_uuid(user_id),
            role,
            disabled,
        })
        .collect())
}

/// Lists every membership of every account (Platform Admin only).
pub async fn list_all(tx: &mut PlatformTx<'_>) -> Result<Vec<PlatformMembership>, DbError> {
    let rows = sqlx::query_as::<_, (Uuid, Uuid, String, Role, bool)>(
        "SELECT user_id, tenant_id, tenant_name, role, disabled FROM auth_platform_memberships()",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(
            |(user_id, tenant_id, tenant_name, role, disabled)| PlatformMembership {
                user_id: UserId::from_uuid(user_id),
                tenant_id: TenantId::from_uuid(tenant_id),
                tenant_name,
                role,
                disabled,
            },
        )
        .collect())
}

/// Changes the role of a membership (Platform Admin only).
///
/// The database function requires the platform scope, refuses the platform tenant and takes the
/// account row lock.
pub async fn set_role_platform(
    tx: &mut PlatformTx<'_>,
    user_id: UserId,
    tenant_id: TenantId,
    role: Role,
) -> Result<WriteStatus, DbError> {
    let (status,): (String,) =
        sqlx::query_as("SELECT auth_platform_set_membership_role($1, $2, $3)")
            .bind(user_id)
            .bind(tenant_id)
            .bind(role.as_str())
            .fetch_one(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    WriteStatus::parse(&status)
}

/// Removes a membership (Platform Admin only).
///
/// The database function requires the platform scope, refuses the platform tenant and takes the
/// account row lock.
pub async fn remove_platform(
    tx: &mut PlatformTx<'_>,
    user_id: UserId,
    tenant_id: TenantId,
) -> Result<WriteStatus, DbError> {
    let (status,): (String,) = sqlx::query_as("SELECT auth_platform_remove_membership($1, $2)")
        .bind(user_id)
        .bind(tenant_id)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    WriteStatus::parse(&status)
}

/// Counts the active memberships with the administrator role in the tenant of the transaction.
///
/// The count covers the role `admin` only (ADR-0029). A disabled membership and a disabled account
/// both do not count, so an account that a Platform Admin disabled does not keep a tenant from
/// removing its last active Admin. The read goes through the member function, so the count agrees
/// with the list the tenant Admin sees. The caller holds the tenant lock ([`TenantTx::lock`]).
pub async fn count_admins(tx: &mut TenantTx<'_>) -> Result<i64, DbError> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM auth_tenant_members() \
         WHERE role = 'admin' AND NOT disabled AND NOT account_disabled",
    )
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(count)
}
