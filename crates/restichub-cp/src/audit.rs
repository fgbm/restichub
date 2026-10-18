//! The audit service and the audit read API (ADR-0023, section Audit log).
//!
//! The diff builder and the `RedactedDiff` type live in the database crate, so the value of a
//! diff can only come from that crate. A secret value never appears in a diff.
//!
//! The read API holds two endpoints: `GET /api/v1/audit` for one tenant and
//! `GET /api/v1/platform/audit` for the platform. No endpoint changes or removes an audit record
//! (AC-AU-01).

use axum::Json;
use axum::extract::State;
use restichub_db::ActorType;
use restichub_db::DbError;
use restichub_db::audit as repo;
use restichub_db::audit::{Actor, AuditAction, AuditEvent, AuditFilter, AuditPage, AuditRecord};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::list_query::{ApiQuery, check_window, parse_search};
use crate::server::AppState;
use crate::validation::Reason;

const MAX_PAGE_LIMIT: i64 = repo::MAX_PAGE_LIMIT;

/// The service writes records for the later epics. It holds no state.
#[derive(Debug, Clone, Copy)]
pub struct AuditService;

impl AuditService {
    pub async fn record(
        tx: &mut restichub_db::TenantTx<'_>,
        event: AuditEvent,
    ) -> Result<(), DbError> {
        repo::insert(tx, &event).await.map(|_| ())
    }

