//! The About ResticHub data (`GET /api/v1/about`).
//!
//! Every authenticated role reads the version, the revision and the protocol versions of the
//! control plane, also in the platform mode and in a session without an organization. An operator
//! gives these facts to support. The tenancy mode and the database schema level go to the Platform
//! Admin only (ADR-0029). The body holds no secret, no environment value and no path.

use axum::Json;
use axum::extract::State;
use restichub_proto::version::Version;
use serde::Serialize;
use utoipa::ToSchema;

use crate::config::TenancyMode;
use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::server::AppState;

/// The wire protocol version that the control plane speaks (FR-PR-09).
#[derive(Debug, Serialize, ToSchema)]
pub struct ProtocolView {
    pub major: u32,
    pub minor: u32,
}

/// The agent protocol versions that the control plane accepts.
///
/// The accepted range follows the N-1 rule: the same major version and the minor versions N-1 and N
/// work. A newer minor version works too, because the control plane ignores the fields that it does
/// not know (FR-PR-09).
#[derive(Debug, Serialize, ToSchema)]
pub struct AgentProtocolView {
    /// The major version of an agent that the control plane accepts.
    pub major: u32,
    /// The oldest accepted agent minor version, N-1.
    pub minimum_minor: u32,
}

/// The version of the control plane and the compatibility facts of the installation.
#[derive(Debug, Serialize, ToSchema)]
pub struct AboutResponse {
    /// The package version with the pre-release part, without the build revision.
    pub version: String,
    /// True when the version carries a pre-release part, for example `-rc.2`.
    pub prerelease: bool,
    /// The commit of the build, or `dev` for a local build.
    pub revision: String,
    pub protocol: ProtocolView,
    pub agent_protocol: AgentProtocolView,
    /// Absent for a role below the Platform Admin (ADR-0029).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenancy_mode: Option<TenancyMode>,
    /// The version of the last applied migration. Absent for a role below the Platform Admin.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_level: Option<i64>,
}

/// Splits the stamped version into the package version, the pre-release part and the revision.
///
/// The build script stamps `<package version>+<revision>` and the package version may hold a
/// pre-release part (`0.2.0-rc.2+3b21922`).
fn split_build_version(value: &str) -> (String, bool, String) {
    let (package, revision) = value.split_once('+').unwrap_or((value, ""));
    match package.split_once('-') {
        Some((version, _)) => (version.to_owned(), true, revision.to_owned()),
        None => (package.to_owned(), false, revision.to_owned()),
    }
}

/// Answers the About ResticHub screen with the version of this build.
#[utoipa::path(
    get,
    path = "/api/v1/about",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The version and the protocol versions of the control plane.", body = AboutResponse),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody)
    )
)]
pub async fn about(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<AboutResponse>, ApiError> {
    context.authorize(Operation::AboutRead)?;
    let (version, prerelease, revision) = split_build_version(restichub_common::BUILD_VERSION);
    let protocol = Version::CURRENT;
    let (tenancy_mode, schema_level) = if context.platform_admin {
        (
            Some(state.tenancy().mode),
            Some(state.db().schema_level().await?),
        )
    } else {
        (None, None)
    };
    Ok(Json(AboutResponse {
        version,
        prerelease,
        revision,
        protocol: ProtocolView {
            major: protocol.major,
            minor: protocol.minor,
        },
        agent_protocol: AgentProtocolView {
            major: protocol.major,
            minimum_minor: protocol.minimum_minor(),
        },
        tenancy_mode,
        schema_level,
    }))
}

#[cfg(test)]
mod tests {
    use super::split_build_version;

    #[test]
    fn a_release_carries_no_pre_release() {
        let (version, prerelease, revision) = split_build_version("0.2.0+3b21922");
        assert_eq!(version, "0.2.0");
        assert!(!prerelease);
        assert_eq!(revision, "3b21922");
    }

    #[test]
    fn a_pre_release_keeps_its_package_version() {
        let (version, prerelease, revision) = split_build_version("0.2.0-rc.2+3b21922");
        assert_eq!(version, "0.2.0");
        assert!(prerelease);
        assert_eq!(revision, "3b21922");
    }

    #[test]
    fn a_local_build_carries_the_dev_revision() {
        let (version, prerelease, revision) = split_build_version("0.2.0+dev");
        assert_eq!(version, "0.2.0");
        assert!(!prerelease);
        assert_eq!(revision, "dev");
    }
}
