//! The repository rows of a tenant (FR-RP-01, FR-RP-08, ADR-0003).
//!
//! One row holds the backend settings, the identifiers of the bound secrets and the maintenance
//! agent. The row never holds a secret value: the values live in the encrypted `secrets` table
//! (ADR-0017).
//!
//! The general tenant policy of the `repositories` table shows the rows of the tenant of the
//! transaction only (ADR-0023, section Tenant isolation).

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::enums::BackendKind;
use crate::ids::{AgentId, RepositoryId, SecretId, UserId};
use crate::tx::TenantTx;

/// The fields of the backend settings of a repository.
///
/// The group of the field follows the backend kind. The database check constraint refuses a field
/// of another backend.
#[derive(Debug, Clone, Default)]
pub struct RepositorySettings<'a> {
    pub s3_endpoint: Option<&'a str>,
    pub s3_bucket: Option<&'a str>,
    pub s3_prefix: Option<&'a str>,
    pub s3_region: Option<&'a str>,
    pub rest_endpoint: Option<&'a str>,
    pub local_path: Option<&'a str>,
}

/// The identifiers of the secrets that a repository binds. The repository password is mandatory;
/// the backend credentials depend on the backend.
#[derive(Debug, Clone, Copy)]
pub struct RepositorySecrets {
    pub password: SecretId,
    pub s3_access_key: Option<SecretId>,
    pub s3_secret_key: Option<SecretId>,
    pub rest_password: Option<SecretId>,
}

/// One stored backend address with its credentials and its maintenance agent.
#[derive(Debug, Clone)]
pub struct Repository {
    pub id: RepositoryId,
    pub name: String,
    pub backend: BackendKind,
    pub s3_endpoint: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_prefix: Option<String>,
    pub s3_region: Option<String>,
    pub rest_endpoint: Option<String>,
    pub local_path: Option<String>,
    pub password_secret_id: SecretId,
    pub s3_access_key_secret_id: Option<SecretId>,
    pub s3_secret_key_secret_id: Option<SecretId>,
    pub rest_password_secret_id: Option<SecretId>,
    pub maintenance_agent_id: Option<AgentId>,
    pub initialized: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub created_by: Option<UserId>,
}

/// The fields of a new row in the `repositories` table.
#[derive(Debug, Clone)]
pub struct NewRepository<'a> {
    pub id: RepositoryId,
    pub name: &'a str,
    pub backend: BackendKind,
    pub settings: RepositorySettings<'a>,
    pub secrets: RepositorySecrets,
    pub maintenance_agent_id: Option<AgentId>,
    /// Absent for an API token actor.
    pub created_by: Option<UserId>,
}

/// Mutable fields of a stored row.
#[derive(Debug, Clone)]
pub struct RepositoryUpdate<'a> {
    pub name: &'a str,
    pub settings: RepositorySettings<'a>,
    pub secrets: RepositorySecrets,
    pub maintenance_agent_id: Option<AgentId>,
}

pub async fn create(
    tx: &mut TenantTx<'_>,
    new: &NewRepository<'_>,
) -> Result<RepositoryId, DbError> {
    sqlx::query(
        "INSERT INTO repositories \
         (id, tenant_id, name, backend, s3_endpoint, s3_bucket, s3_prefix, s3_region, \
          rest_endpoint, local_path, password_secret_id, s3_access_key_secret_id, \
          s3_secret_key_secret_id, rest_password_secret_id, maintenance_agent_id, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
    )
    .bind(new.id)
    .bind(tx.tenant())
    .bind(new.name)
    .bind(new.backend.as_str())
    .bind(new.settings.s3_endpoint)
    .bind(new.settings.s3_bucket)
    .bind(new.settings.s3_prefix)
    .bind(new.settings.s3_region)
    .bind(new.settings.rest_endpoint)
    .bind(new.settings.local_path)
    .bind(new.secrets.password)
    .bind(new.secrets.s3_access_key)
    .bind(new.secrets.s3_secret_key)
    .bind(new.secrets.rest_password)
    .bind(new.maintenance_agent_id)
    .bind(new.created_by)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(new.id)
}

/// Lists the repositories of the tenant of the transaction, newest first.
pub async fn list(tx: &mut TenantTx<'_>) -> Result<Vec<Repository>, DbError> {
    let rows = sqlx::query_as::<_, RepositoryRow>(
        "SELECT id, name, backend, s3_endpoint, s3_bucket, s3_prefix, s3_region, rest_endpoint, \
                local_path, password_secret_id, s3_access_key_secret_id, s3_secret_key_secret_id, \
                rest_password_secret_id, maintenance_agent_id, initialized, created_at, updated_at, \
                created_by \
         FROM repositories ORDER BY created_at DESC, id DESC",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(Repository::from).collect())
}

