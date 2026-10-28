//! The About ResticHub data (`GET /api/v1/about`, ADR-0054, ADR-0055).
//!
//! Every authenticated role reads the version, the revision, the protocol versions and the
//! environment of the control plane, also in the platform mode and in a session without an
//! organization. A person gives these facts to the support team. The database schema level goes to
//! the Platform Admin only. The body holds no secret, no value that identifies the host and no
//! path.

use axum::Json;
use axum::extract::State;
use restichub_proto::version::Version;
use serde::Serialize;
use utoipa::ToSchema;

use crate::context::{Operation, RequestContext};
use crate::environment::{self, EnvironmentView};
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
    /// Absent for a role below the Platform Admin, and while the database holds no migration row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_level: Option<i64>,
    /// Goes to every role (ADR-0055).
    pub environment: EnvironmentView,
}

/// Splits the stamped version into the package version, the pre-release flag and the revision.
///
/// The build script stamps `<package version>+<revision>` and the package version keeps its
/// pre-release part (`0.2.0-rc.2+3b21922`). The version stays exact, so support reads the same
/// string as the release notes.
fn split_build_version(value: &str) -> (String, bool, String) {
    let (package, revision) = value.split_once('+').unwrap_or((value, ""));
    let prerelease = package.contains('-');
    (package.to_owned(), prerelease, revision.to_owned())
}

/// Answers the About ResticHub screen with the version of this build.
#[utoipa::path(
    get,
    path = "/api/v1/about",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The version, the protocol versions and the environment of the control plane.", body = AboutResponse),
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
    let schema_level = if context.platform_admin {
        state.db().schema_level().await?
    } else {
        None
    };
    let environment = environment::read(state.db()).await?;
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
        schema_level,
        environment,
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
        assert_eq!(version, "0.2.0-rc.2");
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
