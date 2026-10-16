//! User account operations.
//!
//! An account is installation-wide and its row lives in the platform tenant (ADR-0025). The
//! username is unique in the installation and the comparison ignores the letter case. A tenant
//! transaction reads no account row directly: it calls the `SECURITY DEFINER` functions of
//! migration 0017 through this module. Each function carries its own scope rule at the database
//! level, so this module cannot widen the reach of a caller.

use uuid::Uuid;

use crate::Database;
use crate::DbError;
use crate::enums::Role;
use crate::ids::{SessionId, TenantId, UserId};
use crate::tx::{PlatformTx, TenantTx};

/// Installation-wide identity that a person signs in with. It holds no password hash.
#[derive(Debug, Clone)]
pub struct Account {
    pub id: UserId,
    pub username: String,
    pub display_name: String,
    /// A Platform Admin set it for the whole installation.
    pub disabled: bool,
    /// The tenant that was last used through this account.
    pub last_used_tenant_id: Option<TenantId>,
    /// True when the account holds the installation-wide right (ADR-0029).
    pub platform_admin: bool,
}

/// An account row with the password hash. The control plane never sends it.
#[derive(Debug, Clone)]
pub struct AccountAuth {
    pub id: UserId,
    /// Argon2id PHC string.
    pub password_hash: String,
    pub disabled: bool,
    /// The tenant that was last used through this account.
    pub last_used_tenant_id: Option<TenantId>,
    /// True when the account holds the installation-wide right (ADR-0029).
    pub platform_admin: bool,
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
    /// The account holds a membership in another tenant that is not deleted.
    pub shared: bool,
    /// True when the account holds the installation-wide right (ADR-0029).
    pub platform_admin: bool,
}

/// The result of an account write of the database functions (migration 0017).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteStatus {
    Ok,
    /// The account holds no membership in the current tenant, or the account is absent.
    NotFound,
    /// The account holds a membership in another tenant that is not deleted.
    Shared,
    /// The current tenant or scope may not do this write.
    Forbidden,
}

impl WriteStatus {
    pub(crate) fn parse(value: &str) -> Result<Self, DbError> {
        match value {
            "ok" => Ok(Self::Ok),
            "not_found" => Ok(Self::NotFound),
            "shared" => Ok(Self::Shared),
            "forbidden" => Ok(Self::Forbidden),
            other => Err(DbError::UnknownWriteStatus(other.to_owned())),
        }
    }
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
    let row = sqlx::query_as::<_, (Uuid, String, bool, Option<Uuid>, bool)>(
        "SELECT id, password_hash, disabled, last_used_tenant_id, platform_admin \
         FROM auth_find_account($1)",
    )
    .bind(username)
    .fetch_optional(db.pool())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(
        |(id, password_hash, disabled, last_used_tenant_id, platform_admin)| AccountAuth {
            id: UserId::from_uuid(id),
            password_hash,
            disabled,
            last_used_tenant_id: last_used_tenant_id.map(TenantId::from_uuid),
            platform_admin,
        },
    ))
}

/// Finds the account by the identifier. It returns no password hash.
pub async fn find_account_by_id(db: &Database, id: UserId) -> Result<Option<Account>, DbError> {
    let row = sqlx::query_as::<_, (Uuid, String, String, bool, Option<Uuid>, bool)>(
        "SELECT id, username, display_name, disabled, last_used_tenant_id, platform_admin \
         FROM auth_find_account_by_id($1)",
    )
    .bind(id)
    .fetch_optional(db.pool())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(
        |(id, username, display_name, disabled, last_used_tenant_id, platform_admin)| Account {
            id: UserId::from_uuid(id),
            username,
            display_name,
            disabled,
            last_used_tenant_id: last_used_tenant_id.map(TenantId::from_uuid),
            platform_admin,
        },
    ))
}

