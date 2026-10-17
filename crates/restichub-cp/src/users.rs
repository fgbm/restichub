//! The user management endpoints for the Admin role (ADR-0025).
//!
//! An account is installation-wide. A tenant Admin manages the membership of an account in its
//! tenant: it creates an account with a membership, changes the role, disables the membership and
//! reads the accounts of its members only. It cannot disable the account itself, cannot reset the
//! password or the display name of a shared account, and cannot add an existing foreign account.
//!
//! The DELETE endpoint disables the membership and removes the sessions of the account in the
//! tenant. A Platform Admin manages the account itself through the platform routes.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::audit::{Actor, AuditAction, AuditEvent, diff};
use restichub_db::users::{AccountField, WriteStatus};
use restichub_db::{
    PLATFORM_TENANT_ID, Role, TenantTx, UserId, auth, memberships, sessions, users,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::context::{Operation, RequestContext, allowed_role, ensure_target_not_above_actor};
use crate::error::ApiError;
use crate::jobs::format_time;
use crate::json::ApiJson;
use crate::password;
use crate::server::AppState;
use crate::validation;
use crate::views::UserView;

/// The advisory lock that serializes a change of the administrator set of a tenant. The platform
/// routes take the same lock in the platform tenant, so the two paths serialize there (ADR-0025).
pub(crate) const ADMIN_LOCK: &str = "restichub.admin:";

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    /// Unique in the installation; ignores the letter case.
    pub username: String,
    /// At least 12 characters.
    pub password: String,
    pub display_name: String,
    pub role: Role,
}

/// Absent fields leave the value unchanged.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUserRequest {
    pub display_name: Option<String>,
    pub disabled: Option<bool>,
    pub role: Option<Role>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ResetPasswordRequest {
    /// At least 12 characters.
    pub new_password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserListResponse {
    pub items: Vec<UserView>,
}

/// Lists the members of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/users",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The members of the tenant.", body = UserListResponse),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_users(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<UserListResponse>, ApiError> {
    context.authorize(Operation::UsersRead)?;
    let (members, sign_ins) = context
        .tenant_tx(&state, async |tx| {
            let members = users::list_members(tx).await?;
            let ids: Vec<UserId> = members.iter().map(|member| member.id).collect();
            Ok((members, users::last_sign_ins(tx, &ids).await?))
        })
        .await?;
    let items = members
        .iter()
        .map(|member| {
            let mut view = UserView::from_member(member);
            view.last_sign_in_at = sign_ins
                .iter()
                .find(|(id, _)| *id == member.id)
                .map(|(_, time)| format_time(*time));
            view
        })
        .collect();
    Ok(Json(UserListResponse { items }))
}

#[utoipa::path(
    post,
    path = "/api/v1/users",
    summary = "Creates an account with a membership in the tenant.",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "The account is created.", body = UserView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 409, description = "The username exists in the installation.", body = crate::error::ProblemBody),
        (status = 503, description = "The password work is busy.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_user(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserView>), ApiError> {
    context.authorize(Operation::UsersWrite)?;
    // The create endpoint is rate limited for each actor and tenant (threat T15).
    check_create_user_limit(&state, &context)?;
    allowed_role(&context, body.role)?;
    let role = body.role;
    let username = validation::validate_username("username", &body.username)?;
    let display_name = validation::validate_display_name("display_name", &body.display_name)?;
    validation::validate_password("password", &body.password)?;

    // The username is unique in the installation (ADR-0025). A tenant Admin cannot add an existing
    // account: a name that exists gives HTTP 409 `username_taken`.
    if users::find_account(state.db(), &username).await?.is_some() {
        return Err(ApiError::taken(AccountField::Username));
    }

    let password_hash =
        password::hash_password_limited(&state.auth().hash_semaphore, body.password.clone())
            .await?;

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    tx.lock(ADMIN_LOCK).await?;
    let user_id = users::create_member(
        &mut tx,
        &users::NewUser {
            username: &username,
            password_hash: &password_hash,
            display_name: &display_name,
        },
        role,
    )
    .await
    .map_err(ApiError::account_write)?;
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
    let member = users::get_member(&mut tx, user_id).await?;
    tx.commit().await?;

    // An account change is also an audit record in the platform audit (ADR-0025).
    audit_platform(&state, &context, user_id, AuditAction::UserCreate).await?;

    let Some(member) = member else {
        return Err(ApiError::internal(format!(
            "the account {user_id} is not found after the create"
        )));
    };
    Ok((StatusCode::CREATED, Json(UserView::from_member(&member))))
}

