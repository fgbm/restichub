//! The parameter values of a host and of a plan (FR-PF-10, FR-PF-11, ADR-0047).
//!
//! A profile declares the parameters; a host holds a value for each name and a plan overrides the
//! value of the host. The declarations live in the profile JSON, because a parameter belongs to the
//! profile that uses it in a template or in a script variable. This module reads and writes the
//! values only.
//!
//! A `secret` value is an identifier of a secret of the purpose `hook_secret`; the row holds no
//! ciphertext (ADR-0017). The foreign keys refuse the delete of a secret that a value names.
//!
//! The general tenant policies of the two tables show the rows of the tenant of the transaction
//! only (ADR-0023, section Tenant isolation).

use uuid::Uuid;

use crate::DbError;
use crate::enums::ParameterKind;
use crate::ids::{AgentId, PlanId, ProfileId, SecretId, UserId};
use crate::tx::TenantTx;

/// One value of a parameter: the name, the kind and the value in the column of that kind.
///
/// The database refuses a row that holds both columns or neither, so the type does not carry a
/// second validity rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterValue {
    pub name: String,
    pub kind: ParameterKind,
    pub value: Option<String>,
    pub secret_id: Option<SecretId>,
}

/// Lists the values of a host, by name.
pub async fn agent_values(
    tx: &mut TenantTx<'_>,
    agent_id: AgentId,
) -> Result<Vec<ParameterValue>, DbError> {
    let rows = sqlx::query_as::<_, ValueRow>(
        "SELECT name, kind, value, secret_id FROM agent_parameter_values \
         WHERE agent_id = $1 ORDER BY name",
    )
    .bind(agent_id)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(ParameterValue::from).collect())
}

/// Replaces the whole set of the values of a host.
pub async fn replace_agent_values(
    tx: &mut TenantTx<'_>,
    agent_id: AgentId,
    values: &[ParameterValue],
    updated_by: Option<UserId>,
) -> Result<(), DbError> {
    sqlx::query("DELETE FROM agent_parameter_values WHERE agent_id = $1")
        .bind(agent_id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    for value in values {
        sqlx::query(
            "INSERT INTO agent_parameter_values \
             (tenant_id, agent_id, name, kind, value, secret_id, updated_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(tx.tenant())
        .bind(agent_id)
        .bind(&value.name)
        .bind(value.kind)
        .bind(&value.value)
        .bind(value.secret_id)
        .bind(updated_by)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    }
    Ok(())
}

/// One override with the name of its secret, for the plan view (FR-PF-10, FR-PF-12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanValue {
    pub plan_id: PlanId,
    pub value: ParameterValue,
    /// The name of the secret of a `secret` value, which is not a secret.
    pub secret_name: Option<String>,
}

/// Lists the overrides of a set of plans, in the order of the plan and of the name.
pub async fn values_of_plans(
    tx: &mut TenantTx<'_>,
    plans: &[PlanId],
) -> Result<Vec<PlanValue>, DbError> {
    let ids: Vec<Uuid> = plans.iter().map(|id| id.as_uuid()).collect();
    let rows = sqlx::query_as::<_, PlanValueNameRow>(
        "SELECT v.plan_id, v.name, v.kind, v.value, v.secret_id, s.name AS secret_name \
         FROM plan_parameter_values v \
           LEFT JOIN secrets s ON s.tenant_id = v.tenant_id AND s.id = v.secret_id \
         WHERE v.plan_id = ANY($1) ORDER BY v.plan_id, v.name",
    )
    .bind(&ids)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(|row| PlanValue {
            plan_id: PlanId::from_uuid(row.plan_id),
            value: ParameterValue {
                name: row.name,
                kind: ParameterKind::parse(&row.kind).unwrap_or(ParameterKind::Text),
                value: row.value,
                secret_id: row.secret_id.map(SecretId::from_uuid),
            },
            secret_name: row.secret_name,
        })
        .collect())
}

/// Lists the overrides of a plan, by name.
pub async fn plan_values(
    tx: &mut TenantTx<'_>,
    plan_id: PlanId,
) -> Result<Vec<ParameterValue>, DbError> {
    let rows = sqlx::query_as::<_, ValueRow>(
        "SELECT name, kind, value, secret_id FROM plan_parameter_values \
         WHERE plan_id = $1 ORDER BY name",
    )
    .bind(plan_id)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(ParameterValue::from).collect())
}

