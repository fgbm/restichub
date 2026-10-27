//! The platform tenant administration endpoints (FR-TN-02).
//!
//! The Platform Admin of the platform tenant creates, lists, reads, suspends, resumes and deletes
//! the tenants. The endpoints return the count of the users only. They never return a user list or
//! a secret of another tenant.
//!
//! A suspended tenant cannot sign in and its API tokens fail (AC-TN-03). The agents of the tenant
//! stay connected the control plane does not close their connections. The deletion is a
//! soft delete in the MVP: the status becomes `deleted`, the sign-in and the tokens stop, and the
//! rows stay. A later retention task removes the rows of a deleted tenant.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use restichub_db::audit::{AuditAction, AuditEvent};
use restichub_db::tenants as repo;
use restichub_db::tenants::{TenantFilter, TenantPage, TenantSummary};
use restichub_db::{PLATFORM_TENANT_ID, TenantId, TenantStatus};
use restichub_secrets::{generate_dek, wrap_dek};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::api_path::ApiPath;
use crate::context::{Operation, RequestContext};
use crate::error::{ApiError, ErrorCode, map_write_error};
use crate::json::ApiJson;
use crate::server::AppState;
use crate::validation;
use crate::views::PlatformTenantView;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTenantRequest {
    /// Holds 1 to 63 characters from `a-z`, `0-9` and `-`, starts with a letter, and is not
    /// `platform`.
    pub name: String,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TenantListQuery {
    /// `active`, `suspended` or `deleted`.
    pub status: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TenantListResponse {
    pub items: Vec<PlatformTenantView>,
    /// Null when the table holds no later tenant.
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
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_tenants(
    State(state): State<AppState>,
    context: RequestContext,
    Query(query): Query<TenantListQuery>,
) -> Result<Json<TenantListResponse>, ApiError> {
    context.authorize(Operation::PlatformTenantsRead)?;
    let filter = filter(&query)?;
    let page = page(&query)?;
    let result = {
        let mut tx = state.db().begin_platform().await?;
        let result = repo::list_platform(&mut tx, &filter, &page).await?;
        tx.commit().await?;
        result
    };
    Ok(Json(response(result)))
}

#[utoipa::path(
    post,
    path = "/api/v1/platform/tenants",
    summary = "Creates a tenant by name only.",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateTenantRequest,
    responses(
        (status = 201, description = "The tenant is created.", body = PlatformTenantView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 409, description = "The name exists.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_tenant(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CreateTenantRequest>,
) -> Result<(StatusCode, Json<PlatformTenantView>), ApiError> {
    context.authorize(Operation::PlatformTenantsWrite)?;
    let name = validation::validate_tenant_name("name", &body.name)?;

    let kek = state
        .kek()
        .ok_or_else(|| ApiError::internal("the server holds no key encryption key provider"))?;

    // A fresh DEK, wrapped by the current KEK. The associated data holds the tenant id, so the id
    // comes first (ADR-0023, section Secrets).
    let tenant_id = TenantId::new();
    let dek = generate_dek()?;
    let wrapped = wrap_dek(kek.current(), tenant_id.as_uuid(), 1, &dek)?;

    // The create writes the tenant row, the key row and the `tenant_create` record only. The first
    // Admin is added through the platform user management (ADR-0028).
    let input = repo::BootstrapTenant {
        id: tenant_id,
        name: &name,
        key_version: 1,
        kek_id: wrapped.kek_id.as_str(),
        wrapped_dek: &wrapped.ciphertext,
        nonce: wrapped.nonce.as_slice(),
        actor: context.actor.clone(),
        source_ip: context.source_ip,
        admin: None,
    };

    let mut tx = state.db().begin_platform().await?;
    let created = repo::create_with_bootstrap(&mut tx, &input)
        .await
        .map_err(map_write_error)?;
    let summary = repo::get_platform(&mut tx, created).await?;
    tx.commit().await?;
    let Some(summary) = summary else {
        return Err(ApiError::internal(format!(
            "the tenant {created} is not found after the create"
        )));
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
        crate::error::ProtectedResponses,
        (status = 404, description = "The tenant is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_tenant(
    State(state): State<AppState>,
    context: RequestContext,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<PlatformTenantView>, ApiError> {
    context.authorize(Operation::PlatformTenantsRead)?;
    let tenant_id = parse_tenant_id(&id)?;
    let tenant = {
        let mut tx = state.db().begin_platform().await?;
        let tenant = repo::get_platform(&mut tx, tenant_id).await?;
        tx.commit().await?;
        tenant
    };
    match tenant {
        Some(tenant) => Ok(Json(PlatformTenantView::from(&tenant))),
        None => Err(ApiError::not_found()),
    }
}

/// Suspends a tenant.
///
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
        crate::error::ProtectedResponses,
        (status = 404, description = "The tenant is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The tenant is the platform tenant or it is deleted.", body = crate::error::ProblemBody)
    )
)]
pub async fn suspend_tenant(
    State(state): State<AppState>,
    context: RequestContext,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::PlatformTenantsWrite)?;
    let tenant_id = parse_tenant_id(&id)?;
    if tenant_id == PLATFORM_TENANT_ID {
        return Err(ApiError::protected(ErrorCode::PlatformTenantProtected));
    }
    let event = context.audit_event(AuditAction::TenantSuspend, "tenant", tenant_id.as_uuid());
    transition(
        &state,
        tenant_id,
        TenantStatus::Active,
        TenantStatus::Suspended,
        &event,
    )
    .await
}

/// Resumes a tenant.
///
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
        crate::error::ProtectedResponses,
        (status = 404, description = "The tenant is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The tenant is deleted.", body = crate::error::ProblemBody)
    )
)]
pub async fn resume_tenant(
    State(state): State<AppState>,
    context: RequestContext,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::PlatformTenantsWrite)?;
    let tenant_id = parse_tenant_id(&id)?;
    let event = context.audit_event(AuditAction::TenantResume, "tenant", tenant_id.as_uuid());
    transition(
        &state,
        tenant_id,
        TenantStatus::Suspended,
        TenantStatus::Active,
        &event,
    )
    .await
}

