//! The version mark of an agent against the control plane (FR-PR-09, AC-PR-06).

use restichub_proto::version::{Version, VersionCheck};
use serde::Serialize;
use utoipa::ToSchema;

/// How the version of an agent relates to the control plane. The Hosts screen shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentVersionStatus {
    /// The agent runs the version of the control plane or a newer one.
    Current,
    /// A newer agent package exists: the control plane version is greater.
    UpdateAvailable,
    /// The control plane refuses the protocol version of the agent.
    Unsupported,
    /// The agent never reported a version that the control plane can read.
    Unknown,
}

pub const CONTROL_PLANE_VERSION: &str = restichub_common::BUILD_VERSION;

/// Compares the package version and the protocol version of an agent with this build.
///
/// A refused protocol version wins: an agent that cannot connect needs an update whatever its
/// package version says.
pub fn status(agent_version: &str, protocol: Option<(u32, u32)>) -> AgentVersionStatus {
    status_against(CONTROL_PLANE_VERSION, agent_version, protocol)
}

fn status_against(
    control_plane: &str,
    agent_version: &str,
    protocol: Option<(u32, u32)>,
) -> AgentVersionStatus {
    if let Some((major, minor)) = protocol
        && Version::CURRENT.check_agent(Version { major, minor }) != VersionCheck::Compatible
    {
        return AgentVersionStatus::Unsupported;
    }
    match (parse(agent_version), parse(control_plane)) {
        (Some(agent), Some(server)) if agent < server => AgentVersionStatus::UpdateAvailable,
        (Some(_), Some(_)) => AgentVersionStatus::Current,
        _ => AgentVersionStatus::Unknown,
    }
}

/// Reads `major.minor.patch`, ignoring a pre-release or build suffix.
fn parse(value: &str) -> Option<(u64, u64, u64)> {
    let core = value.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::{AgentVersionStatus, status_against};
    use restichub_proto::{PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR};

    const CURRENT: Option<(u32, u32)> = Some((PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR));

    #[test]
    fn fr_pr_09_an_older_package_gets_the_update_mark() {
        assert_eq!(
            status_against("1.4.0", "1.3.9", CURRENT),
            AgentVersionStatus::UpdateAvailable
        );
        assert_eq!(
            status_against("1.4.0", "1.4.0-rc.1", CURRENT),
            AgentVersionStatus::Current
        );
    }

    #[test]
    fn fr_pr_09_the_same_or_a_newer_package_is_current() {
        assert_eq!(
            status_against("1.4.0", "1.4.0", CURRENT),
            AgentVersionStatus::Current
        );
        assert_eq!(
            status_against("1.4.0", "1.5.2+build.7", CURRENT),
            AgentVersionStatus::Current
        );
    }

    #[test]
    fn ac_pr_06_a_refused_protocol_version_is_unsupported() {
        assert_eq!(
            status_against("1.4.0", "1.4.0", Some((PROTOCOL_VERSION_MAJOR + 1, 0))),
            AgentVersionStatus::Unsupported
        );
    }

    #[test]
    fn fr_pr_09_an_unreadable_version_is_unknown() {
        assert_eq!(
            status_against("1.4.0", "", CURRENT),
            AgentVersionStatus::Unknown
        );
        assert_eq!(
            status_against("1.4.0", "dev", None),
            AgentVersionStatus::Unknown
        );
    }
}
