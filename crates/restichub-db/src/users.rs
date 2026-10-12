//! User operations.
//!
//! Usernames are unique for each tenant. Sign-in therefore takes the tenant name and the username,
//! not the username alone (ADR-0023, section Local authentication). The authentication code
//! arrives in a later task.

use uuid::Uuid;

use crate::DbError;
use crate::ids::UserId;
use crate::tx::TenantTx;

/// A user row.
#[derive(Debug, Clone)]
pub struct User {
    /// The identifier of the user.
    pub id: UserId,
    /// The username. The name is unique for the tenant.
    pub username: String,
    /// The name for the user interface.
    pub display_name: String,
    /// The flag that disables the user.
    pub disabled: bool,
}

/// The data of a new user.
#[derive(Debug, Clone)]
pub struct NewUser<'a> {
    /// The username. The name is unique for the tenant.
    pub username: &'a str,
    /// The password hash (Argon2id PHC string).
    pub password_hash: &'a str,
    /// The name for the user interface.
    pub display_name: &'a str,
}

/// Creates a user in the tenant of the transaction and returns its identifier.
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

/// Lists the users of the tenant of the transaction.
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
