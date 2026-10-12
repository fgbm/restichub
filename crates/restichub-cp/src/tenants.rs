//! The platform tenant administration endpoints (issue #20, FR-TN-02).
//!
//! The Platform Admin of the platform tenant creates, lists, reads, suspends, resumes and deletes
//! the tenants. The endpoints return the count of the users only. They never return a user list or
//! a secret of another tenant.
//!
//! A suspended tenant cannot sign in and its API tokens fail (AC-TN-03). The agents of the tenant
//! stay connected (epic #1): the control plane does not close their connections. The deletion is a
//! soft delete in the MVP: the status becomes `deleted`, the sign-in and the tokens stop, and the
//! rows stay. A later retention task removes the rows of a deleted tenant.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use restichub_db::audit::{AuditAction, AuditEvent};
use restichub_db::tenants as repo;
use restichub_db::tenants::{TenantFilter, TenantPage, TenantSummary};
use restichub_db::{PLATFORM_TENANT_ID, TenantId};
use restichub_secrets::{generate_dek, wrap_dek};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::client_ip::ClientIp;
use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::password;
use crate::server::{AppState, RequestIdHeader};
use crate::validation;
use crate::views::PlatformTenantView;

/// The body of the create-tenant request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTenantRequest {
    /// The name of the tenant. It holds 1 to 63 characters from `a-z`, `0-9` and `-`, it starts
    /// with a letter, and it is not `platform`.
    pub name: String,
    /// The first administrator of the tenant.
    pub first_admin: CreateTenantAdmin,
}

/// The first administrator of a new tenant.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTenantAdmin {
    /// The username of the administrator. It is unique for the tenant and ignores the letter case.
    pub username: String,
    /// The name of the administrator for the user interface.
    pub display_name: String,
    /// The password of the administrator. It must hold at least 12 characters.
    pub password: String,
}

/// The query of the tenant list.
#[derive(Debug, Default, Deserialize, IntoParams)]
pub struct TenantListQuery {
    /// The status of the tenant: `active`, `suspended` or `deleted`.
    pub status: Option<String>,
    /// The cursor of the next page.
    pub cursor: Option<String>,
    /// The number of tenants.
    pub limit: Option<i64>,
}

/// A page of tenants in the API.
#[derive(Debug, Serialize, ToSchema)]
pub struct TenantListResponse {
    /// The tenants.
    pub items: Vec<PlatformTenantView>,
    /// The cursor of the next page. It is null when the table holds no later tenant.
    pub next_cursor: Option<String>,
}

/// Lists the tenants of the platform.
#[utoipa::path(
    get,
    path = "/api/v1/platform/tenants",
    security(("cookie" = []), ("bearer" = [])),
    params(TenantListQuery),
    responses(
        (status = 200, description = "The tenants of the platform.", body = TenantListResponse),
        (status = 400, description = "A query parameter is not valid.", body = crate::error::ProblemBody),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody)
    )
)]
pub async fn list_tenants(
    State(state): State<AppState>,
    context: RequestContext,
    request_id: RequestIdHeader,
    Query(query): Query<TenantListQuery>,
) -> Result<Json<TenantListResponse>, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::PlatformTenantsRead, &request_id)?;
    let filter = filter(&query, &request_id)?;
    let page = page(&query, &request_id)?;
    let mut tx = state
        .db()
        .begin_platform()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let result = repo::list_platform(&mut tx, &filter, &page)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    Ok(Json(response(result)))
}

/// Creates a tenant with its first administrator.
#[utoipa::path(
    post,
    path = "/api/v1/platform/tenants",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateTenantRequest,
    responses(
        (status = 201, description = "The tenant is created.", body = PlatformTenantView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody),
        (status = 409, description = "The name exists.", body = crate::error::ProblemBody),
        (status = 503, description = "The password work is busy.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_tenant(
    State(state): State<AppState>,
    context: RequestContext,
    client: ClientIp,
    request_id: RequestIdHeader,
    ApiJson(body): ApiJson<CreateTenantRequest>,
) -> Result<(StatusCode, Json<PlatformTenantView>), ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::PlatformTenantsWrite, &request_id)?;
    let name = validation::validate_tenant_name(&body.name)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;
    let username = validation::validate_username(&body.first_admin.username)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;
    let display_name = validation::validate_display_name(&body.first_admin.display_name)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;
    password::validate_password(&body.first_admin.password)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;

    let kek = state
        .kek()
        .ok_or_else(|| ApiError::internal_error(&request_id))?;

    // Hash the password before the transaction (ADR-0023: a hash never holds a transaction).
    let password_hash = password::hash_password_limited(
        &state.auth().hash_semaphore,
        body.first_admin.password.clone(),
    )
    .await
    .map_err(|error| ApiError::from_password(&request_id, &error))?;

    // A fresh DEK, wrapped by the current KEK. The associated data holds the tenant id, so the id
    // comes first (ADR-0023, section Secrets).
    let tenant_id = TenantId::new();
    let dek = generate_dek().map_err(|error| ApiError::internal(&request_id, &error))?;
    let wrapped = wrap_dek(kek.current(), tenant_id.as_uuid(), 1, &dek)
        .map_err(|error| ApiError::internal(&request_id, &error))?;

    let input = repo::BootstrapTenant {
        id: tenant_id,
        name: &name,
        key_version: 1,
        kek_id: wrapped.kek_id.as_str(),
        wrapped_dek: &wrapped.ciphertext,
        nonce: wrapped.nonce.as_slice(),
        actor: context.actor.clone(),
        source_ip: Some(client.0),
        admin: Some(repo::BootstrapAdmin {
            username: &username,
            password_hash: &password_hash,
            display_name: &display_name,
        }),
    };

    let mut tx = state
        .db()
        .begin_platform()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let created = repo::create_with_bootstrap(&mut tx, &input)
        .await
        .map_err(|error| map_write_error(&request_id, &error))?;
    let summary = repo::get_platform(&mut tx, created)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let Some(summary) = summary else {
        return Err(ApiError::internal_error(&request_id));
    };
    Ok((
        StatusCode::CREATED,
        Json(PlatformTenantView::from(&summary)),
    ))
}

