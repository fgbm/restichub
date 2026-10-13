//! User account operations.
//!
//! An account is installation-wide and its row lives in the platform tenant (ADR-0025). The
//! username is unique in the installation and the comparison ignores the letter case. A tenant
//! transaction reads no account row directly: it calls the `SECURITY DEFINER` functions of
//! migration 0016 through this module.

use uuid::Uuid;

use crate::Database;
use crate::DbError;
use crate::enums::Role;
use crate::ids::{TenantId, UserId};
use crate::tx::TenantTx;

/// Installation-wide identity that a person signs in with.
#[derive(Debug, Clone)]
pub struct Account {
    pub id: UserId,
    pub username: String,
    pub display_name: String,
    /// A Platform Admin set it for the whole installation.
    pub disabled: bool,
    /// The tenant that was last used through this account.
    pub last_used_tenant_id: Option<TenantId>,
}

/// Carries the password hash. The control plane never sends it.
#[derive(Debug, Clone)]
pub struct AccountAuth {
    pub id: UserId,
    /// Argon2id PHC string.
    pub password_hash: String,
    pub disabled: bool,
    /// The tenant that was last used through this account.
    pub last_used_tenant_id: Option<TenantId>,
}

/// A member of a tenant: an account with its membership (ADR-0025).
#[derive(Debug, Clone)]
pub struct Member {
    pub id: UserId,
    pub username: String,
    pub display_name: String,
    pub role: Role,
    /// Off in this tenant only; the identity stays usable elsewhere.
    pub disabled: bool,
    /// A Platform Admin set it for the whole installation.
    pub account_disabled: bool,
    /// The account holds an active membership in another active tenant.
    pub shared: bool,
}

#[derive(Debug, Clone)]
pub struct NewUser<'a> {
    /// Unique in the installation; ignores the letter case.
    pub username: &'a str,
    /// Argon2id PHC string.
    pub password_hash: &'a str,
    pub display_name: &'a str,
}

/// Resolves a username for the sign-in, ignoring the letter case.
pub async fn find_account(db: &Database, username: &str) -> Result<Option<AccountAuth>, DbError> {
    let row = sqlx::query_as::<_, (Uuid, String, bool, Option<Uuid>)>(
        "SELECT id, password_hash, disabled, last_used_tenant_id FROM auth_find_account($1)",
    )
    .bind(username)
    .fetch_optional(db.pool())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(
        |(id, password_hash, disabled, last_used_tenant_id)| AccountAuth {
            id: UserId::from_uuid(id),
            password_hash,
            disabled,
            last_used_tenant_id: last_used_tenant_id.map(TenantId::from_uuid),
        },
    ))
}

/// Finds the account by the identifier.
pub async fn find_account_by_id(db: &Database, id: UserId) -> Result<Option<Account>, DbError> {
    let row = sqlx::query_as::<_, (Uuid, String, String, bool, Option<Uuid>)>(
        "SELECT id, username, display_name, disabled, last_used_tenant_id \
         FROM auth_find_account_by_id($1)",
    )
    .bind(id)
    .fetch_optional(db.pool())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(
        |(id, username, display_name, disabled, last_used_tenant_id)| Account {
            id: UserId::from_uuid(id),
            username,
            display_name,
            disabled,
            last_used_tenant_id: last_used_tenant_id.map(TenantId::from_uuid),
        },
    ))
}

/// Finds the account with its password hash by the identifier.
pub async fn find_account_auth_by_id(
    db: &Database,
    id: UserId,
) -> Result<Option<AccountAuth>, DbError> {
    let row = sqlx::query_as::<_, (Uuid, String, bool, Option<Uuid>)>(
        "SELECT id, password_hash, disabled, last_used_tenant_id FROM auth_find_account_by_id($1)",
    )
    .bind(id)
    .fetch_optional(db.pool())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(
        |(id, password_hash, disabled, last_used_tenant_id)| AccountAuth {
            id: UserId::from_uuid(id),
            password_hash,
            disabled,
            last_used_tenant_id: last_used_tenant_id.map(TenantId::from_uuid),
        },
    ))
}

/// Creates an account in the platform tenant. The row commits with the caller's transaction.
pub async fn create(tx: &mut TenantTx<'_>, new: &NewUser<'_>) -> Result<UserId, DbError> {
    let id = UserId::new();
    sqlx::query("SELECT auth_create_account($1, $2, $3, $4)")
        .bind(id)
        .bind(new.username)
        .bind(new.password_hash)
        .bind(new.display_name)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(id)
}

/// The new password applies to the whole installation.
pub async fn set_password(
    tx: &mut TenantTx<'_>,
    id: UserId,
    password_hash: &str,
) -> Result<bool, DbError> {
    let (updated,): (bool,) = sqlx::query_as("SELECT auth_set_account_password($1, $2)")
        .bind(id)
        .bind(password_hash)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(updated)
}

pub async fn set_display_name(
    tx: &mut TenantTx<'_>,
    id: UserId,
    display_name: &str,
) -> Result<bool, DbError> {
    let (updated,): (bool,) = sqlx::query_as("SELECT auth_set_account_display_name($1, $2)")
        .bind(id)
        .bind(display_name)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(updated)
}

/// Disables or enables the account itself. Only a Platform Admin calls this.
pub async fn set_disabled(
    tx: &mut TenantTx<'_>,
    id: UserId,
    disabled: bool,
) -> Result<bool, DbError> {
    let (updated,): (bool,) = sqlx::query_as("SELECT auth_set_account_disabled($1, $2)")
        .bind(id)
        .bind(disabled)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(updated)
}

/// Stores the tenant that was last used through the account.
pub async fn set_last_tenant(
    tx: &mut TenantTx<'_>,
    id: UserId,
    tenant_id: TenantId,
) -> Result<(), DbError> {
    sqlx::query("SELECT auth_set_last_tenant($1, $2)")
        .bind(id)
        .bind(tenant_id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(())
}

/// Lists the members of the tenant of the transaction: the accounts with a membership in it.
pub async fn list_members(tx: &mut TenantTx<'_>) -> Result<Vec<Member>, DbError> {
    let rows = sqlx::query_as::<_, MemberRow>(
        "SELECT user_id, username, display_name, role, disabled, account_disabled, shared \
         FROM auth_tenant_members()",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(Member::from).collect())
}

/// Reads one member of the tenant of the transaction. `None` when the account is not a member.
pub async fn get_member(tx: &mut TenantTx<'_>, id: UserId) -> Result<Option<Member>, DbError> {
    let row = sqlx::query_as::<_, MemberRow>(
        "SELECT user_id, username, display_name, role, disabled, account_disabled, shared \
         FROM auth_tenant_member($1)",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(Member::from))
}

type MemberRow = (Uuid, String, String, Role, bool, bool, bool);

impl From<MemberRow> for Member {
    fn from(row: MemberRow) -> Self {
        let (id, username, display_name, role, disabled, account_disabled, shared) = row;
        Self {
            id: UserId::from_uuid(id),
            username,
            display_name,
            role,
            disabled,
            account_disabled,
            shared,
        }
    }
}
