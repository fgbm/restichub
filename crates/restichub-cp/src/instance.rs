//! The public endpoint that reports the tenancy mode and the setup state (ADR-0024).
//!
//! The user interface reads the endpoint at load, so it adapts to the tenancy mode. The response
//! holds no secret and no version detail.

use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

use crate::config::TenancyMode;
use crate::error::ApiError;
use crate::server::AppState;
use crate::setup;

/// The mode and the setup state of the installation.
#[derive(Debug, Serialize, ToSchema)]
pub struct InstanceResponse {
    pub tenancy_mode: TenancyMode,
    /// True while the installation has no administrator.
    pub setup_required: bool,
}

#[utoipa::path(
    get,
    path = "/api/v1/instance",
    tags = ["system"],
    summary = "Reads the mode and the setup state of the installation.",
    responses((status = 200, description = "The mode and the setup state.", body = InstanceResponse))
)]
pub async fn instance(State(state): State<AppState>) -> Result<Json<InstanceResponse>, ApiError> {
    let setup_required = setup::setup_required(&state).await?;
    Ok(Json(InstanceResponse {
        tenancy_mode: state.tenancy().mode,
        setup_required,
    }))
}
