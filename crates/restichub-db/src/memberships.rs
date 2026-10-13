//! Membership operations.
//!
//! A membership gives a user a role in a tenant (requirements section 2). The role of a request
//! comes from the membership of the actor. The table has the general tenant policy.

use uuid::Uuid;

use crate::DbError;
use crate::enums::Role;
use crate::ids::UserId;
use crate::tx::TenantTx;

#[derive(Debug, Clone)]
pub struct Membership {
    pub user_id: UserId,
    pub role: Role,
}

pub async fn role_of(tx: &mut TenantTx<'_>, user_id: UserId) -> Result<Option<Role>, DbError> {
    let row = sqlx::query_as::<_, (Role,)>("SELECT role FROM memberships WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(row.map(|(role,)| role))
}

pub async fn set_role(tx: &mut TenantTx<'_>, user_id: UserId, role: Role) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO memberships (tenant_id, user_id, role) VALUES ($1, $2, $3) \
         ON CONFLICT (tenant_id, user_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(tx.tenant())
    .bind(user_id)
    .bind(role.as_str())
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

pub async fn list(tx: &mut TenantTx<'_>) -> Result<Vec<Membership>, DbError> {
    let rows = sqlx::query_as::<_, (Uuid, Role)>("SELECT user_id, role FROM memberships")
        .fetch_all(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(|(user_id, role)| Membership {
            user_id: UserId::from_uuid(user_id),
            role,
        })
        .collect())
}

/// Counts the memberships with an administrator role in the tenant of the transaction.
///
/// The count covers the roles `admin` and `platform_admin` and skips the memberships of a disabled
/// user. A disabled Admin therefore does not count as an Admin. The rule "an Admin cannot remove
/// the last Admin of a tenant" uses it. The caller holds the tenant lock ([`TenantTx::lock`]).
pub async fn count_admins(tx: &mut TenantTx<'_>) -> Result<i64, DbError> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM memberships m \
         JOIN users u ON u.tenant_id = m.tenant_id AND u.id = m.user_id \
         WHERE m.role IN ('admin', 'platform_admin') AND NOT u.disabled",
    )
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(count)
}

pub async fn delete(tx: &mut TenantTx<'_>, user_id: UserId) -> Result<(), DbError> {
    sqlx::query("DELETE FROM memberships WHERE user_id = $1")
        .bind(user_id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(())
}