/// Reads one member of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/users/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the account.")),
    responses(
        (status = 200, description = "The member.", body = UserView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The account is not a member of the tenant.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_user(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<UserView>, ApiError> {
    context.authorize(Operation::UsersRead)?;
    let user_id = parse_uuid(&id)?;
    let (member, sign_ins) = context
        .tenant_tx(&state, async |tx| {
            let member = users::get_member(tx, user_id).await?;
            Ok((member, users::last_sign_ins(tx, &[user_id]).await?))
        })
        .await?;
    let Some(member) = member else {
        return Err(ApiError::not_found());
    };
    let mut view = UserView::from_member(&member);
    view.last_sign_in_at = sign_ins.first().map(|(_, time)| format_time(*time));
    Ok(Json(view))
}

/// Changes a member of the tenant.
///
/// A tenant Admin changes the role or disables the membership in its tenant. It cannot change the
/// display name of a shared account (HTTP 403 `shared_account`) and cannot remove its own
/// administrator rights (HTTP 409).
#[utoipa::path(
    patch,
    path = "/api/v1/users/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the account.")),
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "The changed member.", body = UserView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 403, description = "The account is shared.", body = crate::error::ProblemBody),
        (status = 404, description = "The account is not a member of the tenant.", body = crate::error::ProblemBody),
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
    let new_role = match body.role {
        Some(role) => {
            allowed_role(&context, role)?;
            Some(role)
        }
        None => None,
    };
    let display_name = match &body.display_name {
        Some(value) => Some(validation::validate_display_name("display_name", value)?),
        None => None,
    };

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    // The lock serializes the last-admin check with the write.
    tx.lock(ADMIN_LOCK).await?;
    let Some(member) = users::get_member(&mut tx, user_id).await? else {
        return Err(ApiError::not_found());
    };
    ensure_target_not_above_actor(&context, member.role)?;
    // A tenant scope must not change an account that holds the installation-wide right (ADR-0029).
    if member.platform_admin {
        return Err(ApiError::forbidden());
    }

    let role_lost_admin = new_role.is_some_and(|role| !role.is_admin());
    let disabling = body.disabled == Some(true);
    // The caller cannot remove its own administrator rights: the session would keep a role that the
    // membership no longer grants (ADR-0025).
    if context.actor == Actor::User(user_id) && (role_lost_admin || disabling) {
        return Err(ApiError::conflict());
    }
    // The guard applies to an active Admin only. A disabled membership is not the last Admin.
    let active_admin = member.role.is_admin() && !member.disabled && !member.account_disabled;
    if active_admin && (role_lost_admin || disabling) {
        guard_last_admin(&mut tx).await?;
    }

    if let Some(display_name) = &display_name {
        // The database function takes the account row lock, then checks the membership and the
        // sharing, then writes (ADR-0025).
        match users::admin_set_display_name(&mut tx, user_id, display_name).await? {
            WriteStatus::Ok => {}
            WriteStatus::NotFound => return Err(ApiError::not_found()),
            WriteStatus::Shared => return Err(ApiError::shared_account()),
            WriteStatus::Forbidden => return Err(ApiError::forbidden()),
        }
    }
    if let Some(disabled) = body.disabled {
        if context.tenant_id == PLATFORM_TENANT_ID {
            // Only a Platform Admin disables the account itself.
            users::set_disabled(&mut tx, user_id, disabled).await?;
            if disabled {
                auth::delete_account_sessions(&mut tx, user_id, None).await?;
            }
        } else {
            memberships::set_disabled(&mut tx, user_id, disabled).await?;
            if disabled {
                sessions::delete_for_user(&mut tx, user_id).await?;
            }
        }
    }
    if let Some(role) = new_role {
        memberships::set_role(&mut tx, user_id, role).await?;
    }
    let updated = users::get_member(&mut tx, user_id).await?;
    let before = serde_json::json!({
        "display_name": member.display_name,
        "disabled": member.disabled,
        "role": member.role.as_str(),
    });
    let after = serde_json::json!({
        "display_name": updated.as_ref().map(|member| member.display_name.clone()),
        "disabled": updated.as_ref().map(|member| member.disabled),
        "role": updated.as_ref().map(|member| member.role.as_str()),
    });
    let mut event = context.audit_event(AuditAction::UserUpdate, "user", user_id.as_uuid());
    event.diff = Some(diff(Some(&before), Some(&after), &[]));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    match updated {
        Some(member) => Ok(Json(UserView::from_member(&member))),
        None => Err(ApiError::internal(format!(
            "the account {user_id} is not found after the update"
        ))),
    }
}