/// Replaces the whole set of the overrides of a plan.
pub async fn replace_plan_values(
    tx: &mut TenantTx<'_>,
    plan_id: PlanId,
    values: &[ParameterValue],
    updated_by: Option<UserId>,
) -> Result<(), DbError> {
    sqlx::query("DELETE FROM plan_parameter_values WHERE plan_id = $1")
        .bind(plan_id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    for value in values {
        sqlx::query(
            "INSERT INTO plan_parameter_values \
             (tenant_id, plan_id, name, kind, value, secret_id, updated_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(tx.tenant())
        .bind(plan_id)
        .bind(&value.name)
        .bind(value.kind)
        .bind(&value.value)
        .bind(value.secret_id)
        .bind(updated_by)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    }
    Ok(())
}

/// The overrides of every plan of one profile, with the plan that holds each row.
pub async fn values_of_profile_plans(
    tx: &mut TenantTx<'_>,
    profile_id: ProfileId,
) -> Result<Vec<(PlanId, ParameterValue)>, DbError> {
    let rows = sqlx::query_as::<_, PlanValueRow>(
        "SELECT v.plan_id, v.name, v.kind, v.value, v.secret_id FROM plans p \
           JOIN plan_parameter_values v ON v.tenant_id = p.tenant_id AND v.plan_id = p.id \
         WHERE p.profile_id = $1 ORDER BY p.created_at, p.id, v.name",
    )
    .bind(profile_id)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let plan = PlanId::from_uuid(row.plan_id);
            (
                plan,
                ParameterValue {
                    name: row.name,
                    kind: ParameterKind::parse(&row.kind).unwrap_or(ParameterKind::Text),
                    value: row.value,
                    secret_id: row.secret_id.map(SecretId::from_uuid),
                },
            )
        })
        .collect())
}

/// Removes the overrides of one name from every plan of a profile.
///
/// A parameter that leaves a profile takes its overrides with it: the overrides name a parameter
/// that no template of the profile reads any more (ADR-0047). The values of the hosts stay.
pub async fn delete_profile_name(
    tx: &mut TenantTx<'_>,
    profile_id: ProfileId,
    name: &str,
) -> Result<u64, DbError> {
    let result = sqlx::query(
        "DELETE FROM plan_parameter_values v USING plans p \
         WHERE v.tenant_id = p.tenant_id AND v.plan_id = p.id \
           AND p.profile_id = $1 AND v.name = $2",
    )
    .bind(profile_id)
    .bind(name)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}

/// What [`move_agent_values`] did with the values of the old record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedValues {
    /// Values that the successor holds now.
    pub moved: Vec<ParameterValue>,
    /// The rows of the old record that the successor already held by name, so the move dropped them.
    pub dropped: Vec<ParameterValue>,
}

/// Moves the values of a replaced host record to its successor (FR-ID-11).
///
/// A value that the successor already holds wins: the record that the operator keeps has the newer
/// value. The caller runs the move in the transaction of the replacement, and gets the dropped rows
/// for the audit, because the delete removes them (FR-PF-12).
pub async fn move_agent_values(
    tx: &mut TenantTx<'_>,
    from: AgentId,
    to: AgentId,
) -> Result<MovedValues, DbError> {
    let moved = sqlx::query_as::<_, ValueRow>(
        "INSERT INTO agent_parameter_values \
         (tenant_id, agent_id, name, kind, value, secret_id, updated_at, updated_by) \
         SELECT tenant_id, $1, name, kind, value, secret_id, updated_at, updated_by \
         FROM agent_parameter_values WHERE agent_id = $2 \
         ON CONFLICT (tenant_id, agent_id, name) DO NOTHING \
         RETURNING name, kind, value, secret_id",
    )
    .bind(to)
    .bind(from)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    // The delete removes the rows that came to the successor too, so a row whose name the insert
    // copied stays a moved value and only the others are dropped.
    let copied: Vec<String> = moved.iter().map(|row| row.name.clone()).collect();
    let remaining = sqlx::query_as::<_, ValueRow>(
        "DELETE FROM agent_parameter_values WHERE agent_id = $1 \
         RETURNING name, kind, value, secret_id",
    )
    .bind(from)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let dropped: Vec<ParameterValue> = remaining
        .into_iter()
        .filter(|row| !copied.contains(&row.name))
        .map(ParameterValue::from)
        .collect();
    Ok(MovedValues {
        moved: moved.into_iter().map(ParameterValue::from).collect(),
        dropped,
    })
}

#[derive(sqlx::FromRow)]
struct ValueRow {
    name: String,
    kind: String,
    value: Option<String>,
    secret_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct PlanValueNameRow {
    plan_id: Uuid,
    name: String,
    kind: String,
    value: Option<String>,
    secret_id: Option<Uuid>,
    secret_name: Option<String>,
}

#[derive(sqlx::FromRow)]
struct PlanValueRow {
    plan_id: Uuid,
    name: String,
    kind: String,
    value: Option<String>,
    secret_id: Option<Uuid>,
}

impl From<ValueRow> for ParameterValue {
    fn from(row: ValueRow) -> Self {
        Self {
            name: row.name,
            kind: ParameterKind::parse(&row.kind).unwrap_or(ParameterKind::Text),
            value: row.value,
            secret_id: row.secret_id.map(SecretId::from_uuid),
        }
    }
}
