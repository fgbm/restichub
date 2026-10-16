//! The alert rules, the alert state and the delivery outbox (FR-MN-03, FR-MN-04, ADR-0036).
//!
//! Every function runs under the tenant policy, so a transaction sees the rows of its own tenant
//! only. The partial unique index on `alert_state` makes the open of an alert idempotent. The
//! outbox rows commit with the state change that created them, and `claim_due` locks them with
//! `SKIP LOCKED`, so two control plane instances never claim the same row.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::DbError;
use crate::agent_search::search_pattern;
use crate::channels::{self, Channel};
use crate::enums::JobKind;
use crate::ids::{AgentId, AlertId, ChannelId, DeliveryId, JobId, PlanId, RepositoryId};
use crate::job_search::COUNT_LIMIT;
use crate::tx::TenantTx;

pub const DEFAULT_PAGE_LIMIT: i64 = 50;

pub const MAX_PAGE_LIMIT: i64 = 200;

/// The rows that one read of the open alerts takes at a time.
const OPEN_PAGE: i64 = 1000;

/// The largest number of pending deliveries of a tenant. A newer delivery beyond it is stored as
/// failed with the error `queue_full`, so the outbox stays bounded (ADR-0036).
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

/// The setting of one alert kind for a tenant (ADR-0036).
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

/// The typed values of an alert condition. The interface and the messages build the text of the
/// condition from them, in the language of the reader, so the stored alert holds no sentence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AlertParams {
    /// Seconds since the last success of a plan, since the last contact with an agent, or since
    /// a repository lock became stale. For a plan that never succeeded: since its creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_seconds: Option<i64>,
    /// The recovery point objective of a plan, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpo_seconds: Option<i64>,
    /// The finish time of the last succeeded backup, in RFC 3339. Absent when the plan never
    /// succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<String>,
    /// The threshold of the rule at the time of the alert, in minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_minutes: Option<i64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_job_kind"
    )]
    pub job_kind: Option<JobKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

/// A stored row outlives the code that wrote it, so a kind that this version does not know reads
/// as [`JobKind::Other`] and never stops the evaluation of a tenant.
fn lenient_job_kind<'de, D>(deserializer: D) -> Result<Option<JobKind>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.map(|value| serde_json::from_value(value).unwrap_or(JobKind::Other)))
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
    pub params: sqlx::types::Json<AlertParams>,
    /// The scrubbed and bounded error text of a job. Empty when the condition has none.
    pub detail: String,
    pub opened_at: OffsetDateTime,
    pub resolved_at: Option<OffsetDateTime>,
}

macro_rules! alert_columns {
    () => {
        "id, kind, dedupe_key, status, agent_id, plan_id, repository_id, \
                             job_id, agent_hostname, plan_name, repository_name, params, \
                             detail, opened_at, resolved_at"
    };
}

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
    pub params: &'a AlertParams,
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

