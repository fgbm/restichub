//! The user management endpoints for the Admin role (issue #18, FR-AU-03).
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
use restichub_db::audit::{AuditAction, AuditEvent, diff};
use restichub_db::{UserId, memberships, sessions, users};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::context::{Operation, RequestContext, allowed_role, ensure_target_not_above_actor};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::password;
use crate::server::{AppState, RequestIdHeader};
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
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody)
    )
)]
pub async fn list_users(
    State(state): State<AppState>,
    context: RequestContext,
    request_id: RequestIdHeader,
) -> Result<Json<UserListResponse>, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::UsersRead, &request_id)?;
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let list = users::list(&mut tx)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let roles = memberships::list(&mut tx)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let items = list
        .iter()
        .map(|user| {
            let role = roles
                .iter()
                .find(|membership| membership.user_id == user.id)
                .map(|membership| membership.role.as_str())
                .unwrap_or("");
            UserView::new(user, role)
        })
        .collect();
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
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient or the role is too high.", body = crate::error::ProblemBody),
        (status = 409, description = "The username exists.", body = crate::error::ProblemBody),
        (status = 503, description = "The password work is busy.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_user(
    State(state): State<AppState>,
    context: RequestContext,
    request_id: RequestIdHeader,
    ApiJson(body): ApiJson<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserView>), ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::UsersWrite, &request_id)?;
    let role = allowed_role(&context, &body.role, &request_id)?;
    let username = validation::validate_username(&body.username)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;
    let display_name = validation::validate_display_name(&body.display_name)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;
    password::validate_password(&body.password)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;

    // Check the username in one transaction.
    {
        let mut tx = state
            .db()
            .begin_tenant(context.tenant_id)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        let existing = users::find_for_auth(&mut tx, &username)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        tx.commit()
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        if existing.is_some() {
            return Err(ApiError::conflict(&request_id));
        }
    }

    // Hash without a transaction.
    let password_hash =
        password::hash_password_limited(&state.auth().hash_semaphore, body.password.clone())
            .await
            .map_err(|error| ApiError::from_password(&request_id, &error))?;

    // Write the user under the tenant lock.
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.lock(ADMIN_LOCK)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let user_id = users::create(
        &mut tx,
        &users::NewUser {
            username: &username,
            password_hash: &password_hash,
            display_name: &display_name,
        },
    )
    .await
    .map_err(|error| map_write_error(&request_id, &error))?;
    memberships::set_role(&mut tx, user_id, role.as_str())
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let mut event = AuditEvent::new(AuditAction::UserCreate, "user");
    event.actor = context.actor.clone();
    event.object_id = Some(user_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({
            "username": username,
            "display_name": display_name,
            "role": role.as_str(),
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let user = users::get(&mut tx, user_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let Some(user) = user else {
        return Err(ApiError::internal_error(&request_id));
    };
    Ok((
        StatusCode::CREATED,
        Json(UserView::new(&user, role.as_str())),
    ))
}

/// Reads one user of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/users/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the user.")),
    responses(
        (status = 200, description = "The user.", body = UserView),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody),
        (status = 404, description = "The user is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_user(
    State(state): State<AppState>,
    context: RequestContext,
    request_id: RequestIdHeader,
    Path(id): Path<String>,
) -> Result<Json<UserView>, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::UsersRead, &request_id)?;
    let user_id = parse_uuid(&id, &request_id)?;
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let user = users::get(&mut tx, user_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let role = memberships::role_of(&mut tx, user_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    match user {
        Some(user) => Ok(Json(UserView::new(&user, role.as_deref().unwrap_or("")))),
        None => Err(ApiError::not_found(&request_id)),
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
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient or the target role is too high.", body = crate::error::ProblemBody),
        (status = 404, description = "The user is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The change removes the last Admin.", body = crate::error::ProblemBody)
    )
)]
pub async fn update_user(
    State(state): State<AppState>,
    context: RequestContext,
    request_id: RequestIdHeader,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateUserRequest>,
) -> Result<Json<UserView>, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::UsersWrite, &request_id)?;
    let user_id = parse_uuid(&id, &request_id)?;
    let new_role = match &body.role {
        Some(role) => Some(allowed_role(&context, role, &request_id)?),
        None => None,
    };
    let display_name = match &body.display_name {
        Some(value) => Some(
            validation::validate_display_name(value)
                .map_err(|_| ApiError::invalid_argument(&request_id))?,
        ),
        None => None,
    };
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    // The lock serializes the last-admin check with the write.
    tx.lock(ADMIN_LOCK)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let user = users::get(&mut tx, user_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let Some(user) = user else {
        return Err(ApiError::not_found(&request_id));
    };
    let current_role = memberships::role_of(&mut tx, user_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    if let Some(role) = &current_role {
        ensure_target_not_above_actor(&context, role, &request_id)?;
    }
    // The guard applies to an active Admin only. A disabled Admin is not the last Admin.
    let active_admin = current_role.as_deref().is_some_and(is_admin) && !user.disabled;
    let role_lost_admin = new_role.is_some_and(|role| !is_admin(role.as_str()));
    let disabling = body.disabled == Some(true);
    if active_admin && (role_lost_admin || disabling) {
        guard_last_admin(&mut tx, &request_id).await?;
    }
    if let Some(display_name) = &display_name {
        users::set_display_name(&mut tx, user_id, display_name)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
    }
    if let Some(disabled) = body.disabled {
        users::set_disabled(&mut tx, user_id, disabled)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        if disabled {
            sessions::delete_for_user(&mut tx, user_id)
                .await
                .map_err(|error| ApiError::internal(&request_id, &error))?;
        }
    }
    if let Some(role) = new_role {
        memberships::set_role(&mut tx, user_id, role.as_str())
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
    }
    let updated = users::get(&mut tx, user_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let final_role = memberships::role_of(&mut tx, user_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let before = serde_json::json!({
        "display_name": user.display_name,
        "disabled": user.disabled,
        "role": current_role,
    });
    let after = serde_json::json!({
        "display_name": updated.as_ref().map(|user| user.display_name.clone()),
        "disabled": updated.as_ref().map(|user| user.disabled),
        "role": final_role,
    });
    let mut event = AuditEvent::new(AuditAction::UserUpdate, "user");
    event.actor = context.actor.clone();
    event.object_id = Some(user_id.as_uuid());
    event.diff = Some(diff(Some(&before), Some(&after), &[]));
    restichub_db::audit::insert(&mut tx, &event)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    match updated {
        Some(user) => Ok(Json(UserView::new(
            &user,
            final_role.as_deref().unwrap_or(""),
        ))),
        None => Err(ApiError::internal_error(&request_id)),
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
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient or the target role is too high.", body = crate::error::ProblemBody),
        (status = 404, description = "The user is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The user is the last Admin.", body = crate::error::ProblemBody)
    )
)]
pub async fn delete_user(
    State(state): State<AppState>,
    context: RequestContext,
    request_id: RequestIdHeader,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::UsersWrite, &request_id)?;
    let user_id = parse_uuid(&id, &request_id)?;
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.lock(ADMIN_LOCK)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let user = users::get(&mut tx, user_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let Some(user) = user else {
        return Err(ApiError::not_found(&request_id));
    };
    let current_role = memberships::role_of(&mut tx, user_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    if let Some(role) = &current_role {
        ensure_target_not_above_actor(&context, role, &request_id)?;
    }
    // The guard applies to an active Admin only. A disabled Admin is not the last Admin.
    let active_admin = current_role.as_deref().is_some_and(is_admin) && !user.disabled;
    if active_admin {
        guard_last_admin(&mut tx, &request_id).await?;
    }
    users::set_disabled(&mut tx, user_id, true)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    sessions::delete_for_user(&mut tx, user_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let mut event = AuditEvent::new(AuditAction::UserDelete, "user");
    event.actor = context.actor.clone();
    event.object_id = Some(user_id.as_uuid());
    event.diff = Some(diff(
        Some(&serde_json::json!({"disabled": user.disabled})),
        Some(&serde_json::json!({"disabled": true})),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
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
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient or the target role is too high.", body = crate::error::ProblemBody),
        (status = 404, description = "The user is not found.", body = crate::error::ProblemBody),
        (status = 503, description = "The password work is busy.", body = crate::error::ProblemBody)
    )
)]
pub async fn reset_password(
    State(state): State<AppState>,
    context: RequestContext,
    request_id: RequestIdHeader,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ResetPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::UsersWrite, &request_id)?;
    let user_id = parse_uuid(&id, &request_id)?;
    password::validate_password(&body.new_password)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;

    // Read the target.
    let (user, target_role) = {
        let mut tx = state
            .db()
            .begin_tenant(context.tenant_id)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        let user = users::get(&mut tx, user_id)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        let role = memberships::role_of(&mut tx, user_id)
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        tx.commit()
            .await
            .map_err(|error| ApiError::internal(&request_id, &error))?;
        (user, role)
    };
    if user.is_none() {
        return Err(ApiError::not_found(&request_id));
    }
    if let Some(role) = &target_role {
        ensure_target_not_above_actor(&context, role, &request_id)?;
    }

    // Hash without a transaction.
    let password_hash =
        password::hash_password_limited(&state.auth().hash_semaphore, body.new_password.clone())
            .await
            .map_err(|error| ApiError::from_password(&request_id, &error))?;

    // Write the new hash.
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    users::set_password(&mut tx, user_id, &password_hash)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    sessions::delete_for_user(&mut tx, user_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let mut event = AuditEvent::new(AuditAction::UserUpdate, "user");
    event.actor = context.actor.clone();
    event.object_id = Some(user_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({"password": "<reset>"})),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Refuses the change when it would remove the last active Admin of the tenant.
async fn guard_last_admin(
    tx: &mut restichub_db::TenantTx<'_>,
    request_id: &str,
) -> Result<(), ApiError> {
    let count = memberships::count_admins(tx)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?;
    if count <= 1 {
        return Err(ApiError::conflict(request_id));
    }
    Ok(())
}

/// Returns true for an administrator role.
fn is_admin(role: &str) -> bool {
    role == "admin" || role == "platform_admin"
}

/// Maps a write error to the API error.
fn map_write_error(request_id: &str, error: &restichub_db::DbError) -> ApiError {
    if error.is_unique_violation() {
        ApiError::conflict(request_id)
    } else {
        ApiError::internal(request_id, error)
    }
}

/// Parses a UUID from a path parameter.
fn parse_uuid(value: &str, request_id: &str) -> Result<UserId, ApiError> {
    Uuid::parse_str(value)
        .map(UserId::from_uuid)
        .map_err(|_| ApiError::invalid_argument(request_id))
}
