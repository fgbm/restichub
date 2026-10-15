//! The encrypted secret values of a tenant (ADR-0017, ADR-0023, section Secrets).
//!
//! The repository stores the metadata and the ciphertext of a secret value. The metadata functions
//! ([`create`], [`replace`], [`list`], [`get`] and [`delete`]) return the columns `id`, `name`,
//! `purpose`, `created_at`, `updated_at` and `created_by` only. They never return the ciphertext
//! (AC-RP-02).
//!
//! The function [`crate::delivery::load_ciphertext_for_delivery`] returns the ciphertext. Only the
//! delivery of a secret to an agent uses it (AC-RP-03).
//!
//! A row belongs to one tenant. The general tenant policy of the `secrets` table shows the rows of
//! the tenant of the transaction only (ADR-0023, section Tenant isolation).

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::enums::SecretUsageKind;
use crate::ids::{SecretId, UserId};
use crate::tx::TenantTx;

/// The type never holds the ciphertext, so a response built from it cannot disclose the value
/// (AC-RP-02).
#[derive(Debug, Clone)]
pub struct SecretMetadata {
    pub id: SecretId,
    /// Unique for the tenant.
    pub name: String,
    /// Allowed values are owned by the `restichub-secrets` crate.
    pub purpose: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    /// Absent for an API token actor.
    pub created_by: Option<UserId>,
}

/// The caller encrypts the value and passes the ciphertext with the key version of the data
/// encryption key (DEK) that encrypted it. The identifier is generated before the encryption,
/// because it is part of the associated data (ADR-0023, section Secrets).
#[derive(Debug, Clone)]
pub struct NewSecret<'a> {
    pub id: SecretId,
    /// Unique for the tenant.
    pub name: &'a str,
    pub purpose: &'a str,
    pub key_version: i32,
    pub nonce: &'a [u8],
    pub ciphertext: &'a [u8],
    /// Absent for an API token actor.
    pub created_by: Option<UserId>,
}