/// Deletes a tenant (soft delete).
///
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
        crate::error::ProtectedResponses,
        (status = 404, description = "The tenant is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The tenant is the platform tenant or it is not suspended.", body = crate::error::ProblemBody)
    )
)]
pub async fn delete_tenant(
    State(state): State<AppState>,
    context: RequestContext,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::PlatformTenantsWrite)?;
    let tenant_id = parse_tenant_id(&id)?;
    if tenant_id == PLATFORM_TENANT_ID {
        return Err(ApiError::protected(ErrorCode::PlatformTenantProtected));
    }
    let event = context.audit_event(AuditAction::TenantDelete, "tenant", tenant_id.as_uuid());
    transition(
        &state,
        tenant_id,
        TenantStatus::Suspended,
        TenantStatus::Deleted,
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
    tenant_id: TenantId,
    from: TenantStatus,
    to: TenantStatus,
    event: &AuditEvent,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.db().begin_platform().await?;
    let result = repo::set_status_from(&mut tx, tenant_id, from, to, event, event).await?;
    tx.commit().await?;
    match result {
        repo::Transition::Changed | repo::Transition::AlreadyInTarget => Ok(StatusCode::NO_CONTENT),
        repo::Transition::NotFound => Err(ApiError::not_found()),
        repo::Transition::Refused => Err(ApiError::conflict()),
    }
}

fn filter(query: &TenantListQuery) -> Result<TenantFilter, ApiError> {
    let status = match query.status.as_deref() {
        None => None,
        Some(value) => Some(
            TenantStatus::parse(value)
                .ok_or_else(|| ApiError::invalid_field("status", validation::Reason::Unknown))?,
        ),
    };
    Ok(TenantFilter { status })
}

fn page(query: &TenantListQuery) -> Result<TenantPage, ApiError> {
    let limit = query
        .limit
        .unwrap_or(repo::DEFAULT_PAGE_LIMIT)
        .clamp(1, repo::MAX_PAGE_LIMIT);
    let after = query
        .cursor
        .as_deref()
        .map(decode_cursor)
        .transpose()
        .map_err(|cause| {
            ApiError::invalid_field_cause("cursor", validation::Reason::InvalidFormat, cause)
        })?;
    Ok(TenantPage { limit, after })
}

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

fn encode_cursor(tenant: &TenantSummary) -> String {
    format!("{};{}", tenant.created_at.unix_timestamp_nanos(), tenant.id)
}

fn decode_cursor(value: &str) -> Result<repo::TenantCursor, String> {
    let (time, id) = value
        .split_once(';')
        .ok_or_else(|| "the cursor holds no `;`".to_owned())?;
    let nanos: i128 = time
        .parse()
        .map_err(|error: std::num::ParseIntError| error.to_string())?;
    let created_at =
        OffsetDateTime::from_unix_timestamp_nanos(nanos).map_err(|error| error.to_string())?;
    let id: Uuid = id.parse().map_err(|error: uuid::Error| error.to_string())?;
    Ok(repo::TenantCursor {
        created_at,
        id: TenantId::from_uuid(id),
    })
}

fn parse_tenant_id(value: &str) -> Result<TenantId, ApiError> {
    Uuid::parse_str(value)
        .map(TenantId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause(
                "id",
                validation::Reason::InvalidFormat,
                error.to_string(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::{decode_cursor, encode_cursor};
    use restichub_db::TenantId;
    use restichub_db::TenantStatus;
    use restichub_db::tenants::TenantSummary;
    use time::OffsetDateTime;
    use uuid::Uuid;

    #[test]
    fn a_good_cursor_round_trips() {
        let tenant = TenantSummary {
            id: TenantId::new(),
            name: "acme".to_owned(),
            status: TenantStatus::Active,
            created_at: OffsetDateTime::now_utc(),
            user_count: 1,
            agent_count: 0,
            admin_state: restichub_db::AdminState::Present,
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