/// Reads one repository of the tenant of the transaction. A repository of another tenant is absent.
pub async fn get(tx: &mut TenantTx<'_>, id: RepositoryId) -> Result<Option<Repository>, DbError> {
    let row = sqlx::query_as::<_, RepositoryRow>(
        "SELECT id, name, backend, s3_endpoint, s3_bucket, s3_prefix, s3_region, rest_endpoint, \
                local_path, password_secret_id, s3_access_key_secret_id, s3_secret_key_secret_id, \
                rest_password_secret_id, maintenance_agent_id, initialized, created_at, updated_at, \
                created_by \
         FROM repositories WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(Repository::from))
}

/// Reads one repository and locks its row for the transaction.
///
/// An update or a delete uses this function, so a concurrent change cannot remove the row between
/// the read and the write.
pub async fn get_for_update(
    tx: &mut TenantTx<'_>,
    id: RepositoryId,
) -> Result<Option<Repository>, DbError> {
    let row = sqlx::query_as::<_, RepositoryRow>(
        "SELECT id, name, backend, s3_endpoint, s3_bucket, s3_prefix, s3_region, rest_endpoint, \
                local_path, password_secret_id, s3_access_key_secret_id, s3_secret_key_secret_id, \
                rest_password_secret_id, maintenance_agent_id, initialized, created_at, updated_at, \
                created_by \
         FROM repositories WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(Repository::from))
}

/// Writes the mutable fields of a repository and sets `updated_at`.
///
/// The backend kind does not change. The caller passes the secret identifiers: it reuses the current
/// identifier when the value did not change. A missing repository returns `false`.
pub async fn update(
    tx: &mut TenantTx<'_>,
    id: RepositoryId,
    patch: &RepositoryUpdate<'_>,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE repositories SET name = $2, s3_endpoint = $3, s3_bucket = $4, s3_prefix = $5, \
                s3_region = $6, rest_endpoint = $7, local_path = $8, password_secret_id = $9, \
                s3_access_key_secret_id = $10, s3_secret_key_secret_id = $11, \
                rest_password_secret_id = $12, maintenance_agent_id = $13, updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(patch.name)
    .bind(patch.settings.s3_endpoint)
    .bind(patch.settings.s3_bucket)
    .bind(patch.settings.s3_prefix)
    .bind(patch.settings.s3_region)
    .bind(patch.settings.rest_endpoint)
    .bind(patch.settings.local_path)
    .bind(patch.secrets.password)
    .bind(patch.secrets.s3_access_key)
    .bind(patch.secrets.s3_secret_key)
    .bind(patch.secrets.rest_password)
    .bind(patch.maintenance_agent_id)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Removes a repository of the tenant. `true` when it removed the row.
pub async fn delete(tx: &mut TenantTx<'_>, id: RepositoryId) -> Result<bool, DbError> {
    let result = sqlx::query("DELETE FROM repositories WHERE id = $1")
        .bind(id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// True when a repository of the tenant binds the secret as its password.
///
/// The password of a repository cannot change until the rotation of FR-RP-06 (1.0).
pub async fn is_password_secret(tx: &mut TenantTx<'_>, secret: SecretId) -> Result<bool, DbError> {
    let (bound,): (bool,) =
        sqlx::query_as("SELECT EXISTS (SELECT 1 FROM repositories WHERE password_secret_id = $1)")
            .bind(secret)
            .fetch_one(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(bound)
}

#[derive(sqlx::FromRow)]
struct RepositoryRow {
    id: Uuid,
    name: String,
    backend: BackendKind,
    s3_endpoint: Option<String>,
    s3_bucket: Option<String>,
    s3_prefix: Option<String>,
    s3_region: Option<String>,
    rest_endpoint: Option<String>,
    local_path: Option<String>,
    password_secret_id: Uuid,
    s3_access_key_secret_id: Option<Uuid>,
    s3_secret_key_secret_id: Option<Uuid>,
    rest_password_secret_id: Option<Uuid>,
    maintenance_agent_id: Option<Uuid>,
    initialized: bool,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    created_by: Option<Uuid>,
}

impl From<RepositoryRow> for Repository {
    fn from(row: RepositoryRow) -> Self {
        Self {
            id: RepositoryId::from_uuid(row.id),
            name: row.name,
            backend: row.backend,
            s3_endpoint: row.s3_endpoint,
            s3_bucket: row.s3_bucket,
            s3_prefix: row.s3_prefix,
            s3_region: row.s3_region,
            rest_endpoint: row.rest_endpoint,
            local_path: row.local_path,
            password_secret_id: SecretId::from_uuid(row.password_secret_id),
            s3_access_key_secret_id: row.s3_access_key_secret_id.map(SecretId::from_uuid),
            s3_secret_key_secret_id: row.s3_secret_key_secret_id.map(SecretId::from_uuid),
            rest_password_secret_id: row.rest_password_secret_id.map(SecretId::from_uuid),
            maintenance_agent_id: row.maintenance_agent_id.map(AgentId::from_uuid),
            initialized: row.initialized,
            created_at: row.created_at,
            updated_at: row.updated_at,
            created_by: row.created_by.map(UserId::from_uuid),
        }
    }
}