pub async fn create(tx: &mut TenantTx<'_>, new: &NewSecret<'_>) -> Result<SecretId, DbError> {
    sqlx::query(
        "INSERT INTO secrets \
         (id, tenant_id, name, purpose, key_version, nonce, ciphertext, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(new.id)
    .bind(tx.tenant())
    .bind(new.name)
    .bind(new.purpose)
    .bind(new.key_version)
    .bind(new.nonce)
    .bind(new.ciphertext)
    .bind(new.created_by)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(new.id)
}

/// Replaces the value of a secret of the transaction.
///
/// The function writes a new nonce and a new ciphertext and sets `updated_at`. The function returns
/// true if it changed the secret. A missing secret returns false.
pub async fn replace(
    tx: &mut TenantTx<'_>,
    id: SecretId,
    key_version: i32,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE secrets SET key_version = $2, nonce = $3, ciphertext = $4, updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(key_version)
    .bind(nonce)
    .bind(ciphertext)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Lists the metadata of the secrets of the tenant of the transaction.
///
/// The function orders the secrets by name and identifier. It never returns the ciphertext.
pub async fn list(tx: &mut TenantTx<'_>) -> Result<Vec<SecretMetadata>, DbError> {
    let rows = sqlx::query_as::<_, MetadataRow>(
        "SELECT id, name, purpose, created_at, updated_at, created_by \
         FROM secrets ORDER BY name, id",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(SecretMetadata::from).collect())
}

/// Reads the metadata of one secret of the tenant of the transaction.
///
/// The function never returns the ciphertext. A secret of another tenant is invisible.
pub async fn get(tx: &mut TenantTx<'_>, id: SecretId) -> Result<Option<SecretMetadata>, DbError> {
    let row = sqlx::query_as::<_, MetadataRow>(
        "SELECT id, name, purpose, created_at, updated_at, created_by FROM secrets WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(SecretMetadata::from))
}

/// Reads the metadata of one secret and locks its row for the transaction.
///
/// A replace or a delete uses this function, so a concurrent change cannot remove the row between
/// the read and the write. The row lock holds until the transaction ends.
pub async fn get_for_update(
    tx: &mut TenantTx<'_>,
    id: SecretId,
) -> Result<Option<SecretMetadata>, DbError> {
    let row = sqlx::query_as::<_, MetadataRow>(
        "SELECT id, name, purpose, created_at, updated_at, created_by \
         FROM secrets WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(SecretMetadata::from))
}

/// Reads the metadata of one secret and takes a `FOR KEY SHARE` lock on its row.
///
/// A save of an object that references the secret uses this function. A concurrent delete of the
/// secret waits for the end of the transaction, so the delete sees the new reference and refuses.
pub async fn get_for_key_share(
    tx: &mut TenantTx<'_>,
    id: SecretId,
) -> Result<Option<SecretMetadata>, DbError> {
    let row = sqlx::query_as::<_, MetadataRow>(
        "SELECT id, name, purpose, created_at, updated_at, created_by \
         FROM secrets WHERE id = $1 FOR KEY SHARE",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(SecretMetadata::from))
}

/// True if it removed the secret, false for a missing secret.
pub async fn delete(tx: &mut TenantTx<'_>, id: SecretId) -> Result<bool, DbError> {
    let result = sqlx::query("DELETE FROM secrets WHERE id = $1")
        .bind(id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// One object that references a secret.
#[derive(Debug, Clone)]
pub struct SecretUsage {
    pub secret_id: SecretId,
    pub kind: SecretUsageKind,
    pub object_id: Uuid,
    pub object_name: String,
}

/// Lists the references of the objects of the tenant to its secrets, ordered by object name.
///
/// `only` limits the list to one secret. A repository and a channel name a secret by a foreign
/// key. A profile and a job template hold the identifier of a hook secret in a script variable of
/// the kind `secret_ref`, so the match is on that field only (migration 0046 and the script
/// validation keep the identifier in its canonical form).
pub async fn list_usage(
    tx: &mut TenantTx<'_>,
    only: Option<SecretId>,
) -> Result<Vec<SecretUsage>, DbError> {
    let rows = sqlx::query_as::<_, (Uuid, SecretUsageKind, Uuid, String)>(
        "SELECT s.id, 'repository'::text, r.id, r.name FROM secrets s \
           JOIN repositories r ON s.id IN (r.password_secret_id, r.s3_access_key_secret_id, \
                                           r.s3_secret_key_secret_id, r.rest_password_secret_id) \
         WHERE ($1::uuid IS NULL OR s.id = $1) \
         UNION ALL \
         SELECT s.id, 'channel'::text, c.id, c.name FROM secrets s \
           JOIN notification_channels c ON s.id IN (c.secret_id, c.url_secret_id) \
         WHERE ($1::uuid IS NULL OR s.id = $1) \
         UNION ALL \
         SELECT DISTINCT s.id, 'profile'::text, p.id, p.name FROM profiles p \
           CROSS JOIN LATERAL jsonb_path_query(p.scripts, \
                '$[*].env[*] ? (@.kind == \"secret_ref\").value') AS ref(value) \
           JOIN secrets s ON s.id::text = ref.value #>> '{}' \
         WHERE ($1::uuid IS NULL OR s.id = $1) \
         UNION ALL \
         SELECT DISTINCT s.id, 'job_template'::text, t.id, t.name FROM job_templates t \
           CROSS JOIN LATERAL jsonb_path_query(t.scripts, \
                '$[*].env[*] ? (@.kind == \"secret_ref\").value') AS ref(value) \
           JOIN secrets s ON s.id::text = ref.value #>> '{}' \
         WHERE ($1::uuid IS NULL OR s.id = $1) \
         ORDER BY 4, 3",
    )
    .bind(only)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(|(secret_id, kind, object_id, object_name)| SecretUsage {
            secret_id: SecretId::from_uuid(secret_id),
            kind,
            object_id,
            object_name,
        })
        .collect())
}

type MetadataRow = (
    Uuid,
    String,
    String,
    OffsetDateTime,
    OffsetDateTime,
    Option<Uuid>,
);

impl From<MetadataRow> for SecretMetadata {
    fn from(row: MetadataRow) -> Self {
        let (id, name, purpose, created_at, updated_at, created_by) = row;
        Self {
            id: SecretId::from_uuid(id),
            name,
            purpose,
            created_at,
            updated_at,
            created_by: created_by.map(UserId::from_uuid),
        }
    }
}