    /// Writes the record of a secret delivery to an agent (AC-RP-03).
    ///
    /// The tenant comes from the transaction only. The record holds the tenant, the agent, the
    /// job, the repository and the time. The function takes no secret value, so no secret value
    /// can appear in the record. The delivery path
    /// [`restichub_db::delivery::load_ciphertext_for_delivery`] uses the same builder, so the shape
    /// of the record lives in one place.
    pub async fn record_secret_delivered(
        tx: &mut restichub_db::TenantTx<'_>,
        agent_id: Uuid,
        job_id: Uuid,
        repository_id: Uuid,
        secret_id: restichub_db::SecretId,
    ) -> Result<(), DbError> {
        repo::insert_secret_delivered(tx, secret_id, agent_id, job_id, repository_id)
            .await
            .map(|_| ())
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditItem {
    pub id: String,
    pub tenant_id: String,
    /// In RFC 3339.
    pub occurred_at: String,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub source_ip: Option<String>,
    pub action: AuditAction,
    pub object_type: String,
    pub object_id: Option<String>,
    /// What the object is called now. Null for a removed object, an object without a name and, in
    /// the platform list, an object inside an organization.
    pub object_name: Option<String>,
    /// The current name of a user, an API token or an agent actor, with the same rule.
    pub actor_name: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub diff: Option<Value>,
}

impl From<&AuditRecord> for AuditItem {
    fn from(record: &AuditRecord) -> Self {
        Self {
            id: record.id.to_string(),
            tenant_id: record.tenant_id.to_string(),
            occurred_at: format_time(record.occurred_at),
            actor_type: record.actor.actor_type().to_owned(),
            actor_id: record.actor.actor_id().map(|id| id.to_string()),
            source_ip: record.source_ip.map(|ip| ip.to_string()),
            action: record.action.clone(),
            object_type: record.object_type.clone(),
            object_id: record.object_id.map(|id| id.to_string()),
            object_name: record.object_name.clone(),
            actor_name: record.actor_name.clone(),
            diff: record.diff.clone(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditPageResponse {
    pub items: Vec<AuditItem>,
    /// Null when the table holds no later record, and for a page read by `offset`.
    pub next_cursor: Option<String>,
    /// The records that match the filter on all pages, at most 10000.
    pub total: i64,
    /// More records match than `total` shows.
    pub total_capped: bool,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct AuditQuery {
    /// Start of the time range in RFC 3339.
    pub from: Option<String>,
    /// End of the time range in RFC 3339.
    pub to: Option<String>,
    pub action: Option<String>,
    pub actor_type: Option<String>,
    /// Needs `actor_type`.
    pub actor_id: Option<String>,
    pub object_type: Option<String>,
    pub object_id: Option<String>,
    /// Used by the platform list.
    pub tenant_id: Option<String>,
    /// A part of the action code, the object name or the actor name, in any case; at most 200
    /// characters.
    pub q: Option<String>,
    /// Not with `offset`.
    pub cursor: Option<String>,
    /// Skipped records, in `0..=10000`. Not with `cursor`.
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

/// The query parameters that the tenant audit route documents. The route reads rows of the caller's
/// tenant only (ADR-0006), so the documentation omits `tenant_id`.
#[derive(IntoParams)]
#[into_params(parameter_in = Query)]
#[allow(dead_code)]
pub struct TenantAuditParams {
    /// Start of the time range in RFC 3339.
    from: Option<String>,
    /// End of the time range in RFC 3339.
    to: Option<String>,
    action: Option<String>,
    actor_type: Option<String>,
    /// Needs `actor_type`.
    actor_id: Option<String>,
    object_type: Option<String>,
    object_id: Option<String>,
    /// A part of the action code, the object name or the actor name, in any case; at most 200
    /// characters.
    q: Option<String>,
    /// Not with `offset`.
    cursor: Option<String>,
    /// Skipped records, in `0..=10000`. Not with `cursor`.
    offset: Option<i64>,
    limit: Option<i64>,
}

impl AuditQuery {
    fn filter(&self) -> Result<AuditFilter, ApiError> {
        if self.actor_id.is_some() && self.actor_type.is_none() {
            return Err(invalid_query("`actor_id` needs `actor_type`"));
        }
        let action = match self.action.as_deref() {
            Some(value) => Some(
                AuditAction::parse(value)
                    .ok_or_else(|| invalid_query("the action is not known"))?,
            ),
            None => None,
        };
        let actor = self.actor()?;
        Ok(AuditFilter {
            from: self
                .from
                .as_deref()
                .map(parse_time)
                .transpose()
                .map_err(invalid_query)?,
            to: self
                .to
                .as_deref()
                .map(parse_time)
                .transpose()
                .map_err(invalid_query)?,
            action,
            actor,
            object_type: self.object_type.clone(),
            object_id: self
                .object_id
                .as_deref()
                .map(parse_uuid)
                .transpose()
                .map_err(invalid_query)?,
            tenant_id: self
                .tenant_id
                .as_deref()
                .map(parse_uuid)
                .transpose()
                .map_err(invalid_query)?
                .map(restichub_db::TenantId::from_uuid),
            search: parse_search(self.q.as_deref())?,
        })
    }

    fn actor(&self) -> Result<Option<Actor>, ApiError> {
        let Some(actor_type) = self.actor_type.as_deref() else {
            return Ok(None);
        };
        let actor = match ActorType::parse(actor_type)
            .ok_or_else(|| invalid_query("the actor type is not known"))?
        {
            ActorType::System => Actor::System,
            ActorType::User => Actor::User(restichub_db::UserId::from_uuid(
                self.actor_id().map_err(invalid_query)?,
            )),
            ActorType::ApiToken => Actor::ApiToken(restichub_db::ApiTokenId::from_uuid(
                self.actor_id().map_err(invalid_query)?,
            )),
            ActorType::Agent => Actor::Agent(self.actor_id().map_err(invalid_query)?),
        };
        Ok(Some(actor))
    }

    fn actor_id(&self) -> Result<Uuid, String> {
        parse_uuid(self.actor_id.as_deref().ok_or("`actor_id` is missing")?)
    }

    fn page(&self) -> Result<AuditPage, ApiError> {
        let limit = self
            .limit
            .unwrap_or(repo::DEFAULT_PAGE_LIMIT)
            .clamp(1, MAX_PAGE_LIMIT);
        let after = self
            .cursor
            .as_deref()
            .map(decode_cursor)
            .transpose()
            .map_err(invalid_cursor)?;
        check_window(self.cursor.as_deref(), self.offset, true)?;
        Ok(AuditPage {
            limit,
            after,
            offset: self.offset.unwrap_or(0),
        })
    }
}

fn invalid_query(cause: impl Into<String>) -> ApiError {
    ApiError::invalid_field_cause("query", Reason::InvalidFormat, cause)
}

fn invalid_cursor(cause: impl Into<String>) -> ApiError {
    ApiError::invalid_field_cause("cursor", Reason::InvalidFormat, cause)
}

fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

fn parse_time(value: &str) -> Result<OffsetDateTime, String> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|error| error.to_string())
}

fn parse_uuid(value: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value).map_err(|error| error.to_string())
}

fn encode_cursor(cursor: repo::AuditCursor) -> String {
    format!(
        "{};{}",
        cursor.occurred_at.unix_timestamp_nanos(),
        cursor.id
    )
}

fn decode_cursor(value: &str) -> Result<repo::AuditCursor, String> {
    let (time, id) = value
        .split_once(';')
        .ok_or_else(|| "the cursor holds no `;`".to_owned())?;
    let nanos: i128 = time
        .parse()
        .map_err(|error: std::num::ParseIntError| error.to_string())?;
    let occurred_at =
        OffsetDateTime::from_unix_timestamp_nanos(nanos).map_err(|error| error.to_string())?;
    if !repo::is_supported_time(occurred_at) {
        return Err("the time of the cursor is outside the supported range".to_owned());
    }
    let id = id.parse().map_err(|error: uuid::Error| error.to_string())?;
    Ok(repo::AuditCursor { occurred_at, id })
}

/// Lists the audit records of one tenant.
#[utoipa::path(
    get,
    path = "/api/v1/audit",
    security(("cookie" = []), ("bearer" = [])),
    params(TenantAuditParams),
    responses(
        (status = 200, description = "The audit records of the tenant.", body = AuditPageResponse),
        (status = 400, description = "A query parameter is not valid."),
        crate::error::ProtectedResponses
    )
)]
pub async fn list_tenant_audit(
    State(state): State<AppState>,
    context: RequestContext,
    ApiQuery(query): ApiQuery<AuditQuery>,
) -> Result<Json<AuditPageResponse>, ApiError> {
    context.authorize(Operation::AuditRead)?;
    let filter = query.filter()?;
    let page = query.page()?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let result = repo::list(&mut tx, &filter, &page).await?;
    tx.commit().await?;
    Ok(Json(response(result, query.offset.is_none())))
}

/// Lists the audit records of the platform.
#[utoipa::path(
    get,
    path = "/api/v1/platform/audit",
    security(("cookie" = []), ("bearer" = [])),
    params(AuditQuery),
    responses(
        (status = 200, description = "The audit records of the platform.", body = AuditPageResponse),
        (status = 400, description = "A query parameter is not valid."),
        crate::error::ProtectedResponses
    )
)]
pub async fn list_platform_audit(
    State(state): State<AppState>,
    context: RequestContext,
    ApiQuery(query): ApiQuery<AuditQuery>,
) -> Result<Json<AuditPageResponse>, ApiError> {
    context.authorize(Operation::PlatformAuditRead)?;
    let filter = query.filter()?;
    let page = query.page()?;
    let mut tx = state.db().begin_platform().await?;
    let result = repo::list_platform(&mut tx, &filter, &page).await?;
    tx.commit().await?;
    Ok(Json(response(result, query.offset.is_none())))
}

/// `next_cursor` is set only when the table holds a later record and the page did not use an
/// offset, so a client keeps one way to read the next page.
fn response(result: repo::AuditPageResult, by_cursor: bool) -> AuditPageResponse {
    let next_cursor = if result.has_more && by_cursor {
        result.records.last().map(|record| {
            encode_cursor(repo::AuditCursor {
                occurred_at: record.occurred_at,
                id: record.id,
            })
        })
    } else {
        None
    };
    AuditPageResponse {
        items: result.records.iter().map(AuditItem::from).collect(),
        next_cursor,
        total: result.total,
        total_capped: result.total_capped,
    }
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    use uuid::Uuid;

    use super::{decode_cursor, format_time, parse_time};

    fn time(value: &str) -> OffsetDateTime {
        OffsetDateTime::parse(value, &Rfc3339).unwrap_or(OffsetDateTime::UNIX_EPOCH)
    }

    #[test]
    fn occurred_at_round_trips_into_from() {
        let value = time("2026-10-12T10:00:00Z");
        let text = format_time(value);
        assert_eq!(text, "2026-10-12T10:00:00Z");
        assert_eq!(parse_time(&text), Ok(value));
    }

    #[test]
    fn a_bad_cursor_is_refused() {
        assert!(decode_cursor("nope").is_err());
        assert!(decode_cursor("0;not-a-uuid").is_err());
        let old = time("1900-01-01T00:00:00Z");
        let cursor = format!("{};{}", old.unix_timestamp_nanos(), Uuid::nil());
        assert!(decode_cursor(&cursor).is_err());
        // A time after 9999 is refused too (the parse fails for the value or the range check).
        let far = format!("{};{}", i128::MAX, Uuid::nil());
        assert!(decode_cursor(&far).is_err());
    }

    #[test]
    fn a_good_cursor_round_trips() {
        let value = time("2026-10-12T10:00:00Z");
        let id = Uuid::now_v7();
        let cursor = format!("{};{}", value.unix_timestamp_nanos(), id);
        let decoded = decode_cursor(&cursor);
        assert_eq!(
            decoded.map(|cursor| (cursor.occurred_at, cursor.id.as_uuid())),
            Ok((value, id))
        );
    }
}
