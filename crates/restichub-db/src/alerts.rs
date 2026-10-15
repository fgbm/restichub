//! The alert rules, the alert state and the delivery outbox (FR-MN-03, FR-MN-04, ADR-0035).
//!
//! Every function runs under the tenant policy, so a transaction sees the rows of its own tenant
//! only. The partial unique index on `alert_state` makes the open of an alert idempotent. The
//! outbox rows commit with the state change that created them, and `claim_due` locks them with
//! `SKIP LOCKED`, so two control plane instances never claim the same row.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::DbError;
use crate::channels::{self, Channel};
use crate::ids::{AgentId, AlertId, ChannelId, DeliveryId, JobId, PlanId, RepositoryId};
use crate::tx::TenantTx;

pub const DEFAULT_PAGE_LIMIT: i64 = 50;

pub const MAX_PAGE_LIMIT: i64 = 200;

/// The largest number of open alerts that one evaluation reads for a tenant.
pub const MAX_OPEN_ALERTS: i64 = 5000;

/// The largest number of pending deliveries of a tenant. A newer delivery beyond it is stored as
/// failed with the error `queue_full`, so the outbox stays bounded (ADR-0035).
pub const MAX_PENDING_DELIVERIES: i64 = 1000;

/// The kind of an alert. The string value of each variant is stored: do not change a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum AlertKind {
    /// No successful backup of a plan within its recovery point objective.
    RpoViolated,
    /// A plan, repository or agent whose newest finished job failed.
    JobFailed,
    /// The latest finished job of a subject is in the dead letter state.
    JobDeadLetter,
    /// The latest finished job of a subject succeeded with warnings.
    JobWarnings,
    /// An agent was not seen for longer than the threshold.
    AgentOffline,
    /// A job waits on a stale repository lock for longer than the threshold.
    StaleLock,
}

impl AlertKind {
    pub const ALL: [AlertKind; 6] = [
        Self::RpoViolated,
        Self::JobFailed,
        Self::JobDeadLetter,
        Self::JobWarnings,
        Self::AgentOffline,
        Self::StaleLock,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::RpoViolated => "rpo_violated",
            Self::JobFailed => "job_failed",
            Self::JobDeadLetter => "job_dead_letter",
            Self::JobWarnings => "job_warnings",
            Self::AgentOffline => "agent_offline",
            Self::StaleLock => "stale_lock",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    /// True for the kinds that carry a threshold in minutes.
    pub fn has_threshold(self) -> bool {
        matches!(self, Self::AgentOffline | Self::StaleLock)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum AlertStatus {
    Open,
    Resolved,
}

impl AlertStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
        }
    }
}

/// What a delivery tells the receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum DeliveryEvent {
    Opened,
    Resolved,
}

impl DeliveryEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Opened => "opened",
            Self::Resolved => "resolved",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum DeliveryStatus {
    Pending,
    Sent,
    Failed,
}

impl DeliveryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Sent => "sent",
            Self::Failed => "failed",
        }
    }
}

/// The setting of one alert kind for a tenant (ADR-0035).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertRule {
    pub kind: AlertKind,
    pub enabled: bool,
    /// Minutes. Set for the kinds that have a threshold.
    pub threshold_minutes: Option<i32>,
}

/// The default threshold of the agent offline alert, in minutes (FR-MN-03).
pub const DEFAULT_OFFLINE_MINUTES: i32 = 30;

/// The default threshold of the stale lock alert, in minutes (FR-MN-03).
pub const DEFAULT_STALE_LOCK_MINUTES: i32 = 60;

impl AlertRule {
    /// Defaults apply to a kind with no stored row: on, except job warnings.
    pub fn default_for(kind: AlertKind) -> Self {
        Self {
            kind,
            enabled: kind != AlertKind::JobWarnings,
            threshold_minutes: match kind {
                AlertKind::AgentOffline => Some(DEFAULT_OFFLINE_MINUTES),
                AlertKind::StaleLock => Some(DEFAULT_STALE_LOCK_MINUTES),
                _ => None,
            },
        }
    }
}