/// The password hash of the caller's own account, bound to the caller's own session.
pub async fn self_password_hash(
    tx: &mut TenantTx<'_>,
    account_id: UserId,
    session_id: SessionId,
) -> Result<Option<String>, DbError> {
    let row =
        sqlx::query_as::<_, (String,)>("SELECT password_hash FROM auth_self_password_hash($1, $2)")
            .bind(account_id)
            .bind(session_id)
            .fetch_optional(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(row.map(|(hash,)| hash))
}

/// Creates an account with its first membership in the tenant of the transaction (ADR-0025). The
/// database function creates both rows in one step, takes the account row lock and applies the
/// scope rule.
pub async fn create_member(
    tx: &mut TenantTx<'_>,
    new: &NewUser<'_>,
    role: Role,
) -> Result<UserId, DbError> {
    let id = UserId::new();
    let (created,): (bool,) = sqlx::query_as("SELECT auth_create_member($1, $2, $3, $4, $5, $6)")
        .bind(id)
        .bind(new.username)
        .bind(new.password_hash)
        .bind(new.display_name)
        .bind(tx.tenant())
        .bind(role.as_str())
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    if !created {
        return Err(DbError::AccountCreateRefused);
    }
    Ok(id)
}

/// The password write of an administrator (ADR-0025). It needs a membership in the current tenant
/// and refuses a shared account unless the current tenant is the platform tenant.
pub async fn admin_set_password(
    tx: &mut TenantTx<'_>,
    id: UserId,
    password_hash: &str,
) -> Result<WriteStatus, DbError> {
    let (status,): (String,) = sqlx::query_as("SELECT auth_admin_set_password($1, $2)")
        .bind(id)
        .bind(password_hash)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    WriteStatus::parse(&status)
}

/// The password write of the Platform Admin for any account (migration 0051). It needs the platform
/// tenant or the platform scope and no membership.
pub async fn platform_set_password(
    tx: &mut TenantTx<'_>,
    id: UserId,
    password_hash: &str,
) -> Result<WriteStatus, DbError> {
    let (status,): (String,) = sqlx::query_as("SELECT auth_platform_set_password($1, $2)")
        .bind(id)
        .bind(password_hash)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    WriteStatus::parse(&status)
}

/// The display name write of an administrator. The same scope rule as [`admin_set_password`].
pub async fn admin_set_display_name(
    tx: &mut TenantTx<'_>,
    id: UserId,
    display_name: &str,
) -> Result<WriteStatus, DbError> {
    let (status,): (String,) = sqlx::query_as("SELECT auth_admin_set_display_name($1, $2)")
        .bind(id)
        .bind(display_name)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    WriteStatus::parse(&status)
}

/// The password change of the user itself, bound to the caller's own session.
pub async fn self_set_password(
    tx: &mut TenantTx<'_>,
    id: UserId,
    session_id: SessionId,
    password_hash: &str,
) -> Result<bool, DbError> {
    let (written,): (bool,) = sqlx::query_as("SELECT auth_self_set_password($1, $2, $3)")
        .bind(id)
        .bind(session_id)
        .bind(password_hash)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(written)
}

/// Disables or enables the account itself. Only the platform tenant or the platform scope may do
/// it.
pub async fn set_disabled(
    tx: &mut TenantTx<'_>,
    id: UserId,
    disabled: bool,
) -> Result<WriteStatus, DbError> {
    let (status,): (String,) = sqlx::query_as("SELECT auth_set_account_disabled($1, $2)")
        .bind(id)
        .bind(disabled)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    WriteStatus::parse(&status)
}

/// Stores the last used tenant, for the account of the caller's own session only.
pub async fn set_last_tenant(
    tx: &mut TenantTx<'_>,
    id: UserId,
    session_id: SessionId,
    tenant_id: TenantId,
) -> Result<bool, DbError> {
    let (written,): (bool,) = sqlx::query_as("SELECT auth_set_last_tenant($1, $2, $3)")
        .bind(id)
        .bind(session_id)
        .bind(tenant_id)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(written)
}

/// Lists the members of the tenant of the transaction: the accounts with a membership in it.
pub async fn list_members(tx: &mut TenantTx<'_>) -> Result<Vec<Member>, DbError> {
    let rows = sqlx::query_as::<_, MemberRow>(
        "SELECT user_id, username, display_name, role, disabled, account_disabled, shared, \
                platform_admin \
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
        "SELECT user_id, username, display_name, role, disabled, account_disabled, shared, \
                platform_admin \
         FROM auth_tenant_member($1)",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(Member::from))
}

/// An installation account for the platform administration (ADR-0028).
#[derive(Debug, Clone)]
pub struct PlatformAccount {
    pub id: UserId,
    pub username: String,
    pub display_name: String,
    /// Unique in the installation when it is set.
    pub email: Option<String>,
    /// False until an invitation verifies the address; the MVP leaves it false.
    pub email_verified: bool,
    /// A Platform Admin set it for the whole installation.
    pub disabled: bool,
    /// True when the account holds the installation-wide right (ADR-0029).
    pub platform_admin: bool,
}

#[derive(Debug, Clone)]
pub struct NewPlatformAccount<'a> {
    /// Unique in the installation; ignores the letter case.
    pub username: &'a str,
    /// Argon2id PHC string.
    pub password_hash: &'a str,
    pub display_name: &'a str,
    pub email: Option<&'a str>,
    /// Grants the installation-wide right at the create (ADR-0029).
    pub platform_admin: bool,
}

/// The result of the platform account create (migration 0023).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateAccountStatus {
    Ok,
    /// Duplicate username in the installation.
    UsernameTaken,
    /// Duplicate email in the installation.
    EmailTaken,
    /// The transaction is not in the platform scope.
    Forbidden,
}

