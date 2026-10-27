//! The notification channels of a tenant (FR-MN-04, ADR-0036).
//!
//! A channel is an email recipient list or a webhook URL. The table keeps the origin of a webhook
//! URL. The path and the query, which often hold a token, and the signing secret are rows of
//! `secrets` that the channel names through `url_secret_id` and `secret_id`; this module never
//! holds their values.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::DbError;
use crate::ids::{ChannelId, SecretId, UserId};
use crate::tx::TenantTx;

/// Per-tenant cap that keeps the settings page and the fan-out small.
pub const MAX_CHANNELS: i64 = 50;

/// Tells how a notification reaches a person.
// The string value of each variant is stored: do not change a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum ChannelKind {
    Email,
    Webhook,
}

impl ChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Webhook => "webhook",
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Channel {
    pub id: ChannelId,
    pub name: String,
    pub kind: ChannelKind,
    pub enabled: bool,
    pub recipients: Vec<String>,
    /// The scheme, the host and the port of a webhook URL.
    pub url_origin: Option<String>,
    pub secret_id: Option<SecretId>,
    /// The encrypted path and query of a webhook URL.
    pub url_secret_id: Option<SecretId>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    /// The end of the last delivery attempt of an alert. A test send leaves no row.
    pub last_delivery_at: Option<OffsetDateTime>,
    pub last_delivery_ok: Option<bool>,
}

// `alert_deliveries_channel_attempt_idx` (0050) serves both subqueries.
macro_rules! columns {
    () => {
        "id, name, kind, enabled, recipients, url_origin, secret_id, url_secret_id, \
         created_at, updated_at, \
         (SELECT d.attempted_at FROM alert_deliveries d \
          WHERE d.tenant_id = notification_channels.tenant_id \
            AND d.channel_id = notification_channels.id AND d.attempted_at IS NOT NULL \
          ORDER BY d.attempted_at DESC, d.id DESC LIMIT 1) AS last_delivery_at, \
         (SELECT d.status = 'sent' FROM alert_deliveries d \
          WHERE d.tenant_id = notification_channels.tenant_id \
            AND d.channel_id = notification_channels.id AND d.attempted_at IS NOT NULL \
          ORDER BY d.attempted_at DESC, d.id DESC LIMIT 1) AS last_delivery_ok"
    };
}

#[derive(Debug, Clone)]
pub struct NewChannel<'a> {
    pub id: ChannelId,
    pub name: &'a str,
    pub kind: ChannelKind,
    pub enabled: bool,
    pub recipients: &'a [String],
    pub url_origin: Option<&'a str>,
    pub secret_id: Option<SecretId>,
    pub url_secret_id: Option<SecretId>,
    pub created_by: Option<UserId>,
}

#[derive(Debug, Clone)]
pub struct ChannelUpdate<'a> {
    pub name: &'a str,
    pub enabled: bool,
    pub recipients: &'a [String],
    pub url_origin: Option<&'a str>,
}

pub async fn create(tx: &mut TenantTx<'_>, new: &NewChannel<'_>) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO notification_channels \
         (id, tenant_id, name, kind, enabled, recipients, url_origin, secret_id, url_secret_id, \
          created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(new.id)
    .bind(tx.tenant())
    .bind(new.name)
    .bind(new.kind.as_str())
    .bind(new.enabled)
    .bind(new.recipients)
    .bind(new.url_origin)
    .bind(new.secret_id)
    .bind(new.url_secret_id)
    .bind(new.created_by)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

pub async fn count(tx: &mut TenantTx<'_>) -> Result<i64, DbError> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM notification_channels")
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(count)
}

/// Oldest first. The tenant holds at most [`MAX_CHANNELS`].
pub async fn list(tx: &mut TenantTx<'_>) -> Result<Vec<Channel>, DbError> {
    sqlx::query_as::<_, Channel>(concat!(
        "SELECT ",
        columns!(),
        " FROM notification_channels ORDER BY created_at, id"
    ))
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

pub async fn get(tx: &mut TenantTx<'_>, id: ChannelId) -> Result<Option<Channel>, DbError> {
    sqlx::query_as::<_, Channel>(concat!(
        "SELECT ",
        columns!(),
        " FROM notification_channels WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// Reads a channel and locks its row for the transaction.
pub async fn get_for_update(
    tx: &mut TenantTx<'_>,
    id: ChannelId,
) -> Result<Option<Channel>, DbError> {
    sqlx::query_as::<_, Channel>(concat!(
        "SELECT ",
        columns!(),
        " FROM notification_channels WHERE id = $1 FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

pub async fn enabled_ids(tx: &mut TenantTx<'_>) -> Result<Vec<ChannelId>, DbError> {
    sqlx::query_scalar::<_, ChannelId>(
        "SELECT id FROM notification_channels WHERE enabled ORDER BY created_at, id",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

pub async fn update(
    tx: &mut TenantTx<'_>,
    id: ChannelId,
    patch: &ChannelUpdate<'_>,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE notification_channels \
         SET name = $2, enabled = $3, recipients = $4, url_origin = $5, updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(patch.name)
    .bind(patch.enabled)
    .bind(patch.recipients)
    .bind(patch.url_origin)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Removes a channel. The pending deliveries of the channel go with it.
pub async fn delete(tx: &mut TenantTx<'_>, id: ChannelId) -> Result<bool, DbError> {
    let result = sqlx::query("DELETE FROM notification_channels WHERE id = $1")
        .bind(id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}