/// Reads the rules of the tenant, one for each kind, in the order of [`AlertKind::ALL`]. A kind
/// without a stored row gets the default.
pub async fn list_rules(tx: &mut TenantTx<'_>) -> Result<Vec<AlertRule>, DbError> {
    let rows = sqlx::query_as::<_, (String, bool, Option<i32>)>(
        "SELECT kind, enabled, threshold_minutes FROM alert_rules",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(AlertKind::ALL
        .into_iter()
        .map(|kind| {
            rows.iter()
                .find(|(name, _, _)| name == kind.as_str())
                .map_or_else(
                    || AlertRule::default_for(kind),
                    |(_, enabled, threshold)| AlertRule {
                        kind,
                        enabled: *enabled,
                        threshold_minutes: *threshold,
                    },
                )
        })
        .collect())
}

pub async fn upsert_rule(tx: &mut TenantTx<'_>, rule: &AlertRule) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO alert_rules (tenant_id, kind, enabled, threshold_minutes) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (tenant_id, kind) DO UPDATE \
         SET enabled = EXCLUDED.enabled, threshold_minutes = EXCLUDED.threshold_minutes, \
             updated_at = now()",
    )
    .bind(tx.tenant())
    .bind(rule.kind.as_str())
    .bind(rule.enabled)
    .bind(rule.threshold_minutes)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

/// Row of `alert_state`. The display names are copies.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Alert {
    pub id: AlertId,
    pub kind: AlertKind,
    pub dedupe_key: String,
    pub status: AlertStatus,
    pub agent_id: Option<AgentId>,
    pub plan_id: Option<PlanId>,
    pub repository_id: Option<RepositoryId>,
    pub job_id: Option<JobId>,
    pub agent_hostname: Option<String>,
    pub plan_name: Option<String>,
    pub repository_name: Option<String>,
    pub detail: String,
    pub opened_at: OffsetDateTime,
    pub resolved_at: Option<OffsetDateTime>,
}

const ALERT_COLUMNS: &str = "id, kind, dedupe_key, status, agent_id, plan_id, repository_id, \
                             job_id, agent_hostname, plan_name, repository_name, detail, \
                             opened_at, resolved_at";

#[derive(Debug, Clone)]
pub struct NewAlert<'a> {
    pub id: AlertId,
    pub kind: AlertKind,
    pub dedupe_key: &'a str,
    pub agent_id: Option<AgentId>,
    pub plan_id: Option<PlanId>,
    pub repository_id: Option<RepositoryId>,
    pub job_id: Option<JobId>,
    pub agent_hostname: Option<&'a str>,
    pub plan_name: Option<&'a str>,
    pub repository_name: Option<&'a str>,
    pub detail: &'a str,
}