/// Reads one tenant of the platform.
#[utoipa::path(
    get,
    path = "/api/v1/platform/tenants/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the tenant.")),
    responses(
        (status = 200, description = "The tenant.", body = PlatformTenantView),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody),
        (status = 404, description = "The tenant is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_tenant(
    State(state): State<AppState>,
    context: RequestContext,
    request_id: RequestIdHeader,
    Path(id): Path<String>,
) -> Result<Json<PlatformTenantView>, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::PlatformTenantsRead, &request_id)?;
    let tenant_id = parse_tenant_id(&id, &request_id)?;
    let mut tx = state
        .db()
        .begin_platform()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let tenant = repo::get_platform(&mut tx, tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    match tenant {
        Some(tenant) => Ok(Json(PlatformTenantView::from(&tenant))),
        None => Err(ApiError::not_found(&request_id)),
    }
}

/// Suspends a tenant.
///
/// The endpoint is idempotent. The platform tenant cannot be suspended.
#[utoipa::path(
    post,
    path = "/api/v1/platform/tenants/{id}/suspend",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the tenant.")),
    responses(
        (status = 204, description = "The tenant is suspended."),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody),
        (status = 404, description = "The tenant is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The tenant is the platform tenant or it is deleted.", body = crate::error::ProblemBody)
    )
)]
pub async fn suspend_tenant(
    State(state): State<AppState>,
    context: RequestContext,
    client: ClientIp,
    request_id: RequestIdHeader,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::PlatformTenantsWrite, &request_id)?;
    let tenant_id = parse_tenant_id(&id, &request_id)?;
    if tenant_id == PLATFORM_TENANT_ID {
        return Err(ApiError::conflict(&request_id));
    }
    let event = audit_event(&context, client.0, AuditAction::TenantSuspend, tenant_id);
    transition(
        &state,
        &request_id,
        tenant_id,
        "active",
        "suspended",
        &event,
    )
    .await
}

/// Resumes a tenant.
///
/// The endpoint is idempotent. A deleted tenant cannot be resumed.
#[utoipa::path(
    post,
    path = "/api/v1/platform/tenants/{id}/resume",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the tenant.")),
    responses(
        (status = 204, description = "The tenant is active."),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody),
        (status = 404, description = "The tenant is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The tenant is deleted.", body = crate::error::ProblemBody)
    )
)]
pub async fn resume_tenant(
    State(state): State<AppState>,
    context: RequestContext,
    client: ClientIp,
    request_id: RequestIdHeader,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::PlatformTenantsWrite, &request_id)?;
    let tenant_id = parse_tenant_id(&id, &request_id)?;
    let event = audit_event(&context, client.0, AuditAction::TenantResume, tenant_id);
    transition(
        &state,
        &request_id,
        tenant_id,
        "suspended",
        "active",
        &event,
    )
    .await
}

/// Deletes a tenant (soft delete).
///
/// The tenant must be suspended first. The endpoint sets the status to `deleted`; it removes no
/// row. The platform tenant cannot be deleted.
#[utoipa::path(
    delete,
    path = "/api/v1/platform/tenants/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the tenant.")),
    responses(
        (status = 204, description = "The tenant is deleted."),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody),
        (status = 404, description = "The tenant is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The tenant is the platform tenant or it is not suspended.", body = crate::error::ProblemBody)
    )
)]
pub async fn delete_tenant(
    State(state): State<AppState>,
    context: RequestContext,
    client: ClientIp,
    request_id: RequestIdHeader,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::PlatformTenantsWrite, &request_id)?;
    let tenant_id = parse_tenant_id(&id, &request_id)?;
    if tenant_id == PLATFORM_TENANT_ID {
        return Err(ApiError::conflict(&request_id));
    }
    let event = audit_event(&context, client.0, AuditAction::TenantDelete, tenant_id);
    transition(
        &state,
        &request_id,
        tenant_id,
        "suspended",
        "deleted",
        &event,
    )
    .await
}

