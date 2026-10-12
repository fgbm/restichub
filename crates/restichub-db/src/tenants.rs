//! Tenant operations.
//!
//! Only a [`PlatformTx`] can create and list tenants (ADR-0023, section Tenant isolation). A
//! tenant-scoped transaction reads its own row only.

use uuid::Uuid;

use crate::DbError;
use crate::ids::{AuditLogId, TenantId, UserId};
use crate::tx::PlatformTx;

/// A tenant row.
#[derive(Debug, Clone)]
pub struct Tenant {
    /// The identifier of the tenant.
    pub id: TenantId,
    /// The name of the tenant. The name is unique.
    pub name: String,
    /// The status: `active`, `suspended` or `deleted`.
    pub status: String,
}

/// The first administrator of a new tenant (N2).
#[derive(Debug, Clone)]
pub struct BootstrapAdmin<'a> {
    /// The username. The name is unique for the tenant.
    pub username: &'a str,
    /// The password hash (Argon2id PHC string).
    pub password_hash: &'a str,
    /// The name for the user interface.
    pub display_name: &'a str,
}

/// The input of [`create_with_bootstrap`] (N2).
///
/// The caller wraps the data encryption key (DEK). The caller passes the wrapped bytes.
#[derive(Debug, Clone)]
pub struct BootstrapTenant<'a> {
    /// The name of the tenant. The name is unique.
    pub name: &'a str,
    /// The version of the tenant key.
    pub key_version: i32,
    /// The identifier of the key encryption key (KEK) that wrapped the DEK.
    pub kek_id: &'a str,
    /// The wrapped DEK.
    pub wrapped_dek: &'a [u8],
    /// The nonce of the wrapped DEK.
    pub nonce: &'a [u8],
    /// The first administrator. It is optional.
    pub admin: Option<BootstrapAdmin<'a>>,
}

/// Creates a tenant and returns its identifier.
///
/// The transaction records the tenant. Only this tenant is valid for
/// [`PlatformTx::enter_tenant`].
pub async fn create(tx: &mut PlatformTx<'_>, name: &str) -> Result<TenantId, DbError> {
    let id = TenantId::new();
    sqlx::query("INSERT INTO tenants (id, tenant_id, name) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(id)
        .bind(name)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    tx.mark_created(id);
    Ok(id)
}

/// Creates a tenant with its key row, an optional first administrator and an audit record (N2).
///
/// The function writes every row in the transaction of the caller. The rows commit together or not
/// at all. The future tasks T3 to T6 use this function.
pub async fn create_with_bootstrap(
    tx: &mut PlatformTx<'_>,
    input: &BootstrapTenant<'_>,
) -> Result<TenantId, DbError> {
    let tenant = create(tx, input.name).await?;
    tx.enter_tenant(tenant).await?;

    sqlx::query(
        "INSERT INTO tenant_keys (tenant_id, key_version, kek_id, wrapped_dek, nonce) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(tenant)
    .bind(input.key_version)
    .bind(input.kek_id)
    .bind(input.wrapped_dek)
    .bind(input.nonce)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;

    if let Some(admin) = &input.admin {
        let user = UserId::new();
        sqlx::query(
            "INSERT INTO users (id, tenant_id, username, password_hash, display_name) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user)
        .bind(tenant)
        .bind(admin.username)
        .bind(admin.password_hash)
        .bind(admin.display_name)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
        sqlx::query("INSERT INTO memberships (tenant_id, user_id, role) VALUES ($1, $2, 'admin')")
            .bind(tenant)
            .bind(user)
            .execute(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    }

    sqlx::query(
        "INSERT INTO audit_log (id, tenant_id, actor_type, action, object_type, object_id) \
         VALUES ($1, $2, 'system', 'tenant.create', 'tenant', $2)",
    )
    .bind(AuditLogId::new())
    .bind(tenant)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;

    Ok(tenant)
}

/// Lists the tenants.
pub async fn list(tx: &mut PlatformTx<'_>) -> Result<Vec<Tenant>, DbError> {
    let rows = sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id, name, status FROM tenants ORDER BY id",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(|(id, name, status)| Tenant {
            id: TenantId::from_uuid(id),
            name,
            status,
        })
        .collect())
}
