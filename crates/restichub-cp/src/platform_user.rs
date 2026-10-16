//! The record endpoints of one installation account for a Platform Admin (ADR-0028, ADR-0029).
//!
//! The Platform Admin reads one account with its memberships and resets its password. The
//! endpoints are platform-scoped: a tenant scope gets HTTP 403.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::audit::{Actor, AuditAction, diff};
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
use crate::views::{PlatformMembershipView, PlatformUserView};

/// Reads one account of the installation with its memberships (Platform Admin only).
#[utoipa::path(
    get,
    path = "/api/v1/platform/users/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the account.")),
    responses(
        (status = 200, description = "The account.", body = PlatformUserView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The account is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_account(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<PlatformUserView>, ApiError> {
    context.authorize(Operation::PlatformUsersRead)?;
    let user_id = parse_uuid(&id)?;
    // The list functions give the same fields and the same membership filter as the account list.
    let (account, member_rows) = {
        let mut tx = state.db().begin_platform().await?;
        let account = users::list_platform_accounts(&mut tx)
            .await?
            .into_iter()
            .find(|account| account.id == user_id);
        let member_rows = memberships::list_all(&mut tx).await?;
        tx.commit().await?;
        (account, member_rows)
    };
    let Some(account) = account else {
        return Err(ApiError::not_found());
    };
    let account_memberships = member_rows
        .iter()
        .filter(|member| member.user_id == user_id)
        .map(PlatformMembershipView::from)
        .collect();
    Ok(Json(PlatformUserView::new(&account, account_memberships)))
}

/// Resets the password of an account and ends every session of it (Platform Admin only).
///
/// The caller's own account is refused with HTTP 403: the caller changes its own password with the
/// current password. A shared account is no obstacle in the platform scope (ADR-0025).
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

    let mut tx = state.db().begin_tenant(PLATFORM_TENANT_ID).await?;
    match users::platform_set_password(&mut tx, user_id, &password_hash).await? {
        WriteStatus::Ok => {}
        WriteStatus::NotFound => return Err(ApiError::not_found()),
        WriteStatus::Forbidden | WriteStatus::Shared => return Err(ApiError::forbidden()),
    }
    auth::delete_account_sessions(&mut tx, user_id, None).await?;
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
