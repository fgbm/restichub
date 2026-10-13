//! The platform account endpoints for a Platform Admin (ADR-0028).
//!
//! The Platform Admin creates an installation account, lists every account with its memberships,
//! adds a membership for an existing account, changes the role of a membership, removes a
//! membership and changes an account. These endpoints are platform-scoped: a tenant scope gets
//! HTTP 403. The account row lock serializes an administrator write of the same account with the
//! membership write.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::audit::{Actor, AuditAction, diff};
use restichub_db::tenants as tenant_repo;
use restichub_db::users::{CreateAccountStatus, NewPlatformAccount, WriteStatus};
use restichub_db::{
    DbError, PLATFORM_TENANT_ID, Role, TenantId, TenantStatus, UserId, auth, memberships, users,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::password;
use crate::server::AppState;
use crate::users::ADMIN_LOCK;
use crate::validation;
use crate::views::{PlatformMembershipView, PlatformUserView};

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddMembershipRequest {
    pub tenant_id: String,
    pub role: Role,
}

/// One membership of the account create request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MembershipInput {
    pub tenant_id: String,
    pub role: Role,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAccountRequest {
    /// Unique in the installation; ignores the letter case.
    pub username: String,
    pub display_name: String,
    /// At least 12 characters.
    pub password: String,
    /// Unique in the installation when it is set (ADR-0028).
    #[serde(default)]
    pub email: Option<String>,
    /// Grants the installation-wide right at the create (ADR-0029).
    #[serde(default)]
    pub platform_admin: bool,
    /// The organizations of the account with the role in each one.
    #[serde(default)]
    pub memberships: Vec<MembershipInput>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetRoleRequest {
    pub role: Role,
}

/// Absent fields leave the value unchanged.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAccountRequest {
    pub disabled: Option<bool>,
    /// Written by the update; the MVP treats it as unverified.
    pub email: Option<String>,
    /// True when the account holds the installation-wide right (ADR-0029).
    pub platform_admin: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MembershipView {
    pub tenant_id: String,
    pub role: Role,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PlatformUserListResponse {
    pub items: Vec<PlatformUserView>,
}

/// Lists every account of the installation with its memberships (Platform Admin only).
#[utoipa::path(
    get,
    path = "/api/v1/platform/users",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "Every account of the installation.", body = PlatformUserListResponse),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_accounts(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<PlatformUserListResponse>, ApiError> {
    context.authorize(Operation::PlatformUsersRead)?;
    let (accounts, member_rows) = {
        let mut tx = state.db().begin_platform().await?;
        let accounts = users::list_platform_accounts(&mut tx).await?;
        let member_rows = memberships::list_all(&mut tx).await?;
        tx.commit().await?;
        (accounts, member_rows)
    };
    let mut by_user: HashMap<UserId, Vec<PlatformMembershipView>> = HashMap::new();
    for member in &member_rows {
        by_user
            .entry(member.user_id)
            .or_default()
            .push(PlatformMembershipView::from(member));
    }
    let items = accounts
        .iter()
        .map(|account| {
            PlatformUserView::new(account, by_user.remove(&account.id).unwrap_or_default())
        })
        .collect();
    Ok(Json(PlatformUserListResponse { items }))
}

/// Creates an installation account with optional memberships (Platform Admin only).
///
/// The username and the email are unique in the installation. A clash gives HTTP 409
/// `username_taken` or `email_taken`. The account create and each membership commit together.
#[utoipa::path(
    post,
    path = "/api/v1/platform/users",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateAccountRequest,
    responses(
        (status = 201, description = "The account is created.", body = PlatformUserView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "A membership names an unknown organization.", body = crate::error::ProblemBody),
        (status = 409, description = "The username or the email exists.", body = crate::error::ProblemBody),
        (status = 503, description = "The password work is busy.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_account(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CreateAccountRequest>,
) -> Result<(StatusCode, Json<PlatformUserView>), ApiError> {
    context.authorize(Operation::PlatformUsersWrite)?;
    let username = validation::validate_username("username", &body.username)?;
    let display_name = validation::validate_display_name("display_name", &body.display_name)?;
    validation::validate_password("password", &body.password)?;
    let email = match &body.email {
        Some(value) => Some(validation::validate_email("email", value)?),
        None => None,
    };
    // A membership names a role below the Platform Admin (the platform tenant is refused).
    let mut requested: Vec<(TenantId, Role)> = Vec::with_capacity(body.memberships.len());
    for input in &body.memberships {
        let tenant_id = parse_tenant_id(&input.tenant_id)?;
        if input.role == Role::PlatformAdmin || !context.role.allows(input.role) {
            return Err(ApiError::forbidden());
        }
        requested.push((tenant_id, input.role));
    }

    let password_hash =
        password::hash_password_limited(&state.auth().hash_semaphore, body.password.clone())
            .await?;
    let user_id = UserId::new();

    let mut tx = state.db().begin_platform().await?;
    // Every membership names an active organization that is not the platform tenant.
    let mut resolved: Vec<(TenantId, String, Role)> = Vec::with_capacity(requested.len());
    for (tenant_id, role) in requested {
        if tenant_id == PLATFORM_TENANT_ID {
            return Err(ApiError::forbidden());
        }
        let tenant = tenant_repo::get_platform(&mut tx, tenant_id).await?;
        match tenant {
            Some(tenant) if tenant.status == TenantStatus::Active => {
                resolved.push((tenant_id, tenant.name, role));
            }
            Some(_) => return Err(ApiError::conflict()),
            None => return Err(ApiError::not_found()),
        }
    }

    let status = users::platform_create_account(
        &mut tx,
        user_id,
        &NewPlatformAccount {
            username: &username,
            password_hash: &password_hash,
            display_name: &display_name,
            email: email.as_deref(),
            platform_admin: body.platform_admin,
        },
    )
    .await?;
    match status {
        CreateAccountStatus::Ok => {}
        CreateAccountStatus::UsernameTaken => return Err(ApiError::username_taken()),
        CreateAccountStatus::EmailTaken => return Err(ApiError::email_taken()),
        CreateAccountStatus::Forbidden => return Err(ApiError::forbidden()),
    }
    for (tenant_id, _, role) in &resolved {
        let added = auth::add_membership(&mut tx, user_id, *tenant_id, *role).await?;
        if !added {
            return Err(ApiError::internal(
                "the membership add is refused after the account create",
            ));
        }
    }

    // The account create is an audit record in the platform tenant (ADR-0025).
    let memberships_json: Vec<serde_json::Value> = resolved
        .iter()
        .map(|(tenant_id, _, role)| {
            serde_json::json!({"tenant_id": tenant_id.to_string(), "role": role.as_str()})
        })
        .collect();
    let mut event = context.audit_event(AuditAction::UserCreate, "user", user_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({
            "username": username,
            "display_name": display_name,
            "email": email,
            "platform_admin": body.platform_admin,
            "memberships": memberships_json,
        })),
        &[],
    ));
    restichub_db::audit::insert_platform(&mut tx, PLATFORM_TENANT_ID, &event).await?;
    // A membership change is an audit record in that organization (ADR-0025).
    for (tenant_id, _, _) in &resolved {
        let mut member_event =
            context.audit_event(AuditAction::UserUpdate, "user", user_id.as_uuid());
        member_event.diff = Some(diff(None, Some(&serde_json::json!({"added": true})), &[]));
        restichub_db::audit::insert_platform_for(&mut tx, *tenant_id, &member_event).await?;
    }
    tx.commit().await?;

    let account = users::PlatformAccount {
        id: user_id,
        username: username.clone(),
        display_name: display_name.clone(),
        email: email.clone(),
        email_verified: false,
        disabled: false,
        platform_admin: body.platform_admin,
    };
    let view_memberships = resolved
        .iter()
        .map(|(tenant_id, tenant_name, role)| PlatformMembershipView {
            tenant_id: tenant_id.to_string(),
            tenant_name: tenant_name.clone(),
            role: *role,
            disabled: false,
        })
        .collect();
    Ok((
        StatusCode::CREATED,
        Json(PlatformUserView::new(&account, view_memberships)),
    ))
}

/// Adds a membership for an existing account (Platform Admin only).
///
/// The platform tenant is refused: a Platform Admin membership is created by the setup only.
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
    // The platform tenant is refused: a Platform Admin membership is created by the setup only.
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
    // The self-add of a membership is an audit record in the platform audit too (ADR-0029).
    let mut tx = state.db().begin_platform().await?;
    let mut platform_event =
        context.audit_event(AuditAction::UserUpdate, "user", user_id.as_uuid());
    platform_event.diff = Some(diff(
        None,
        Some(&serde_json::json!({
            "tenant_id": tenant_id.to_string(),
            "role": body.role.as_str(),
        })),
        &[],
    ));
    restichub_db::audit::insert_platform(&mut tx, PLATFORM_TENANT_ID, &platform_event).await?;
    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(MembershipView {
            tenant_id: tenant_id.to_string(),
            role: body.role,
        }),
    ))
}