/// Moves a tenant from one status to another and maps the result to the response.
///
/// The function names the allowed source status in the update, so a concurrent change wins or loses
/// cleanly. A changed status and an idempotent no-op give HTTP 204. A missing tenant gives HTTP
/// 404. A current status that is neither the source nor the target gives HTTP 409.
async fn transition(
    state: &AppState,
    request_id: &str,
    tenant_id: TenantId,
    from: &str,
    to: &str,
    event: &AuditEvent,
) -> Result<StatusCode, ApiError> {
    let mut tx = state
        .db()
        .begin_platform()
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    let result = repo::set_status_from(&mut tx, tenant_id, from, to, event, event)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    match result {
        repo::Transition::Changed | repo::Transition::AlreadyInTarget => Ok(StatusCode::NO_CONTENT),
        repo::Transition::NotFound => Err(ApiError::not_found(request_id)),
        repo::Transition::Refused => Err(ApiError::conflict(request_id)),
    }
}

/// Builds the audit event of a platform tenant operation.
fn audit_event(
    context: &RequestContext,
    source_ip: std::net::IpAddr,
    action: AuditAction,
    tenant_id: TenantId,
) -> AuditEvent {
    let mut event = AuditEvent::new(action, "tenant");
    event.actor = context.actor.clone();
    event.source_ip = Some(source_ip);
    event.object_id = Some(tenant_id.as_uuid());
    event
}

/// Builds the filter of the list. The function returns an error on a bad status.
fn filter(query: &TenantListQuery, request_id: &str) -> Result<TenantFilter, ApiError> {
    let status = match query.status.as_deref() {
        None => None,
        Some(value @ ("active" | "suspended" | "deleted")) => Some(value.to_owned()),
        Some(_) => return Err(ApiError::invalid_argument(request_id)),
    };
    Ok(TenantFilter { status })
}

/// Builds the page of the list. The function returns an error on a bad cursor.
fn page(query: &TenantListQuery, request_id: &str) -> Result<TenantPage, ApiError> {
    let limit = query
        .limit
        .unwrap_or(repo::DEFAULT_PAGE_LIMIT)
        .clamp(1, repo::MAX_PAGE_LIMIT);
    let after = query
        .cursor
        .as_deref()
        .map(decode_cursor)
        .transpose()
        .map_err(|_| ApiError::invalid_argument(request_id))?;
    Ok(TenantPage { limit, after })
}

/// Builds the API page response.
fn response(result: repo::TenantPageResult) -> TenantListResponse {
    let next_cursor = if result.has_more {
        result.tenants.last().map(encode_cursor)
    } else {
        None
    };
    TenantListResponse {
        items: result
            .tenants
            .iter()
            .map(PlatformTenantView::from)
            .collect(),
        next_cursor,
    }
}

/// Encodes the cursor of a page.
fn encode_cursor(tenant: &TenantSummary) -> String {
    format!("{};{}", tenant.created_at.unix_timestamp_nanos(), tenant.id)
}

/// Decodes the cursor of a page.
fn decode_cursor(value: &str) -> Result<repo::TenantCursor, ()> {
    let (time, id) = value.split_once(';').ok_or(())?;
    let nanos: i128 = time.parse().map_err(|_| ())?;
    let created_at = OffsetDateTime::from_unix_timestamp_nanos(nanos).map_err(|_| ())?;
    let id: Uuid = id.parse().map_err(|_| ())?;
    Ok(repo::TenantCursor {
        created_at,
        id: TenantId::from_uuid(id),
    })
}

/// Parses a tenant identifier from a path parameter.
fn parse_tenant_id(value: &str, request_id: &str) -> Result<TenantId, ApiError> {
    Uuid::parse_str(value)
        .map(TenantId::from_uuid)
        .map_err(|_| ApiError::invalid_argument(request_id))
}

/// Maps a write error to the API error. A duplicate name is a conflict.
fn map_write_error(request_id: &str, error: &restichub_db::DbError) -> ApiError {
    if error.is_unique_violation() {
        ApiError::conflict(request_id)
    } else {
        ApiError::internal(request_id, error)
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_cursor, encode_cursor};
    use restichub_db::TenantId;
    use restichub_db::tenants::TenantSummary;
    use time::OffsetDateTime;
    use uuid::Uuid;

    #[test]
    fn a_good_cursor_round_trips() {
        let tenant = TenantSummary {
            id: TenantId::new(),
            name: "acme".to_owned(),
            status: "active".to_owned(),
            created_at: OffsetDateTime::now_utc(),
            user_count: 1,
        };
        let decoded = decode_cursor(&encode_cursor(&tenant));
        assert_eq!(
            decoded.map(|cursor| (cursor.created_at, cursor.id)),
            Ok((tenant.created_at, tenant.id))
        );
    }

    #[test]
    fn a_bad_cursor_is_refused() {
        assert!(decode_cursor("nope").is_err());
        assert!(decode_cursor("0;not-a-uuid").is_err());
        let cursor = format!("{};{}", i128::MAX, Uuid::nil());
        assert!(decode_cursor(&cursor).is_err());
    }
}