/// Takes the evaluation lock of the tenant for the transaction. `false` means another instance
/// holds it, so the caller skips the pass.
pub async fn try_lock_tenant(tx: &mut TenantTx<'_>) -> Result<bool, DbError> {
    let (locked,): (bool,) =
        sqlx::query_as("SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("restichub-alerts:{}", tx.tenant()))
            .fetch_one(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(locked)
}

/// Feeds the evaluation pass: at most [`MAX_OPEN_ALERTS`] rows.
pub async fn open_alerts(tx: &mut TenantTx<'_>) -> Result<Vec<Alert>, DbError> {
    sqlx::query_as::<_, Alert>(sqlx::AssertSqlSafe(format!(
        "SELECT {ALERT_COLUMNS} FROM alert_state WHERE status = 'open' \
         ORDER BY opened_at, id LIMIT {MAX_OPEN_ALERTS}"
    )))
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// Opens an alert. A second open alert with the same kind and key is a unique violation.
pub async fn insert_open(
    tx: &mut TenantTx<'_>,
    new: &NewAlert<'_>,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO alert_state \
         (id, tenant_id, kind, dedupe_key, status, agent_id, plan_id, repository_id, job_id, \
          agent_hostname, plan_name, repository_name, detail, opened_at) \
         VALUES ($1, $2, $3, $4, 'open', $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(new.id)
    .bind(tx.tenant())
    .bind(new.kind.as_str())
    .bind(new.dedupe_key)
    .bind(new.agent_id)
    .bind(new.plan_id)
    .bind(new.repository_id)
    .bind(new.job_id)
    .bind(new.agent_hostname)
    .bind(new.plan_name)
    .bind(new.repository_name)
    .bind(new.detail)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

/// Resolves an open alert. `false` means the alert was not open.
pub async fn resolve(
    tx: &mut TenantTx<'_>,
    id: AlertId,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE alert_state SET status = 'resolved', resolved_at = $2 \
         WHERE id = $1 AND status = 'open'",
    )
    .bind(id)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

pub async fn get(tx: &mut TenantTx<'_>, id: AlertId) -> Result<Option<Alert>, DbError> {
    sqlx::query_as::<_, Alert>(sqlx::AssertSqlSafe(format!(
        "SELECT {ALERT_COLUMNS} FROM alert_state WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// The keyset cursor of the alert list. It orders by `(opened_at, id)`, newest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertCursor {
    pub opened_at: OffsetDateTime,
    pub id: AlertId,
}

#[derive(Debug, Clone, Copy)]
pub struct AlertPage {
    /// Clamped to [`MAX_PAGE_LIMIT`].
    pub limit: i64,
    pub after: Option<AlertCursor>,
    /// `None` lists both states.
    pub status: Option<AlertStatus>,
}

#[derive(Debug, Clone)]
pub struct AlertPageResult {
    pub alerts: Vec<Alert>,
    pub has_more: bool,
}

pub async fn list(tx: &mut TenantTx<'_>, page: &AlertPage) -> Result<AlertPageResult, DbError> {
    let limit = page.limit.clamp(1, MAX_PAGE_LIMIT);
    let (cursor_time, cursor_id) = match page.after {
        Some(cursor) => (Some(cursor.opened_at), Some(cursor.id)),
        None => (None, None),
    };
    let mut alerts = sqlx::query_as::<_, Alert>(sqlx::AssertSqlSafe(format!(
        "SELECT {ALERT_COLUMNS} FROM alert_state \
         WHERE ($1::text IS NULL OR status = $1) \
           AND ($2::timestamptz IS NULL OR (opened_at, id) < ($2::timestamptz, $3::uuid)) \
         ORDER BY opened_at DESC, id DESC LIMIT $4"
    )))
    .bind(page.status.map(AlertStatus::as_str))
    .bind(cursor_time)
    .bind(cursor_id)
    .bind(limit + 1)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let has_more = alerts.len() as i64 > limit;
    alerts.truncate(limit as usize);
    Ok(AlertPageResult { alerts, has_more })
}

/// Removes the resolved alerts that resolved before `before`. Their deliveries go with them.
pub async fn purge_resolved(tx: &mut TenantTx<'_>, before: OffsetDateTime) -> Result<u64, DbError> {
    let result =
        sqlx::query("DELETE FROM alert_state WHERE status = 'resolved' AND resolved_at < $1")
            .bind(before)
            .execute(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}

/// Queues one delivery of the alert event for each enabled channel and returns the number of
/// queued rows. A delivery beyond [`MAX_PENDING_DELIVERIES`] is stored as failed.
pub async fn enqueue(
    tx: &mut TenantTx<'_>,
    alert_id: AlertId,
    event: DeliveryEvent,
    now: OffsetDateTime,
) -> Result<usize, DbError> {
    let channel_ids = channels::enabled_ids(tx).await?;
    if channel_ids.is_empty() {
        return Ok(0);
    }
    let (mut pending,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM alert_deliveries WHERE status = 'pending'")
            .fetch_one(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    for channel_id in &channel_ids {
        let full = pending >= MAX_PENDING_DELIVERIES;
        sqlx::query(
            "INSERT INTO alert_deliveries \
             (id, tenant_id, alert_id, channel_id, event, status, next_attempt_at, last_error, \
              created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $7)",
        )
        .bind(DeliveryId::new())
        .bind(tx.tenant())
        .bind(alert_id)
        .bind(channel_id)
        .bind(event.as_str())
        .bind(if full {
            DeliveryStatus::Failed.as_str()
        } else {
            DeliveryStatus::Pending.as_str()
        })
        .bind(now)
        .bind(full.then_some("queue_full"))
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
        if !full {
            pending += 1;
        }
    }
    Ok(channel_ids.len())
}

#[derive(Debug, Clone)]
pub struct ClaimedDelivery {
    pub id: DeliveryId,
    pub event: DeliveryEvent,
    /// The number of the attempt, counted from one, including this one.
    pub attempt: i32,
    pub alert: Alert,
    pub channel: Channel,
}

#[derive(sqlx::FromRow)]
struct ClaimRow {
    id: DeliveryId,
    alert_id: AlertId,
    channel_id: ChannelId,
    event: DeliveryEvent,
    attempts: i32,
}

/// Claims the due deliveries of the tenant and moves their next attempt to `lease_until`.
///
/// The claim is a lease: a sender that crashes before it reports the result leaves the row due
/// again at `lease_until`, so a delivery is at-least-once (ADR-0035).
pub async fn claim_due(
    tx: &mut TenantTx<'_>,
    now: OffsetDateTime,
    lease_until: OffsetDateTime,
    limit: i64,
) -> Result<Vec<ClaimedDelivery>, DbError> {
    let rows = sqlx::query_as::<_, ClaimRow>(
        "WITH due AS ( \
             SELECT id FROM alert_deliveries \
             WHERE status = 'pending' AND next_attempt_at <= $1 \
             ORDER BY next_attempt_at, id LIMIT $3 FOR UPDATE SKIP LOCKED) \
         UPDATE alert_deliveries d SET attempts = d.attempts + 1, next_attempt_at = $2 \
         FROM due WHERE d.id = due.id \
         RETURNING d.id, d.alert_id, d.channel_id, d.event, d.attempts",
    )
    .bind(now)
    .bind(lease_until)
    .bind(limit)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let mut claimed = Vec::with_capacity(rows.len());
    for row in rows {
        let alert = get(tx, row.alert_id).await?;
        let channel = channels::get(tx, row.channel_id).await?;
        if let (Some(alert), Some(channel)) = (alert, channel) {
            claimed.push(ClaimedDelivery {
                id: row.id,
                event: row.event,
                attempt: row.attempts,
                alert,
                channel,
            });
        }
    }
    Ok(claimed)
}

pub async fn mark_sent(
    tx: &mut TenantTx<'_>,
    id: DeliveryId,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE alert_deliveries SET status = 'sent', sent_at = $2, last_error = NULL \
         WHERE id = $1",
    )
    .bind(id)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

/// Records a failed attempt. `retry_at` set keeps the delivery pending for that time; `None` ends
/// it as failed.
pub async fn mark_attempt_failed(
    tx: &mut TenantTx<'_>,
    id: DeliveryId,
    error: &str,
    retry_at: Option<OffsetDateTime>,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE alert_deliveries \
         SET status = CASE WHEN $3::timestamptz IS NULL THEN 'failed' ELSE 'pending' END, \
             next_attempt_at = COALESCE($3, next_attempt_at), last_error = $2 \
         WHERE id = $1",
    )
    .bind(id)
    .bind(error)
    .bind(retry_at)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

/// Removes the finished deliveries that were created before `before`.
pub async fn purge_finished_deliveries(
    tx: &mut TenantTx<'_>,
    before: OffsetDateTime,
) -> Result<u64, DbError> {
    let result =
        sqlx::query("DELETE FROM alert_deliveries WHERE status <> 'pending' AND created_at < $1")
            .bind(before)
            .execute(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}

/// The number of deliveries in each status, for the tests and the operator.
pub async fn delivery_counts(tx: &mut TenantTx<'_>) -> Result<Vec<(DeliveryStatus, i64)>, DbError> {
    sqlx::query_as::<_, (DeliveryStatus, i64)>(
        "SELECT status, count(*) FROM alert_deliveries GROUP BY status ORDER BY status",
    )
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}