impl CreateAccountStatus {
    fn parse(value: &str) -> Result<Self, DbError> {
        match value {
            "ok" => Ok(Self::Ok),
            "username_taken" => Ok(Self::UsernameTaken),
            "email_taken" => Ok(Self::EmailTaken),
            "forbidden" => Ok(Self::Forbidden),
            other => Err(DbError::UnknownWriteStatus(other.to_owned())),
        }
    }
}

/// Reads the accounts of the installation for the platform administration.
pub async fn list_platform_accounts(
    tx: &mut PlatformTx<'_>,
) -> Result<Vec<PlatformAccount>, DbError> {
    let rows = sqlx::query_as::<_, (Uuid, String, String, Option<String>, bool, bool, bool)>(
        "SELECT user_id, username, display_name, email, email_verified, disabled, platform_admin \
         FROM auth_platform_accounts()",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(
            |(id, username, display_name, email, email_verified, disabled, platform_admin)| {
                PlatformAccount {
                    id: UserId::from_uuid(id),
                    username,
                    display_name,
                    email,
                    email_verified,
                    disabled,
                    platform_admin,
                }
            },
        )
        .collect())
}

/// Creates an installation account (Platform Admin only).
///
/// The database function requires the platform scope, takes the account row lock and refuses a
/// duplicate username or email.
pub async fn platform_create_account(
    tx: &mut PlatformTx<'_>,
    id: UserId,
    new: &NewPlatformAccount<'_>,
) -> Result<CreateAccountStatus, DbError> {
    let (status,): (String,) =
        sqlx::query_as("SELECT auth_platform_create_account($1, $2, $3, $4, $5, $6)")
            .bind(id)
            .bind(new.username)
            .bind(new.password_hash)
            .bind(new.display_name)
            .bind(new.email)
            .bind(new.platform_admin)
            .fetch_one(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    CreateAccountStatus::parse(&status)
}

/// Creates the first Platform Admin account with the right and no membership (ADR-0029). The setup
/// calls it in the platform tenant.
pub async fn create_platform_admin(
    tx: &mut TenantTx<'_>,
    id: UserId,
    new: &NewUser<'_>,
) -> Result<bool, DbError> {
    let (created,): (bool,) = sqlx::query_as("SELECT auth_create_platform_admin($1, $2, $3, $4)")
        .bind(id)
        .bind(new.username)
        .bind(new.password_hash)
        .bind(new.display_name)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(created)
}

/// Reads the number of the active installation administrators (ADR-0029). It needs the platform
/// tenant or the platform scope.
pub async fn platform_admin_count(tx: &mut TenantTx<'_>) -> Result<i64, DbError> {
    let (count,): (i64,) = sqlx::query_as("SELECT auth_platform_admin_count()")
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(count)
}

/// True when the account holds the active Platform Admin right (ADR-0029).
pub async fn is_platform_admin(tx: &mut TenantTx<'_>, id: UserId) -> Result<bool, DbError> {
    let (value,): (bool,) = sqlx::query_as("SELECT auth_is_platform_admin($1)")
        .bind(id)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(value)
}

/// Writes the Platform Admin right of an account (ADR-0029). It needs the platform tenant or the
/// platform scope and takes the account row lock.
pub async fn set_platform_admin(
    tx: &mut TenantTx<'_>,
    id: UserId,
    value: bool,
) -> Result<WriteStatus, DbError> {
    let (status,): (String,) = sqlx::query_as("SELECT auth_set_platform_admin($1, $2)")
        .bind(id)
        .bind(value)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    WriteStatus::parse(&status)
}

/// Grants the Platform Admin right to an existing account with a new password and ends every
/// session of the account (ADR-0029). It needs the platform tenant or the platform scope and takes
/// the account row lock.
pub async fn grant_platform_admin(
    tx: &mut TenantTx<'_>,
    id: UserId,
    password_hash: &str,
) -> Result<WriteStatus, DbError> {
    let (status,): (String,) = sqlx::query_as("SELECT auth_grant_platform_admin($1, $2)")
        .bind(id)
        .bind(password_hash)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    WriteStatus::parse(&status)
}

/// Writes the email address of an account for the platform administration.
///
/// The unique index of the email makes a duplicate a database error with SQLSTATE 23505.
pub async fn platform_set_email(
    tx: &mut TenantTx<'_>,
    id: UserId,
    email: &str,
) -> Result<WriteStatus, DbError> {
    let (status,): (String,) = sqlx::query_as("SELECT auth_platform_set_email($1, $2)")
        .bind(id)
        .bind(email)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    WriteStatus::parse(&status)
}

type MemberRow = (Uuid, String, String, Role, bool, bool, bool, bool);

impl From<MemberRow> for Member {
    fn from(row: MemberRow) -> Self {
        let (id, username, display_name, role, disabled, account_disabled, shared, platform_admin) =
            row;
        Self {
            id: UserId::from_uuid(id),
            username,
            display_name,
            role,
            disabled,
            account_disabled,
            shared,
            platform_admin,
        }
    }
}
