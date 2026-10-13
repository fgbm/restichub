//! The platform account endpoints for a Platform Admin (ADR-0025).
//!
//! Only a Platform Admin adds a membership for an existing account and disables an account. These
//! endpoints are platform-scoped: a tenant scope gets HTTP 403. The account row lock serializes an
//! administrator write of the same account with the membership insert.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::audit::{Actor, AuditAction, diff};
use restichub_db::tenants as tenant_repo;
use restichub_db::users::WriteStatus;
use restichub_db::{PLATFORM_TENANT_ID, Role, TenantId, TenantStatus, UserId, auth, users};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::server::AppState;
use crate::users::ADMIN_LOCK;
use crate::validation;

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddMembershipRequest {
    pub tenant_id: String,
    pub role: Role,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DisableAccountRequest {
    pub disabled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MembershipView {
    pub tenant_id: String,
    pub role: Role,
}

/// Adds a membership for an existing account (Platform Admin only).
///
/// The platform tenant is refused: a Platform Admin membership is created by the setup or by the
/// platform user create only.
#[utoipa::path(
    post,
    path = "/api/v1/platform/users/{id}/memberships",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the account.")),
    request_body = AddMembershipRequest,
    responses(
        (status = 201, description = "The membership is added.", body = MembershipView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The account or the tenant is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn add_membership(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<AddMembershipRequest>,
) -> Result<(StatusCode, Json<MembershipView>), ApiError> {
    context.authorize(Operation::PlatformUsersWrite)?;
    let user_id = parse_uuid(&id)?;
    let tenant_id = parse_tenant_id(&body.tenant_id)?;
    // The platform tenant holds the platform_admin role only.
    if body.role == Role::PlatformAdmin || !context.role.allows(body.role) {
        return Err(ApiError::forbidden());
    }

    if users::find_account_by_id(state.db(), user_id)
        .await?
        .is_none()
    {
        return Err(ApiError::not_found());
    }
    // The platform tenant is refused: a Platform Admin membership is created by the setup or by
    // the platform user create only.
    if tenant_id == PLATFORM_TENANT_ID {
        return Err(ApiError::forbidden());
    }
    let tenant = {
        let mut tx = state.db().begin_platform().await?;
        let tenant = tenant_repo::get_platform(&mut tx, tenant_id).await?;
        tx.commit().await?;
        tenant
    };
    let Some(tenant) = tenant else {
        return Err(ApiError::not_found());
    };
    if tenant.status != TenantStatus::Active {
        return Err(ApiError::forbidden());
    }

    // The database function takes the account row lock, then re-enables a disabled membership and
    // sets the role. It needs the platform scope, so the write runs in a platform transaction.
    let added = {
        let mut tx = state.db().begin_platform().await?;
        let added = auth::add_membership(&mut tx, user_id, tenant_id, body.role).await?;
        tx.commit().await?;
        added
    };
    if !added {
        return Err(ApiError::forbidden());
    }
    // The membership change is an audit record in the target tenant.
    let mut tx = state.db().begin_tenant(tenant_id).await?;
    let event = context.audit_event(AuditAction::UserUpdate, "user", user_id.as_uuid());
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(MembershipView {
            tenant_id: tenant_id.to_string(),
            role: body.role,
        }),
    ))
}

/// Disables or enables an account itself (Platform Admin only).
///
/// The endpoint refuses the caller's own account and the last active Platform Admin.
#[utoipa::path(
    patch,
    path = "/api/v1/platform/users/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the account.")),
    request_body = DisableAccountRequest,
    responses(
        (status = 204, description = "The account is changed."),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The account is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The account is the last active Platform Admin.", body = crate::error::ProblemBody)
    )
)]
pub async fn set_account_disabled(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<DisableAccountRequest>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::PlatformUsersWrite)?;
    let user_id = parse_uuid(&id)?;
    if context.actor == Actor::User(user_id) {
        return Err(ApiError::forbidden());
    }

    let mut tx = state.db().begin_tenant(PLATFORM_TENANT_ID).await?;
    // The lock serializes the last-Platform-Admin check with the write.
    tx.lock(ADMIN_LOCK).await?;
    // An active Platform Admin is an account that is not disabled with a membership that is not
    // disabled.
    let members = users::list_members(&mut tx).await?;
    let active_count = members
        .iter()
        .filter(|member| member.role.is_admin() && !member.disabled && !member.account_disabled)
        .count();
    let target_active = members.iter().any(|member| {
        member.id == user_id
            && member.role.is_admin()
            && !member.disabled
            && !member.account_disabled
    });
    if body.disabled && target_active && active_count <= 1 {
        return Err(ApiError::conflict());
    }
    match users::set_disabled(&mut tx, user_id, body.disabled).await? {
        WriteStatus::Ok => {}
        WriteStatus::NotFound => return Err(ApiError::not_found()),
        WriteStatus::Forbidden => return Err(ApiError::forbidden()),
        WriteStatus::Shared => return Err(ApiError::forbidden()),
    }
    if body.disabled {
        // A disabled account loses every session in every tenant.
        auth::delete_account_sessions(&mut tx, user_id, None).await?;
    }
    let mut event = context.audit_event(AuditAction::UserUpdate, "user", user_id.as_uuid());
    event.diff = Some(diff(
        Some(&serde_json::json!({"disabled": !body.disabled})),
        Some(&serde_json::json!({"disabled": body.disabled})),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

fn parse_uuid(value: &str) -> Result<UserId, ApiError> {
    Uuid::parse_str(value)
        .map(UserId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause(
                "id",
                validation::Reason::InvalidFormat,
                error.to_string(),
            )
        })
}

fn parse_tenant_id(value: &str) -> Result<TenantId, ApiError> {
    Uuid::parse_str(value)
        .map(TenantId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause(
                "tenant_id",
                validation::Reason::InvalidFormat,
                error.to_string(),
            )
        })
}