/// Reads one page of the open alerts in the order `(opened_at, id)`. `after` is the last row of the
/// previous page.
pub async fn open_alerts_page(
    tx: &mut TenantTx<'_>,
    after: Option<AlertCursor>,
    limit: i64,
) -> Result<Vec<Alert>, DbError> {
    let (cursor_time, cursor_id) = match after {
        Some(cursor) => (Some(cursor.opened_at), Some(cursor.id)),
        None => (None, None),
    };
    sqlx::query_as::<_, Alert>(concat!(
        "SELECT ",
        alert_columns!(),
        " FROM alert_state WHERE status = 'open' \
           AND ($1::timestamptz IS NULL OR (opened_at, id) > ($1::timestamptz, $2::uuid)) \
         ORDER BY opened_at, id LIMIT $3"
    ))
    .bind(cursor_time)
    .bind(cursor_id)
    .bind(limit)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// The evaluation needs the whole set, not a page: a missing alert would open a second one.
pub async fn open_alerts(tx: &mut TenantTx<'_>) -> Result<Vec<Alert>, DbError> {
    let mut all = Vec::new();
    let mut after = None;
    loop {
        let page = open_alerts_page(tx, after, OPEN_PAGE).await?;
        let full = page.len() as i64 == OPEN_PAGE;
        after = page.last().map(|alert| AlertCursor {
            opened_at: alert.opened_at,
            id: alert.id,
        });
        all.extend(page);
        if !full {
            return Ok(all);
        }
    }
}

pub async fn count_open(tx: &mut TenantTx<'_>) -> Result<i64, DbError> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM alert_state WHERE status = 'open'")
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(count)
}

pub async fn newest_open(tx: &mut TenantTx<'_>, limit: i64) -> Result<Vec<Alert>, DbError> {
    sqlx::query_as::<_, Alert>(concat!(
        "SELECT ",
        alert_columns!(),
        " FROM alert_state WHERE status = 'open' \
         ORDER BY opened_at DESC, id DESC LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)
}

/// Opens an alert. `false` means an open alert with the same kind and key exists, so the call
/// changed nothing.
pub async fn insert_open(
    tx: &mut TenantTx<'_>,
    new: &NewAlert<'_>,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "INSERT INTO alert_state \
         (id, tenant_id, kind, dedupe_key, status, agent_id, plan_id, repository_id, job_id, \
          agent_hostname, plan_name, repository_name, params, detail, opened_at) \
         VALUES ($1, $2, $3, $4, 'open', $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
         ON CONFLICT (tenant_id, kind, dedupe_key) WHERE status = 'open' DO NOTHING",
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
    .bind(sqlx::types::Json(new.params))
    .bind(new.detail)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
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
    sqlx::query_as::<_, Alert>(concat!(
        "SELECT ",
        alert_columns!(),
        " FROM alert_state WHERE id = $1"
    ))
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

#[derive(Debug, Clone, Default)]
pub struct AlertPage {
    /// Clamped to [`MAX_PAGE_LIMIT`].
    pub limit: i64,
    pub after: Option<AlertCursor>,
    /// Skipped rows of a numbered page; the caller keeps it in `0..=COUNT_LIMIT` and sends no
    /// cursor with it.
    pub offset: i64,
    /// `None` lists both states.
    pub status: Option<AlertStatus>,
    /// Empty: no filter.
    pub kinds: Vec<AlertKind>,
    /// Matches the host, the plan or the repository name anywhere, without case.
    pub search: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AlertPageResult {
    pub alerts: Vec<Alert>,
    pub has_more: bool,
    /// The alerts that match the filter on all pages, at most [`COUNT_LIMIT`].
    pub total: i64,
    /// More alerts match than [`COUNT_LIMIT`].
    pub total_capped: bool,
}

macro_rules! alert_filter_sql {
    () => {
        " WHERE ($1::text IS NULL OR status = $1) \
           AND (cardinality($2::text[]) = 0 OR kind = ANY($2)) \
           AND ($3::text IS NULL OR agent_hostname ILIKE $3 OR plan_name ILIKE $3 \
                OR repository_name ILIKE $3)"
    };
}

pub async fn list(tx: &mut TenantTx<'_>, page: &AlertPage) -> Result<AlertPageResult, DbError> {
    let limit = page.limit.clamp(1, MAX_PAGE_LIMIT);
    let (cursor_time, cursor_id) = match page.after {
        Some(cursor) => (Some(cursor.opened_at), Some(cursor.id)),
        None => (None, None),
    };
    let status = page.status.map(AlertStatus::as_str);
    let kinds: Vec<&str> = page.kinds.iter().map(|kind| kind.as_str()).collect();
    let pattern = page.search.as_deref().map(search_pattern);
    let mut alerts = sqlx::query_as::<_, Alert>(concat!(
        "SELECT ",
        alert_columns!(),
        " FROM alert_state",
        alert_filter_sql!(),
        " AND ($4::timestamptz IS NULL OR (opened_at, id) < ($4::timestamptz, $5::uuid)) \
         ORDER BY opened_at DESC, id DESC LIMIT $6 OFFSET $7"
    ))
    .bind(status)
    .bind(&kinds)
    .bind(pattern.as_deref())
    .bind(cursor_time)
    .bind(cursor_id)
    .bind(limit + 1)
    .bind(page.offset.max(0))
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let has_more = alerts.len() as i64 > limit;
    alerts.truncate(limit as usize);
    let counted: i64 = sqlx::query_scalar(concat!(
        "SELECT count(*) FROM (SELECT 1 FROM alert_state",
        alert_filter_sql!(),
        " LIMIT $4) s"
    ))
    .bind(status)
    .bind(&kinds)
    .bind(pattern.as_deref())
    .bind(COUNT_LIMIT + 1)
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(AlertPageResult {
        alerts,
        has_more,
        total: counted.min(COUNT_LIMIT),
        total_capped: counted > COUNT_LIMIT,
    })
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
    /// The sends that happened before this claim. [`extend_lease`] counts the next one.
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

/// Claims the due deliveries of the tenant and moves their next attempt to `lease_until`. A due
/// delivery with `max_attempts` sends behind it ends as failed instead.
///
/// The claim is a lease: a sender that crashes before it reports the result leaves the row due
/// again at `lease_until`, so a delivery is at-least-once (ADR-0036). The claim does not count an
/// attempt; [`extend_lease`] does when the send starts, and the count is the token of the send.
pub async fn claim_due(
    tx: &mut TenantTx<'_>,
    now: OffsetDateTime,
    lease_until: OffsetDateTime,
    limit: i64,
    max_attempts: i32,
) -> Result<Vec<ClaimedDelivery>, DbError> {
    sqlx::query(
        "UPDATE alert_deliveries SET status = 'failed', \
             last_error = COALESCE(last_error, 'attempts_exhausted') \
         WHERE status = 'pending' AND next_attempt_at <= $1 AND attempts >= $2",
    )
    .bind(now)
    .bind(max_attempts)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let rows = sqlx::query_as::<_, ClaimRow>(
        "WITH due AS ( \
             SELECT id FROM alert_deliveries \
             WHERE status = 'pending' AND next_attempt_at <= $1 AND attempts < $4 \
             ORDER BY next_attempt_at, id LIMIT $3 FOR UPDATE SKIP LOCKED) \
         UPDATE alert_deliveries d SET next_attempt_at = $2 \
         FROM due WHERE d.id = due.id \
         RETURNING d.id, d.alert_id, d.channel_id, d.event, d.attempts",
    )
    .bind(now)
    .bind(lease_until)
    .bind(limit)
    .bind(max_attempts)
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

/// Starts the send of a claim: counts the attempt and moves the lease to `lease_until`. `None`
/// means the claim is no longer the current one, so the caller must not send. `Some` is the number
/// of the attempt, counted from one, and the token of every later write of the send.
pub async fn extend_lease(
    tx: &mut TenantTx<'_>,
    id: DeliveryId,
    claimed_attempt: i32,
    lease_until: OffsetDateTime,
) -> Result<Option<i32>, DbError> {
    let row: Option<(i32,)> = sqlx::query_as(
        "UPDATE alert_deliveries SET attempts = attempts + 1, next_attempt_at = $3 \
         WHERE id = $1 AND attempts = $2 AND status = 'pending' \
         RETURNING attempts",
    )
    .bind(id)
    .bind(claimed_attempt)
    .bind(lease_until)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(|(attempt,)| attempt))
}

/// Gives a claim back unsent: the delivery is due again at `now` and keeps its attempts.
pub async fn release(
    tx: &mut TenantTx<'_>,
    id: DeliveryId,
    claimed_attempt: i32,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE alert_deliveries SET next_attempt_at = $3 \
         WHERE id = $1 AND attempts = $2 AND status = 'pending'",
    )
    .bind(id)
    .bind(claimed_attempt)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Records a sent delivery. `false` means the claim with this attempt number is no longer the
/// current one, and the row stays as it is.
pub async fn mark_sent(
    tx: &mut TenantTx<'_>,
    id: DeliveryId,
    attempt: i32,
    now: OffsetDateTime,
    note: Option<&str>,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE alert_deliveries SET status = 'sent', sent_at = $3, last_error = $4 \
         WHERE id = $1 AND attempts = $2 AND status = 'pending'",
    )
    .bind(id)
    .bind(attempt)
    .bind(now)
    .bind(note)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Records a failed attempt. `retry_at` set keeps the delivery pending for that time; `None` ends
/// it as failed. `false` means the claim is no longer the current one.
pub async fn mark_attempt_failed(
    tx: &mut TenantTx<'_>,
    id: DeliveryId,
    attempt: i32,
    error: &str,
    retry_at: Option<OffsetDateTime>,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE alert_deliveries \
         SET status = CASE WHEN $4::timestamptz IS NULL THEN 'failed' ELSE 'pending' END, \
             next_attempt_at = COALESCE($4, next_attempt_at), last_error = $3 \
         WHERE id = $1 AND attempts = $2 AND status = 'pending'",
    )
    .bind(id)
    .bind(attempt)
    .bind(error)
    .bind(retry_at)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
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
