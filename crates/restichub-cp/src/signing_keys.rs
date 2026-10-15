//! The platform action for a compromised job signing key (ADR-0030, FR-PR-11).
//!
//! The action rotates the key set and retires the old current key with the reason `compromise`.
//! The open streams get the signed update at once. An agent that was offline during the compromise
//! enrolls again, because the re-sync chain never carries a compromised key.

use axum::Json;
use axum::extract::State;
use restichub_db::audit::AuditAction;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::server::AppState;

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CompromiseRequest {
    /// The identifier of the current job signing key, as the caller read it. A different current
    /// key gives HTTP 409, so a repeated call does not retire the key set that the first call made.
    pub expected_current_key_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CompromiseResponse {
    /// The identifiers of the keys that the action retired for a compromise.
    pub retired_key_ids: Vec<String>,
}

/// Marks the current job signing key as compromised (ADR-0030).
#[utoipa::path(
    post,
    path = "/api/v1/platform/signing-keys/compromise",
    summary = "Marks the current job signing key as compromised.",
    description = "The next key becomes the current key and a new next key is created. The old current key retires with the reason `compromise` and the signed update revokes it on every open agent stream. An agent that was offline enrolls again.",
    request_body = CompromiseRequest,
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The key set is rotated.", body = CompromiseResponse),
        (status = 409, description = "The current key is not the expected one.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
    )
)]
pub async fn mark_compromised(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CompromiseRequest>,
) -> Result<Json<CompromiseResponse>, ApiError> {
    context.authorize(Operation::PlatformSigningKeysWrite)?;
    let signing = state
        .job_signing()
        .ok_or_else(|| ApiError::internal("the job signing key set is not loaded"))?;
    let template = context.audit_event(
        AuditAction::SigningKeyCompromise,
        "signing_key",
        Uuid::nil(),
    );
    let retired = signing
        .mark_compromised(
            state.db(),
            state.auth().clock.now(),
            &body.expected_current_key_id,
            &template,
        )
        .await?;
    Ok(Json(CompromiseResponse {
        retired_key_ids: retired.iter().map(Uuid::to_string).collect(),
    }))
}