/// Changes the role of an existing membership (Platform Admin only).
///
/// Refuses the platform tenant.
#[utoipa::path(
    patch,
    path = "/api/v1/platform/users/{id}/memberships/{tenant_id}",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = String, Path, description = "The identifier of the account."),
        ("tenant_id" = String, Path, description = "The identifier of the organization.")
    ),
    request_body = SetRoleRequest,
    responses(
        (status = 204, description = "The role is changed."),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The membership is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn set_membership_role(
    State(state): State<AppState>,
    context: RequestContext,
    Path((id, tenant_id)): Path<(String, String)>,
    ApiJson(body): ApiJson<SetRoleRequest>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::PlatformUsersWrite)?;
    let user_id = parse_uuid(&id)?;
    let tenant_id = parse_tenant_id(&tenant_id)?;
    if body.role == Role::PlatformAdmin || !context.role.allows(body.role) {
        return Err(ApiError::forbidden());
    }
    let mut tx = state.db().begin_platform().await?;
    match memberships::set_role_platform(&mut tx, user_id, tenant_id, body.role).await? {
        WriteStatus::Ok => {}
        WriteStatus::NotFound => return Err(ApiError::not_found()),
        WriteStatus::Forbidden | WriteStatus::Shared => return Err(ApiError::forbidden()),
    }
    // A membership change is an audit record in that organization (ADR-0025).
    let mut event = context.audit_event(AuditAction::UserUpdate, "user", user_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({"role": body.role.as_str()})),
        &[],
    ));
    restichub_db::audit::insert_platform_for(&mut tx, tenant_id, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a membership of an account (Platform Admin only).
///
/// Refuses the platform tenant. Keeps the last active Platform Admin and the caller's own
/// membership.
#[utoipa::path(
    delete,
    path = "/api/v1/platform/users/{id}/memberships/{tenant_id}",
    security(("cookie" = []), ("bearer" = [])),
    params(
        ("id" = String, Path, description = "The identifier of the account."),
        ("tenant_id" = String, Path, description = "The identifier of the organization.")
    ),
    responses(
        (status = 204, description = "The membership is removed."),
        crate::error::ProtectedResponses,
        (status = 404, description = "The membership is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The membership is the caller's own membership.", body = crate::error::ProblemBody)
    )
)]
pub async fn remove_membership(
    State(state): State<AppState>,
    context: RequestContext,
    Path((id, tenant_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::PlatformUsersWrite)?;
    let user_id = parse_uuid(&id)?;
    let tenant_id = parse_tenant_id(&tenant_id)?;
    // The caller cannot remove its own membership in the organization of its session (ADR-0025).
    if context.actor == Actor::User(user_id) && tenant_id == context.tenant_id {
        return Err(ApiError::conflict());
    }
    let mut tx = state.db().begin_platform().await?;
    match memberships::remove_platform(&mut tx, user_id, tenant_id).await? {
        WriteStatus::Ok => {}
        WriteStatus::NotFound => return Err(ApiError::not_found()),
        WriteStatus::Forbidden | WriteStatus::Shared => return Err(ApiError::forbidden()),
    }
    // A membership change is an audit record in that organization (ADR-0025).
    let mut event = context.audit_event(AuditAction::UserUpdate, "user", user_id.as_uuid());
    event.diff = Some(diff(
        Some(&serde_json::json!({"membership": tenant_id.to_string()})),
        None,
        &[],
    ));
    restichub_db::audit::insert_platform_for(&mut tx, tenant_id, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Changes an account itself (Platform Admin only).
///
/// The endpoint refuses the caller's own account and the last active Platform Admin. At least one
/// field must be present.
#[utoipa::path(
    patch,
    path = "/api/v1/platform/users/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the account.")),
    request_body = UpdateAccountRequest,
    responses(
        (status = 204, description = "The account is changed."),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The account is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The account is the last active Platform Admin or the email exists.", body = crate::error::ProblemBody)
    )
)]
pub async fn update_account(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateAccountRequest>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::PlatformUsersWrite)?;
    let user_id = parse_uuid(&id)?;
    if body.disabled.is_none() && body.email.is_none() && body.platform_admin.is_none() {
        return Err(ApiError::invalid_field(
            "body",
            validation::Reason::InvalidBody,
        ));
    }
    // The caller cannot disable its own account and cannot remove its own right (ADR-0029).
    if context.actor == Actor::User(user_id)
        && (body.disabled == Some(true) || body.platform_admin == Some(false))
    {
        return Err(ApiError::forbidden());
    }
    let email = match &body.email {
        Some(value) => Some(validation::validate_email("email", value)?),
        None => None,
    };

    let mut tx = state.db().begin_tenant(PLATFORM_TENANT_ID).await?;
    // The lock serializes the last-Platform-Admin check with the write.
    tx.lock(ADMIN_LOCK).await?;
    let target_active = users::is_platform_admin(&mut tx, user_id).await?;
    let active_count = users::platform_admin_count(&mut tx).await?;
    // The last active Platform Admin cannot lose the right and cannot be disabled (ADR-0029).
    let removes_right = body.platform_admin == Some(false);
    let disables = body.disabled == Some(true);
    if target_active && active_count <= 1 && (removes_right || disables) {
        return Err(ApiError::conflict());
    }
    if let Some(value) = body.platform_admin {
        match users::set_platform_admin(&mut tx, user_id, value).await? {
            WriteStatus::Ok => {}
            WriteStatus::NotFound => return Err(ApiError::not_found()),
            WriteStatus::Forbidden | WriteStatus::Shared => return Err(ApiError::forbidden()),
        }
    }
    if let Some(email) = &email {
        match users::platform_set_email(&mut tx, user_id, email)
            .await
            .map_err(map_email_taken)?
        {
            WriteStatus::Ok => {}
            WriteStatus::NotFound => return Err(ApiError::not_found()),
            WriteStatus::Forbidden | WriteStatus::Shared => return Err(ApiError::forbidden()),
        }
    }
    if let Some(disabled) = body.disabled {
        match users::set_disabled(&mut tx, user_id, disabled).await? {
            WriteStatus::Ok => {}
            WriteStatus::NotFound => return Err(ApiError::not_found()),
            WriteStatus::Forbidden | WriteStatus::Shared => return Err(ApiError::forbidden()),
        }
        if disabled {
            // A disabled account loses every session in every tenant.
            auth::delete_account_sessions(&mut tx, user_id, None).await?;
        }
    }
    let mut event = context.audit_event(AuditAction::UserUpdate, "user", user_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({
            "disabled": body.disabled,
            "email": email,
            "platform_admin": body.platform_admin,
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A duplicate email is HTTP 409 `email_taken` (ADR-0028).
fn map_email_taken(error: DbError) -> ApiError {
    if error.is_unique_violation() {
        ApiError::email_taken()
    } else {
        ApiError::from(error)
    }
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
