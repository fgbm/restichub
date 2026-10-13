//! User operations.
//!
//! Usernames are unique for each tenant. Sign-in therefore takes the tenant name and the username,
//! not the username alone (ADR-0023, section Local authentication). The authentication code
//! arrives in a later task.

use uuid::Uuid;

use crate::DbError;
use crate::ids::UserId;
use crate::tx::TenantTx;

#[derive(Debug, Clone)]
pub struct User {
    pub id: UserId,
    /// Unique for the tenant.
    pub username: String,
    pub display_name: String,
    pub disabled: bool,
}

#[derive(Debug, Clone)]
pub struct NewUser<'a> {
    /// Unique for the tenant.
    pub username: &'a str,
    /// Argon2id PHC string.
    pub password_hash: &'a str,
    pub display_name: &'a str,
}

pub async fn create(tx: &mut TenantTx<'_>, new: &NewUser<'_>) -> Result<UserId, DbError> {
    let id = UserId::new();
    sqlx::query(
        "INSERT INTO users (id, tenant_id, username, password_hash, display_name) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(tx.tenant())
    .bind(new.username)
    .bind(new.password_hash)
    .bind(new.display_name)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(id)
}

pub async fn list(tx: &mut TenantTx<'_>) -> Result<Vec<User>, DbError> {
    let rows = sqlx::query_as::<_, (Uuid, String, String, bool)>(
        "SELECT id, username, display_name, disabled FROM users ORDER BY username",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(|(id, username, display_name, disabled)| User {
            id: UserId::from_uuid(id),
            username,
            display_name,
            disabled,
        })
        .collect())
}

/// The type holds the password hash. The control plane never sends it and never writes it to the
/// audit log.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: UserId,
    /// Argon2id PHC string.
    pub password_hash: String,
    pub disabled: bool,
}

/// Finds a user by the username of the tenant of the transaction.
///
/// The comparison ignores the letter case, as the unique index `users_tenant_username_lower_unique`
/// does.
pub async fn find_for_auth(
    tx: &mut TenantTx<'_>,
    username: &str,
) -> Result<Option<AuthUser>, DbError> {
    let row = sqlx::query_as::<_, (Uuid, String, bool)>(
        "SELECT id, password_hash, disabled FROM users WHERE lower(username) = lower($1)",
    )
    .bind(username)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(|(id, password_hash, disabled)| AuthUser {
        id: UserId::from_uuid(id),
        password_hash,
        disabled,
    }))
}

pub async fn get(tx: &mut TenantTx<'_>, id: UserId) -> Result<Option<User>, DbError> {
    let row = sqlx::query_as::<_, (Uuid, String, String, bool)>(
        "SELECT id, username, display_name, disabled FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(|(id, username, display_name, disabled)| User {
        id: UserId::from_uuid(id),
        username,
        display_name,
        disabled,
    }))
}

pub async fn get_auth(tx: &mut TenantTx<'_>, id: UserId) -> Result<Option<AuthUser>, DbError> {
    let row = sqlx::query_as::<_, (Uuid, String, bool)>(
        "SELECT id, password_hash, disabled FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(|(id, password_hash, disabled)| AuthUser {
        id: UserId::from_uuid(id),
        password_hash,
        disabled,
    }))
}

pub async fn set_password(
    tx: &mut TenantTx<'_>,
    id: UserId,
    password_hash: &str,
) -> Result<bool, DbError> {
    let result =
        sqlx::query("UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1")
            .bind(id)
            .bind(password_hash)
            .execute(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

pub async fn set_display_name(
    tx: &mut TenantTx<'_>,
    id: UserId,
    display_name: &str,
) -> Result<bool, DbError> {
    let result =
        sqlx::query("UPDATE users SET display_name = $2, updated_at = now() WHERE id = $1")
            .bind(id)
            .bind(display_name)
            .execute(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

pub async fn set_disabled(
    tx: &mut TenantTx<'_>,
    id: UserId,
    disabled: bool,
) -> Result<bool, DbError> {
    let result = sqlx::query("UPDATE users SET disabled = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(disabled)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}
