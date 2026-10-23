//! The alert endpoints (FR-MN-03, AC-MN-02, ADR-0036).
//!
//! A Viewer reads the alerts and the rules of the tenant. An Admin changes the rules. The alerts
//! come from the evaluation loop of [`crate::alerts`]; no endpoint creates or resolves one.

use axum::Json;
use axum::extract::State;
use restichub_db::AlertId;
use restichub_db::alerts::{
    self as repo, Alert, AlertCursor, AlertKind, AlertPage, AlertParams, AlertRule, AlertStatus,
};
use restichub_db::audit::{AuditAction, diff};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};

use crate::alerts::message::{error_text, link};
use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::list_query::{ApiQuery, TabCount, check_window, parse_list, parse_search};
use crate::server::AppState;
use crate::validation::Reason;

/// The largest threshold of a rule: one year in minutes.
const MAX_THRESHOLD_MINUTES: i32 = 525_600;

#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusFilter {
    #[default]
    Open,
    Resolved,
    All,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct AlertListQuery {
    /// `open` (default), `resolved` or `all`.
    #[param(inline)]
    pub status: Option<StatusFilter>,
    /// Comma-separated, for example `job_failed,agent_offline`.
    pub kind: Option<String>,
    /// Matches the host, the plan or the repository name; at most 200 characters.
    pub q: Option<String>,
    /// Not with `offset`.
    pub cursor: Option<String>,
    /// Skipped alerts, in `0..=10000`. Not with `cursor`.
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AlertView {
    pub id: String,
    pub kind: AlertKind,
    pub status: AlertStatus,
    pub agent_id: Option<String>,
    pub agent_hostname: Option<String>,
    pub plan_id: Option<String>,
    pub plan_name: Option<String>,
    pub repository_id: Option<String>,
    pub repository_name: Option<String>,
    pub job_id: Option<String>,
    /// The typed values of the condition. A client builds the sentence from them in the language of
    /// the reader.
    pub params: AlertParams,
    pub error_text: Option<String>,
    /// The link of the alert. Absent when the installation sets no public origin.
    pub url: Option<String>,
    #[schema(format = DateTime)]
    pub opened_at: String,
    #[schema(format = DateTime)]
    pub resolved_at: Option<String>,
}

fn alert_view(alert: &Alert, origin: Option<&str>) -> AlertView {
    AlertView {
        id: alert.id.to_string(),
        kind: alert.kind,
        status: alert.status,
        agent_id: alert.agent_id.map(|id| id.to_string()),
        agent_hostname: alert.agent_hostname.clone(),
        plan_id: alert.plan_id.map(|id| id.to_string()),
        plan_name: alert.plan_name.clone(),
        repository_id: alert.repository_id.map(|id| id.to_string()),
        repository_name: alert.repository_name.clone(),
        job_id: alert.job_id.map(|id| id.to_string()),
        params: alert.params.0.clone(),
        error_text: error_text(alert),
        url: link(origin, alert),
        opened_at: format_time(alert.opened_at),
        resolved_at: alert.resolved_at.map(format_time),
    }
}

pub(crate) fn views(state: &AppState, alerts: &[Alert]) -> Vec<AlertView> {
    let origin = state
        .sender()
        .and_then(|sender| sender.settings().public_origin.as_deref());
    alerts
        .iter()
        .map(|alert| alert_view(alert, origin))
        .collect()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AlertListResponse {
    pub items: Vec<AlertView>,
    /// Null for a page read by `offset`.
    pub next_cursor: Option<String>,
    /// The alerts that match the filter on all pages, at most 10000.
    pub total: i64,
    /// More alerts match than `total` shows.
    pub total_capped: bool,
    /// The alerts of each state tab under the kind and search filters.
    pub counts: AlertCounts,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AlertCounts {
    pub open: TabCount,
    pub resolved: TabCount,
}

/// Lists the alerts of the tenant, newest first.
#[utoipa::path(
    get,
    path = "/api/v1/alerts",
    tags = ["alerts"],
    security(("cookie" = []), ("bearer" = [])),
    params(AlertListQuery),
    responses(
        (status = 200, description = "The alerts of the tenant.", body = AlertListResponse),
        (status = 400, description = "A query parameter is not valid.", body = crate::error::ProblemBody, content_type = "application/problem+json"),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_alerts(
    State(state): State<AppState>,
    context: RequestContext,
    ApiQuery(query): ApiQuery<AlertListQuery>,
) -> Result<Json<AlertListResponse>, ApiError> {
    context.authorize(Operation::AlertsRead)?;
    check_window(query.cursor.as_deref(), query.offset, true)?;
    let status = match query.status.unwrap_or_default() {
        StatusFilter::Open => Some(AlertStatus::Open),
        StatusFilter::Resolved => Some(AlertStatus::Resolved),
        StatusFilter::All => None,
    };
    let page = AlertPage {
        limit: query.limit.unwrap_or(repo::DEFAULT_PAGE_LIMIT),
        status,
        kinds: parse_list("kind", query.kind.as_deref(), AlertKind::parse)?,
        search: parse_search(query.q.as_deref())?,
        offset: query.offset.unwrap_or(0),
        after: query
            .cursor
            .as_deref()
            .map(decode_cursor)
            .transpose()
            .map_err(|cause| {
                ApiError::invalid_field_cause("cursor", Reason::InvalidFormat, cause)
            })?,
    };
    let (result, (open, resolved)) = context
        .tenant_tx(&state, async |tx| {
            let result = repo::list(tx, &page).await?;
            Ok((result, repo::status_counts(tx, &page).await?))
        })
        .await?;
    let next_cursor = if result.has_more && query.offset.is_none() {
        result.alerts.last().map(|alert| {
            encode_cursor(AlertCursor {
                opened_at: alert.opened_at,
                id: alert.id,
            })
        })
    } else {
        None
    };
    Ok(Json(AlertListResponse {
        items: views(&state, &result.alerts),
        next_cursor,
        total: result.total,
        total_capped: result.total_capped,
        counts: AlertCounts {
            open: open.into(),
            resolved: resolved.into(),
        },
    }))
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AlertRuleView {
    pub kind: AlertKind,
    pub enabled: bool,
    /// Minutes. Set for `agent_offline` and `stale_lock` only.
    pub threshold_minutes: Option<i32>,
}

impl From<&AlertRule> for AlertRuleView {
    fn from(rule: &AlertRule) -> Self {
        Self {
            kind: rule.kind,
            enabled: rule.enabled,
            threshold_minutes: rule.threshold_minutes,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AlertRulesResponse {
    pub items: Vec<AlertRuleView>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAlertRulesRequest {
    /// The rules to change. A kind that is absent keeps its setting.
    pub items: Vec<AlertRuleView>,
}

/// Reads the alert rules of the tenant, one for each kind.
#[utoipa::path(
    get,
    path = "/api/v1/alert-rules",
    tags = ["alerts"],
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The alert rules.", body = AlertRulesResponse),
        crate::error::ProtectedResponses,
    )
)]
pub async fn get_rules(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<AlertRulesResponse>, ApiError> {
    context.authorize(Operation::AlertsRead)?;
    let rules = context
        .tenant_tx(&state, async |tx| Ok(repo::list_rules(tx).await?))
        .await?;
    Ok(Json(AlertRulesResponse {
        items: rules.iter().map(AlertRuleView::from).collect(),
    }))
}

/// Changes the alert rules of the tenant.
#[utoipa::path(
    put,
    path = "/api/v1/alert-rules",
    tags = ["alerts"],
    security(("cookie" = []), ("bearer" = [])),
    request_body = UpdateAlertRulesRequest,
    responses(
        (status = 200, description = "The alert rules after the change.", body = AlertRulesResponse),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody, content_type = "application/problem+json"),
        crate::error::ProtectedResponses,
    )
)]
pub async fn put_rules(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<UpdateAlertRulesRequest>,
) -> Result<Json<AlertRulesResponse>, ApiError> {
    context.authorize(Operation::AlertsWrite)?;
    let mut changes: Vec<AlertRule> = Vec::with_capacity(body.items.len());
    for item in &body.items {
        if changes.iter().any(|rule| rule.kind == item.kind) {
            return Err(ApiError::invalid_field("items", Reason::InvalidFormat));
        }
        let threshold = match (item.kind.has_threshold(), item.threshold_minutes) {
            (false, Some(_)) => {
                return Err(ApiError::invalid_field(
                    "threshold_minutes",
                    Reason::Unsupported,
                ));
            }
            (false, None) => None,
            (true, None) => AlertRule::default_for(item.kind).threshold_minutes,
            (true, Some(minutes)) if minutes < 1 => {
                return Err(ApiError::invalid_field(
                    "threshold_minutes",
                    Reason::TooShort,
                ));
            }
            (true, Some(minutes)) if minutes > MAX_THRESHOLD_MINUTES => {
                return Err(ApiError::invalid_field(
                    "threshold_minutes",
                    Reason::TooLong,
                ));
            }
            (true, Some(minutes)) => Some(minutes),
        };
        changes.push(AlertRule {
            kind: item.kind,
            enabled: item.enabled,
            threshold_minutes: threshold,
        });
    }

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let before = repo::list_rules(&mut tx).await?;
    for rule in &changes {
        repo::upsert_rule(&mut tx, rule).await?;
    }
    let after = repo::list_rules(&mut tx).await?;
    let mut event = context.audit_event(
        AuditAction::AlertRuleUpdate,
        "alert_rules",
        context.tenant_id.as_uuid(),
    );
    event.diff = Some(diff(
        Some(&rules_value(&before)),
        Some(&rules_value(&after)),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(Json(AlertRulesResponse {
        items: after.iter().map(AlertRuleView::from).collect(),
    }))
}

fn rules_value(rules: &[AlertRule]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for rule in rules {
        map.insert(
            rule.kind.as_str().to_owned(),
            json!({"enabled": rule.enabled, "threshold_minutes": rule.threshold_minutes}),
        );
    }
    serde_json::Value::Object(map)
}

fn encode_cursor(cursor: AlertCursor) -> String {
    format!("{};{}", cursor.opened_at.unix_timestamp_nanos(), cursor.id)
}

fn decode_cursor(value: &str) -> Result<AlertCursor, String> {
    let (time, id) = value
        .split_once(';')
        .ok_or_else(|| "the cursor holds no `;`".to_owned())?;
    let nanos: i128 = time
        .parse()
        .map_err(|error: std::num::ParseIntError| error.to_string())?;
    let opened_at =
        OffsetDateTime::from_unix_timestamp_nanos(nanos).map_err(|error| error.to_string())?;
    if !(1970..=9999).contains(&opened_at.year()) {
        return Err("the time of the cursor is outside the supported range".to_owned());
    }
    let id: uuid::Uuid = id.parse().map_err(|error: uuid::Error| error.to_string())?;
    Ok(AlertCursor {
        opened_at,
        id: AlertId::from_uuid(id),
    })
}

fn format_time(value: OffsetDateTime) -> String {
    use time::format_description::well_known::Rfc3339;
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}
