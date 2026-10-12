//! The audit service and the audit read API (ADR-0023, section Audit log).
//!
//! The diff builder and the [`RedactedDiff`] type live in the database crate, so the value of a
//! diff can only come from that crate. A secret value never appears in a diff.
//!
//! The read API holds two endpoints: `GET /api/v1/audit` for one tenant and
//! `GET /api/v1/platform/audit` for the platform. No endpoint changes or removes an audit record
//! (AC-AU-01).

use axum::Json;
use axum::extract::{FromRequestParts, Query, State};
use axum::http::request::Parts;
use restichub_db::DbError;
use restichub_db::audit as repo;
use restichub_db::audit::{
    Actor, AuditAction, AuditEvent, AuditFilter, AuditPage, AuditRecord, diff,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::client_ip::ClientIp;
use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::server::{AppState, RequestIdHeader, request_id_of};

/// The highest number of records of the audit API.
const MAX_PAGE_LIMIT: i64 = repo::MAX_PAGE_LIMIT;

/// The audit service of the control plane.
///
/// The service writes records for the later epics. It holds no state.
#[derive(Debug, Clone, Copy)]
pub struct AuditService;

impl AuditService {
    /// Writes an audit event in the tenant of the transaction.
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
    /// can appear in the record.
    pub async fn record_secret_delivered(
        tx: &mut restichub_db::TenantTx<'_>,
        agent_id: Uuid,
        job_id: Uuid,
        repository_id: Uuid,
        secret_id: restichub_db::SecretId,
    ) -> Result<(), DbError> {
        let tenant = tx.tenant();
        let after = serde_json::json!({
            "tenant_id": tenant.to_string(),
            "agent_id": agent_id.to_string(),
            "job_id": job_id.to_string(),
            "repository_id": repository_id.to_string(),
        });
        let mut event = AuditEvent::new(AuditAction::SecretDelivered, "secret");
        event.object_id = Some(secret_id.as_uuid());
        event.diff = Some(diff(None, Some(&after), &[]));
        repo::insert(tx, &event).await.map(|_| ())
    }
}

/// One audit record in the API.
#[derive(Debug, Serialize, ToSchema)]
pub struct AuditItem {
    /// The identifier of the record.
    pub id: String,
    /// The tenant of the record.
    pub tenant_id: String,
    /// The time of the record in RFC 3339.
    pub occurred_at: String,
    /// The actor type.
    pub actor_type: String,
    /// The actor identifier.
    pub actor_id: Option<String>,
    /// The source IP address.
    pub source_ip: Option<String>,
    /// The action.
    pub action: String,
    /// The object type.
    pub object_type: String,
    /// The object identifier.
    pub object_id: Option<String>,
    /// The diff.
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
            action: record.action.as_str().to_owned(),
            object_type: record.object_type.clone(),
            object_id: record.object_id.map(|id| id.to_string()),
            diff: record.diff.clone(),
        }
    }
}

/// A page of audit records in the API.
#[derive(Debug, Serialize, ToSchema)]
pub struct AuditPageResponse {
    /// The records.
    pub items: Vec<AuditItem>,
    /// The cursor of the next page. It is null when the table holds no later record.
    pub next_cursor: Option<String>,
}

/// The query of the audit API.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct AuditQuery {
    /// The start of the time range in RFC 3339.
    pub from: Option<String>,
    /// The end of the time range in RFC 3339.
    pub to: Option<String>,
    /// The action.
    pub action: Option<String>,
    /// The actor type.
    pub actor_type: Option<String>,
    /// The actor identifier. It needs `actor_type`.
    pub actor_id: Option<String>,
    /// The object type.
    pub object_type: Option<String>,
    /// The object identifier.
    pub object_id: Option<String>,
    /// The tenant of the record. The platform list uses it.
    pub tenant_id: Option<String>,
    /// The cursor of the next page.
    pub cursor: Option<String>,
    /// The number of records.
    pub limit: Option<i64>,
}

impl AuditQuery {
    /// Builds the filter. The function returns an error on a bad value.
    fn filter(&self) -> Result<AuditFilter, ()> {
        if self.actor_id.is_some() && self.actor_type.is_none() {
            return Err(());
        }
        let action = match self.action.as_deref() {
            Some(value) => Some(AuditAction::parse(value).ok_or(())?),
            None => None,
        };
        let actor = self.actor()?;
        Ok(AuditFilter {
            from: self.from.as_deref().map(parse_time).transpose()?,
            to: self.to.as_deref().map(parse_time).transpose()?,
            action,
            actor,
            object_type: self.object_type.clone(),
            object_id: self.object_id.as_deref().map(parse_uuid).transpose()?,
            tenant_id: self
                .tenant_id
                .as_deref()
                .map(parse_uuid)
                .transpose()?
                .map(restichub_db::TenantId::from_uuid),
        })
    }

    /// Builds the actor of the filter.
    fn actor(&self) -> Result<Option<Actor>, ()> {
        let Some(actor_type) = self.actor_type.as_deref() else {
            return Ok(None);
        };
        let actor = match actor_type {
            "system" => Actor::System,
            "user" => Actor::User(restichub_db::UserId::from_uuid(parse_uuid(
                self.actor_id.as_deref().ok_or(())?,
            )?)),
            "api_token" => Actor::ApiToken(restichub_db::ApiTokenId::from_uuid(parse_uuid(
                self.actor_id.as_deref().ok_or(())?,
            )?)),
            "agent" => Actor::Agent(parse_uuid(self.actor_id.as_deref().ok_or(())?)?),
            _ => return Err(()),
        };
        Ok(Some(actor))
    }