/// Disables the membership of the account in the tenant and removes its sessions in the tenant.
#[utoipa::path(
    delete,
    path = "/api/v1/users/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the account.")),
    responses(
        (status = 204, description = "The membership is disabled."),
        crate::error::ProtectedResponses,
        (status = 404, description = "The account is not a member of the tenant.", body = crate::error::ProblemBody),
        (status = 409, description = "The account is the last Admin.", body = crate::error::ProblemBody)
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
    let Some(member) = users::get_member(&mut tx, user_id).await? else {
        return Err(ApiError::not_found());
    };
    ensure_target_not_above_actor(&context, member.role)?;
    // A tenant scope must not remove the membership of an account that holds the installation-wide
    // right (ADR-0029).
    if member.platform_admin {
        return Err(ApiError::forbidden());
    }
    // The caller cannot disable its own membership: the session would keep a role that the
    // membership no longer grants (ADR-0025).
    if context.actor == Actor::User(user_id) {
        return Err(ApiError::conflict());
    }
    let active_admin = member.role.is_admin() && !member.disabled && !member.account_disabled;
    if active_admin {
        guard_last_admin(&mut tx).await?;
    }
    memberships::set_disabled(&mut tx, user_id, true).await?;
    sessions::delete_for_user(&mut tx, user_id).await?;
    let mut event = context.audit_event(AuditAction::UserDelete, "user", user_id.as_uuid());
    event.diff = Some(diff(
        Some(&serde_json::json!({"disabled": member.disabled})),
        Some(&serde_json::json!({"disabled": true})),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Resets the password of a member (Admin action).
///
/// A tenant Admin cannot reset the password of a shared account (HTTP 403 `shared_account`) or of
/// its own account (HTTP 403).
#[utoipa::path(
    post,
    path = "/api/v1/users/{id}/password",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the account.")),
    request_body = ResetPasswordRequest,
    responses(
        (status = 204, description = "The password is changed."),
        (status = 400, description = "The new password is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 403, description = "The account is shared or is the account of the caller.", body = crate::error::ProblemBody),
        (status = 404, description = "The account is not a member of the tenant.", body = crate::error::ProblemBody),
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
    // A reset of the own account skips the current password check of the self change.
    if context.actor == Actor::User(user_id) {
        return Err(ApiError::forbidden());
    }
    validation::validate_password("new_password", &body.new_password)?;

    let member = context
        .tenant_tx(&state, async |tx| Ok(users::get_member(tx, user_id).await?))
        .await?;
    let Some(member) = member else {
        return Err(ApiError::not_found());
    };
    ensure_target_not_above_actor(&context, member.role)?;
    // A tenant scope must not reset the password of an account that holds the installation-wide
    // right (ADR-0029).
    if member.platform_admin {
        return Err(ApiError::forbidden());
    }

    let password_hash =
        password::hash_password_limited(&state.auth().hash_semaphore, body.new_password.clone())
            .await?;

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    // The database function takes the account row lock, then checks the membership and the sharing,
    // then writes (ADR-0025).
    match users::admin_set_password(&mut tx, user_id, &password_hash).await? {
        WriteStatus::Ok => {}
        WriteStatus::NotFound => return Err(ApiError::not_found()),
        WriteStatus::Shared => return Err(ApiError::shared_account()),
        WriteStatus::Forbidden => return Err(ApiError::forbidden()),
    }
    // A tenant Admin reset removes the sessions of this tenant only.
    auth::delete_account_sessions(&mut tx, user_id, None).await?;
    let mut event = context.audit_event(AuditAction::UserUpdate, "user", user_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({"password": "<reset>"})),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    audit_platform(&state, &context, user_id, AuditAction::UserUpdate).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn guard_last_admin(tx: &mut TenantTx<'_>) -> Result<(), ApiError> {
    if memberships::count_admins(tx).await? <= 1 {
        return Err(ApiError::conflict());
    }
    Ok(())
}

/// Records an account change for the platform administration too.
async fn audit_platform(
    state: &AppState,
    context: &RequestContext,
    user_id: UserId,
    action: AuditAction,
) -> Result<(), ApiError> {
    let mut event = AuditEvent::new(action, "user");
    event.actor = context.actor.clone();
    event.source_ip = context.source_ip;
    event.object_id = Some(user_id.as_uuid());
    let mut tx = state.db().begin_platform().await?;
    restichub_db::audit::insert_platform(&mut tx, PLATFORM_TENANT_ID, &event).await?;
    tx.commit().await?;
    Ok(())
}

/// The create endpoint limit for each actor and tenant, and for the tenant itself.
fn check_create_user_limit(state: &AppState, context: &RequestContext) -> Result<(), ApiError> {
    let actor = crate::rate_limit::actor_label(&context.actor);
    let now = std::time::Instant::now();
    let key = crate::rate_limit::create_user_key(context.tenant_id, &actor);
    if let Err(wait) = state.auth().create_user_limiter.check(&key, now) {
        return Err(ApiError::rate_limited(wait.as_secs().max(1)));
    }
    // The tenant bucket bounds an actor that mints many API tokens (threat T15).
    let tenant_key = crate::rate_limit::create_user_tenant_key(context.tenant_id);
    if let Err(wait) = state
        .auth()
        .create_user_tenant_limiter
        .check(&tenant_key, now)
    {
        return Err(ApiError::rate_limited(wait.as_secs().max(1)));
    }
    Ok(())
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
