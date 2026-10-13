//! The user management endpoints for the Admin role (FR-AU-03).
//!
//! An Admin manages the users of the tenant. An Admin cannot remove the last active Admin of a
//! tenant and cannot grant the role `platform_admin` unless the tenant is the platform tenant.
//!
//! The DELETE endpoint disables the user and removes its sessions. An API token is owned by the
//! tenant, not by the user, so the tokens stay. They stop working only when the tenant is not
//! active.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::audit::{AuditAction, diff};
use restichub_db::{Role, TenantTx, UserId, memberships, sessions, users};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::context::{Operation, RequestContext, allowed_role, ensure_target_not_above_actor};
use crate::error::{ApiError, map_write_error};
use crate::json::ApiJson;
use crate::password;
use crate::server::AppState;
use crate::validation;
use crate::views::UserView;

/// The advisory lock scope of the tenant user administration.
const ADMIN_LOCK: &str = "restichub.tenant_admin:";

/// The body of the create-user request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    /// The username. It is unique for the tenant and ignores the letter case.
    pub username: String,
    /// The password. It must hold at least 12 characters.
    pub password: String,
    /// The name for the user interface.
    pub display_name: String,
    /// The role in the tenant.
    pub role: String,
}

/// The body of the update-user request. An absent field does not change.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUserRequest {
    /// The new display name.
    pub display_name: Option<String>,
    /// The new disabled flag.
    pub disabled: Option<bool>,
    /// The new role.
    pub role: Option<String>,
}

/// The body of the admin password reset request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ResetPasswordRequest {
    /// The new password. It must hold at least 12 characters.
    pub new_password: String,
}

/// A list of users.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserListResponse {
    /// The users.
    pub items: Vec<UserView>,
}

/// Lists the users of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/users",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The users of the tenant.", body = UserListResponse),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_users(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<UserListResponse>, ApiError> {
    context.authorize(Operation::UsersRead)?;
    let (list, roles) = context
        .tenant_tx(&state, async |tx| {
            let list = users::list(tx).await?;
            let roles = memberships::list(tx).await?;
            Ok((list, roles))
        })
        .await?;
    let mut items = Vec::with_capacity(list.len());
    for user in &list {
        let role = roles
            .iter()
            .find(|membership| membership.user_id == user.id)
            .map(|membership| membership.role)
            .ok_or_else(|| {
                ApiError::internal(format!("the user {} holds no membership", user.id))
            })?;
        items.push(UserView::new(user, role));
    }
    Ok(Json(UserListResponse { items }))
}