    /// Builds the page.
    fn page(&self) -> Result<AuditPage, ()> {
        let limit = self
            .limit
            .unwrap_or(repo::DEFAULT_PAGE_LIMIT)
            .clamp(1, MAX_PAGE_LIMIT);
        let after = self.cursor.as_deref().map(decode_cursor).transpose()?;
        Ok(AuditPage { limit, after })
    }
}

/// The query extractor of the audit API.
///
/// A bad query gives an RFC 9457 problem with the code `invalid_argument`.
pub struct AuditParams(pub AuditQuery);

impl FromRequestParts<AppState> for AuditParams {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match Query::<AuditQuery>::from_request_parts(parts, state).await {
            Ok(Query(query)) => Ok(Self(query)),
            Err(error) => {
                tracing::debug!(%error, "the audit query is not valid");
                Err(ApiError::invalid_argument(&request_id_of(parts)))
            }
        }
    }
}

/// Formats a time as RFC 3339.
fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

/// Parses an RFC 3339 time.
fn parse_time(value: &str) -> Result<OffsetDateTime, ()> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| ())
}

/// Parses a UUID.
fn parse_uuid(value: &str) -> Result<Uuid, ()> {
    Uuid::parse_str(value).map_err(|_| ())
}

/// Encodes the cursor of a page.
fn encode_cursor(cursor: repo::AuditCursor) -> String {
    format!(
        "{};{}",
        cursor.occurred_at.unix_timestamp_nanos(),
        cursor.id
    )
}

/// Decodes the cursor of a page.
fn decode_cursor(value: &str) -> Result<repo::AuditCursor, ()> {
    let (time, id) = value.split_once(';').ok_or(())?;
    let nanos: i128 = time.parse().map_err(|_| ())?;
    let occurred_at = OffsetDateTime::from_unix_timestamp_nanos(nanos).map_err(|_| ())?;
    if !repo::is_supported_time(occurred_at) {
        return Err(());
    }
    let id = id.parse().map_err(|_| ())?;
    Ok(repo::AuditCursor { occurred_at, id })
}

/// Lists the audit records of one tenant.
#[utoipa::path(
    get,
    path = "/api/v1/audit",
    params(AuditQuery),
    responses(
        (status = 200, description = "The audit records of the tenant.", body = AuditPageResponse),
        (status = 400, description = "A query parameter is not valid."),
        (status = 401, description = "The request is not authenticated."),
        (status = 403, description = "The role is not sufficient.")
    )
)]
pub async fn list_tenant_audit(
    State(state): State<AppState>,
    context: RequestContext,
    _client: ClientIp,
    request_id: RequestIdHeader,
    params: AuditParams,
) -> Result<Json<AuditPageResponse>, ApiError> {
    context.authorize(Operation::AuditRead, &request_id.0)?;
    let query = params.0;
    let filter = query
        .filter()
        .map_err(|_| ApiError::invalid_argument(&request_id.0))?;
    let page = query
        .page()
        .map_err(|_| ApiError::invalid_argument(&request_id.0))?;
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id.0, &error))?;
    let result = repo::list(&mut tx, &filter, &page)
        .await
        .map_err(|error| ApiError::internal(&request_id.0, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id.0, &error))?;
    Ok(Json(response(result)))
}

/// Lists the audit records of the platform.
#[utoipa::path(
    get,
    path = "/api/v1/platform/audit",
    params(AuditQuery),
    responses(
        (status = 200, description = "The audit records of the platform.", body = AuditPageResponse),
        (status = 400, description = "A query parameter is not valid."),
        (status = 401, description = "The request is not authenticated."),
        (status = 403, description = "The role is not sufficient.")
    )
)]
pub async fn list_platform_audit(
    State(state): State<AppState>,
    context: RequestContext,
    _client: ClientIp,
    request_id: RequestIdHeader,
    params: AuditParams,
) -> Result<Json<AuditPageResponse>, ApiError> {
    context.authorize(Operation::PlatformAuditRead, &request_id.0)?;
    let query = params.0;
    let filter = query
        .filter()
        .map_err(|_| ApiError::invalid_argument(&request_id.0))?;
    let page = query
        .page()
        .map_err(|_| ApiError::invalid_argument(&request_id.0))?;
    let mut tx = state
        .db()
        .begin_platform()
        .await
        .map_err(|error| ApiError::internal(&request_id.0, &error))?;
    let result = repo::list_platform(&mut tx, &filter, &page)
        .await
        .map_err(|error| ApiError::internal(&request_id.0, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id.0, &error))?;
    Ok(Json(response(result)))
}

/// Builds the API page response.
///
/// The function returns `next_cursor` only when the table holds a later record.
fn response(result: repo::AuditPageResult) -> AuditPageResponse {
    let next_cursor = if result.has_more {
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
        // A bad format.
        assert!(decode_cursor("nope").is_err());
        // A bad identifier.
        assert!(decode_cursor("0;not-a-uuid").is_err());
        // A time before 1970.
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
