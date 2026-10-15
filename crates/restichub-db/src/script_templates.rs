//! The script templates and the job templates of a tenant (FR-HK-12, ADR-0037).
//!
//! A script template holds a script body with declared parameters. A job template holds an
//! ordered set of scripts from which a profile is made. Both belong to one tenant; the control
//! plane adds its read-only built-in templates to the lists without a row.

use serde_json::Value;
use time::OffsetDateTime;

use crate::DbError;
use crate::ids::{JobTemplateId, ScriptTemplateId, UserId};
use crate::tx::TenantTx;

/// Per-tenant cap that keeps the settings page small.
pub const MAX_TEMPLATES: i64 = 200;

/// A script template row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ScriptTemplate {
    pub id: ScriptTemplateId,
    pub name: String,
    pub description: String,
    pub interpreter: String,
    pub phase: String,
    pub body: String,
    pub timeout_seconds: i32,
    pub failure_policy: String,
    /// An array of declared parameters.
    pub parameters: Value,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

const COLUMNS: &str = "id, name, description, interpreter, phase, body, timeout_seconds, \
                       failure_policy, parameters, created_at, updated_at";

/// The fields of a new script template row.
#[derive(Debug, Clone)]
pub struct NewScriptTemplate<'a> {
    pub id: ScriptTemplateId,
    pub name: &'a str,
    pub description: &'a str,
    pub interpreter: &'a str,
    pub phase: &'a str,
    pub body: &'a str,
    pub timeout_seconds: i32,
    pub failure_policy: &'a str,
    pub parameters: &'a Value,
    pub created_by: Option<UserId>,
}

/// The mutable fields of a script template row.
#[derive(Debug, Clone)]
pub struct ScriptTemplateUpdate<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub interpreter: &'a str,
    pub phase: &'a str,
    pub body: &'a str,
    pub timeout_seconds: i32,
    pub failure_policy: &'a str,
    pub parameters: &'a Value,
}

pub async fn create(tx: &mut TenantTx<'_>, new: &NewScriptTemplate<'_>) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO script_templates \
         (id, tenant_id, name, description, interpreter, phase, body, timeout_seconds, \
          failure_policy, parameters, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(new.id)
    .bind(tx.tenant())
    .bind(new.name)
    .bind(new.description)
    .bind(new.interpreter)
    .bind(new.phase)
    .bind(new.body)
    .bind(new.timeout_seconds)
    .bind(new.failure_policy)
    .bind(new.parameters)
    .bind(new.created_by)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

pub async fn count(tx: &mut TenantTx<'_>) -> Result<i64, DbError> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM script_templates")
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(count)
}

/// By name. The tenant holds at most [`MAX_TEMPLATES`].
pub async fn list(tx: &mut TenantTx<'_>) -> Result<Vec<ScriptTemplate>, DbError> {
    sqlx::query_as::<_, ScriptTemplate>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM script_templates ORDER BY name, id"
    )))
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

pub async fn get(
    tx: &mut TenantTx<'_>,
    id: ScriptTemplateId,
) -> Result<Option<ScriptTemplate>, DbError> {
    sqlx::query_as::<_, ScriptTemplate>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM script_templates WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// Reads a template and locks its row for the transaction.
pub async fn get_for_update(
    tx: &mut TenantTx<'_>,
    id: ScriptTemplateId,
) -> Result<Option<ScriptTemplate>, DbError> {
    sqlx::query_as::<_, ScriptTemplate>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM script_templates WHERE id = $1 FOR UPDATE"
    )))
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

pub async fn update(
    tx: &mut TenantTx<'_>,
    id: ScriptTemplateId,
    patch: &ScriptTemplateUpdate<'_>,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE script_templates SET name = $2, description = $3, interpreter = $4, phase = $5, \
                body = $6, timeout_seconds = $7, failure_policy = $8, parameters = $9, \
                updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(patch.name)
    .bind(patch.description)
    .bind(patch.interpreter)
    .bind(patch.phase)
    .bind(patch.body)
    .bind(patch.timeout_seconds)
    .bind(patch.failure_policy)
    .bind(patch.parameters)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

pub async fn delete(tx: &mut TenantTx<'_>, id: ScriptTemplateId) -> Result<bool, DbError> {
    let result = sqlx::query("DELETE FROM script_templates WHERE id = $1")
        .bind(id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// A job template row: a profile template with an ordered set of scripts.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct JobTemplate {
    pub id: JobTemplateId,
    pub name: String,
    pub description: String,
    /// An array of scripts, in the shape of `profiles.scripts`.
    pub scripts: Value,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

const JOB_COLUMNS: &str = "id, name, description, scripts, created_at, updated_at";

/// The fields of a new job template row.
#[derive(Debug, Clone)]
pub struct NewJobTemplate<'a> {
    pub id: JobTemplateId,
    pub name: &'a str,
    pub description: &'a str,
    pub scripts: &'a Value,
    pub created_by: Option<UserId>,
}

pub async fn create_job_template(
    tx: &mut TenantTx<'_>,
    new: &NewJobTemplate<'_>,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO job_templates (id, tenant_id, name, description, scripts, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(new.id)
    .bind(tx.tenant())
    .bind(new.name)
    .bind(new.description)
    .bind(new.scripts)
    .bind(new.created_by)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

pub async fn count_job_templates(tx: &mut TenantTx<'_>) -> Result<i64, DbError> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM job_templates")
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(count)
}

pub async fn list_job_templates(tx: &mut TenantTx<'_>) -> Result<Vec<JobTemplate>, DbError> {
    sqlx::query_as::<_, JobTemplate>(sqlx::AssertSqlSafe(format!(
        "SELECT {JOB_COLUMNS} FROM job_templates ORDER BY name, id"
    )))
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

pub async fn get_job_template(
    tx: &mut TenantTx<'_>,
    id: JobTemplateId,
) -> Result<Option<JobTemplate>, DbError> {
    sqlx::query_as::<_, JobTemplate>(sqlx::AssertSqlSafe(format!(
        "SELECT {JOB_COLUMNS} FROM job_templates WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// Reads a job template and locks its row for the transaction.
pub async fn get_job_template_for_update(
    tx: &mut TenantTx<'_>,
    id: JobTemplateId,
) -> Result<Option<JobTemplate>, DbError> {
    sqlx::query_as::<_, JobTemplate>(sqlx::AssertSqlSafe(format!(
        "SELECT {JOB_COLUMNS} FROM job_templates WHERE id = $1 FOR UPDATE"
    )))
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

pub async fn update_job_template(
    tx: &mut TenantTx<'_>,
    id: JobTemplateId,
    name: &str,
    description: &str,
    scripts: &Value,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE job_templates SET name = $2, description = $3, scripts = $4, updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(scripts)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

pub async fn delete_job_template(
    tx: &mut TenantTx<'_>,
    id: JobTemplateId,
) -> Result<bool, DbError> {
    let result = sqlx::query("DELETE FROM job_templates WHERE id = $1")
        .bind(id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}
