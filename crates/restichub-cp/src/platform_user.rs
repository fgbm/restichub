//! The record endpoints of one installation account for a Platform Admin (ADR-0028, ADR-0029).
//!
//! The Platform Admin reads one account with its memberships and resets its password. The
//! endpoints are platform-scoped: a tenant scope gets HTTP 403.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::audit::{self, Actor, AuditAction, diff};
use restichub_db::users::WriteStatus;
use restichub_db::{PLATFORM_TENANT_ID, auth, memberships, users};

use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::password;
use crate::platform_users::parse_uuid;
use crate::server::AppState;
use crate::users::ResetPasswordRequest;
use crate::validation;
use crate::views::{PlatformMembershipView, PlatformUserRecordView, PlatformUserView};

/// Reads one account of the installation with its memberships (Platform Admin only).
#[utoipa::path(
    get,
    path = "/api/v1/platform/users/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the account.")),
    responses(
        (status = 200, description = "The account.", body = PlatformUserRecordView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The account is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_account(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<PlatformUserRecordView>, ApiError> {
    context.authorize(Operation::PlatformUsersRead)?;
    let user_id = parse_uuid(&id)?;
    let mut tx = state.db().begin_platform().await?;
    let Some(account) = users::platform_account(&mut tx, user_id).await? else {
        return Err(ApiError::not_found());
    };
    let member_rows = memberships::list_for_account(&mut tx, user_id).await?;
    let platform_admin_count = users::platform_admin_count_platform(&mut tx).await?;
    tx.commit().await?;
    let account_memberships = member_rows
        .iter()
        .map(PlatformMembershipView::from)
        .collect();
    Ok(Json(PlatformUserRecordView {
        account: PlatformUserView::new(&account, account_memberships),
        platform_admin_count,
    }))
}

/// Resets the password of an account and ends every session of it (Platform Admin only).
///
/// The caller's own account is refused with HTTP 403: the caller changes its own password with the
/// current password. A shared account and another Platform Admin are no obstacle in the platform
/// scope (ADR-0025, ADR-0029). The reset is recorded in the platform audit and in the audit of each
/// organization of the account.
#[utoipa::path(
    post,
    path = "/api/v1/platform/users/{id}/password",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the account.")),
    request_body = ResetPasswordRequest,
    responses(
        (status = 204, description = "The password is changed."),
        (status = 400, description = "The new password is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The account is not found.", body = crate::error::ProblemBody),
        (status = 503, description = "The password work is busy.", body = crate::error::ProblemBody)
    )
)]
pub async fn reset_password(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ResetPasswordRequest>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::PlatformUsersWrite)?;
    let user_id = parse_uuid(&id)?;
    // A reset of the own account skips the current password check of the self change.
    if context.actor == Actor::User(user_id) {
        return Err(ApiError::forbidden());
    }
    validation::validate_password("new_password", &body.new_password)?;
    let password_hash =
        password::hash_password_limited(&state.auth().hash_semaphore, body.new_password.clone())
            .await?;

    let mut tx = state.db().begin_platform().await?;
    match users::platform_set_password(&mut tx, user_id, &password_hash).await? {
        WriteStatus::Ok => {}
        WriteStatus::NotFound => return Err(ApiError::not_found()),
        WriteStatus::Forbidden | WriteStatus::Shared => return Err(ApiError::forbidden()),
    }
    auth::delete_all_account_sessions(&mut tx, user_id).await?;
    let mut event = context.audit_event(AuditAction::UserUpdate, "user", user_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&serde_json::json!({"password": "<reset>"})),
        &[],
    ));
    audit::insert_platform(&mut tx, PLATFORM_TENANT_ID, &event).await?;
    // The new password signs in to every organization of the account, so each one records the
    // reset (T22).
    for member in memberships::list_for_account(&mut tx, user_id).await? {
        audit::insert_platform_for(&mut tx, member.tenant_id, &event).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