/// Creates a user in the tenant.
#[utoipa::path(
    post,
    path = "/api/v1/users",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "The user is created.", body = UserView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 409, description = "The username exists.", body = crate::error::ProblemBody),
        (status = 503, description = "The password work is busy.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_user(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserView>), ApiError> {
    context.authorize(Operation::UsersWrite)?;
    let role = allowed_role(&context, &body.role)?;
    let username = validation::validate_username("username", &body.username)?;
    let display_name = validation::validate_display_name("display_name", &body.display_name)?;
    validation::validate_password("password", &body.password)?;

    // Check the username in one transaction.
    {
        let mut tx = state.db().begin_tenant(context.tenant_id).await?;
        let existing = users::find_for_auth(&mut tx, &username).await?;
        tx.commit().await?;
        if existing.is_some() {
            return Err(ApiError::conflict());
        }
    }

    // Hash without a transaction.
    let password_hash =
        password::hash_password_limited(&state.auth().hash_semaphore, body.password.clone())
            .await?;

    // Write the user under the tenant lock.
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    tx.lock(ADMIN_LOCK).await?;
    let user_id = users::create(
        &mut tx,
        &users::NewUser {
            username: &username,
            password_hash: &password_hash,
            display_name: &display_name,
        },
    )
    .await
    .map_err(map_write_error)?;
    memberships::set_role(&mut tx, user_id, role).await?;
    let mut event = context.audit_event(AuditAction::UserCreate, "user", user_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({
            "username": username,
            "display_name": display_name,
            "role": role.as_str(),
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    let user = users::get(&mut tx, user_id).await?;
    tx.commit().await?;
    let Some(user) = user else {
        return Err(ApiError::internal(format!(
            "the user {user_id} is not found after the create"
        )));
    };
    Ok((StatusCode::CREATED, Json(UserView::new(&user, role))))
}

/// Reads one user of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/users/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the user.")),
    responses(
        (status = 200, description = "The user.", body = UserView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The user is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_user(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<UserView>, ApiError> {
    context.authorize(Operation::UsersRead)?;
    let user_id = parse_uuid(&id)?;
    let (user, role) = context
        .tenant_tx(&state, async |tx| {
            let user = users::get(tx, user_id).await?;
            let role = memberships::role_of(tx, user_id).await?;
            Ok((user, role))
        })
        .await?;
    match user {
        Some(user) => Ok(Json(UserView::new(
            &user,
            role.ok_or_else(|| {
                ApiError::internal(format!("the user {user_id} holds no membership"))
            })?,
        ))),
        None => Err(ApiError::not_found()),
    }
}

/// Changes a user of the tenant.
#[utoipa::path(
    patch,
    path = "/api/v1/users/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the user.")),
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "The changed user.", body = UserView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The user is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The change removes the last Admin.", body = crate::error::ProblemBody)
    )
)]
pub async fn update_user(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateUserRequest>,
) -> Result<Json<UserView>, ApiError> {
    context.authorize(Operation::UsersWrite)?;
    let user_id = parse_uuid(&id)?;
    let new_role = match &body.role {
        Some(role) => Some(allowed_role(&context, role)?),
        None => None,
    };
    let display_name = match &body.display_name {
        Some(value) => Some(validation::validate_display_name("display_name", value)?),
        None => None,
    };
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    // The lock serializes the last-admin check with the write.
    tx.lock(ADMIN_LOCK).await?;
    let user = users::get(&mut tx, user_id).await?;
    let Some(user) = user else {
        return Err(ApiError::not_found());
    };
    let current_role = memberships::role_of(&mut tx, user_id).await?;
    if let Some(role) = current_role {
        ensure_target_not_above_actor(&context, role)?;
    }
    // The guard applies to an active Admin only. A disabled Admin is not the last Admin.
    let active_admin = current_role.is_some_and(Role::is_admin) && !user.disabled;
    let role_lost_admin = new_role.is_some_and(|role| !role.is_admin());
    let disabling = body.disabled == Some(true);
    if active_admin && (role_lost_admin || disabling) {
        guard_last_admin(&mut tx).await?;
    }
    if let Some(display_name) = &display_name {
        users::set_display_name(&mut tx, user_id, display_name).await?;
    }
    if let Some(disabled) = body.disabled {
        users::set_disabled(&mut tx, user_id, disabled).await?;
        if disabled {
            sessions::delete_for_user(&mut tx, user_id).await?;
        }
    }
    if let Some(role) = new_role {
        memberships::set_role(&mut tx, user_id, role).await?;
    }
    let updated = users::get(&mut tx, user_id).await?;
    let final_role = memberships::role_of(&mut tx, user_id).await?;
    let before = serde_json::json!({
        "display_name": user.display_name,
        "disabled": user.disabled,
        "role": current_role.map(Role::as_str),
    });
    let after = serde_json::json!({
        "display_name": updated.as_ref().map(|user| user.display_name.clone()),
        "disabled": updated.as_ref().map(|user| user.disabled),
        "role": final_role.map(Role::as_str),
    });
    let mut event = context.audit_event(AuditAction::UserUpdate, "user", user_id.as_uuid());
    event.diff = Some(diff(Some(&before), Some(&after), &[]));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    match updated {
        Some(user) => Ok(Json(UserView::new(
            &user,
            final_role.ok_or_else(|| {
                ApiError::internal(format!("the user {user_id} holds no membership"))
            })?,
        ))),
        None => Err(ApiError::internal(format!(
            "the user {user_id} is not found after the update"
        ))),
    }
}

/// Disables a user of the tenant and removes its sessions.
#[utoipa::path(
    delete,
    path = "/api/v1/users/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the user.")),
    responses(
        (status = 204, description = "The user is disabled."),
        crate::error::ProtectedResponses,
        (status = 404, description = "The user is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The user is the last Admin.", body = crate::error::ProblemBody)
    )
)]
pub async fn delete_user(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::UsersWrite)?;
    let user_id = parse_uuid(&id)?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    tx.lock(ADMIN_LOCK).await?;
    let user = users::get(&mut tx, user_id).await?;
    let Some(user) = user else {
        return Err(ApiError::not_found());
    };
    let current_role = memberships::role_of(&mut tx, user_id).await?;
    if let Some(role) = current_role {
        ensure_target_not_above_actor(&context, role)?;
    }
    // The guard applies to an active Admin only. A disabled Admin is not the last Admin.
    let active_admin = current_role.is_some_and(Role::is_admin) && !user.disabled;
    if active_admin {
        guard_last_admin(&mut tx).await?;
    }
    users::set_disabled(&mut tx, user_id, true).await?;
    sessions::delete_for_user(&mut tx, user_id).await?;
    let mut event = context.audit_event(AuditAction::UserDelete, "user", user_id.as_uuid());
    event.diff = Some(diff(
        Some(&serde_json::json!({"disabled": user.disabled})),
        Some(&serde_json::json!({"disabled": true})),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Resets the password of a user of the tenant (Admin action).
#[utoipa::path(
    post,
    path = "/api/v1/users/{id}/password",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the user.")),
    request_body = ResetPasswordRequest,
    responses(
        (status = 204, description = "The password is changed."),
        (status = 400, description = "The new password is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The user is not found.", body = crate::error::ProblemBody),
        (status = 503, description = "The password work is busy.", body = crate::error::ProblemBody)
    )
)]
pub async fn reset_password(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ResetPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::UsersWrite)?;
    let user_id = parse_uuid(&id)?;
    validation::validate_password("new_password", &body.new_password)?;

    // Read the target.
    let (user, target_role) = context
        .tenant_tx(&state, async |tx| {
            let user = users::get(tx, user_id).await?;
            let role = memberships::role_of(tx, user_id).await?;
            Ok((user, role))
        })
        .await?;
    if user.is_none() {
        return Err(ApiError::not_found());
    }
    if let Some(role) = target_role {
        ensure_target_not_above_actor(&context, role)?;
    }

    // Hash without a transaction.
    let password_hash =
        password::hash_password_limited(&state.auth().hash_semaphore, body.new_password.clone())
            .await?;

    // Write the new hash.
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    users::set_password(&mut tx, user_id, &password_hash).await?;
    sessions::delete_for_user(&mut tx, user_id).await?;
    let mut event = context.audit_event(AuditAction::UserUpdate, "user", user_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({"password": "<reset>"})),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Refuses the change when it would remove the last active Admin of the tenant.
async fn guard_last_admin(tx: &mut TenantTx<'_>) -> Result<(), ApiError> {
    if memberships::count_admins(tx).await? <= 1 {
        return Err(ApiError::conflict());
    }
    Ok(())
}

/// Parses a UUID from a path parameter.
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
