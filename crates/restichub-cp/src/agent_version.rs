//! The version mark of an agent against the control plane (FR-PR-09, AC-PR-06).

use std::cmp::Ordering;

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

/// A package version that compares by the precedence of Semantic Versioning 2.0.
struct PackageVersion {
    /// `major`, `minor` and `patch`.
    core: (u64, u64, u64),
    /// Empty for a release.
    pre_release: Vec<PreReleaseIdentifier>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PreReleaseIdentifier {
    /// The variant order carries the rule: a number is lower than an alphanumeric identifier.
    Number(u64),
    Text(String),
}

impl PartialEq for PackageVersion {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for PackageVersion {}

impl PartialOrd for PackageVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PackageVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| compare_pre_release(&self.pre_release, &other.pre_release))
    }
}

/// Compares the identifiers field by field. A shorter list is lower when the common fields are
/// equal, and a release is higher than each of its pre-releases.
fn compare_pre_release(left: &[PreReleaseIdentifier], right: &[PreReleaseIdentifier]) -> Ordering {
    match (left.is_empty(), right.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => left.iter().cmp(right.iter()),
    }
}

/// Reads `major.minor.patch` with an optional pre-release and build metadata. The build
/// metadata after `+` does not change the comparison.
fn parse(value: &str) -> Option<PackageVersion> {
    let without_build = value.split_once('+').map_or(value, |(head, _)| head);
    let (core, pre_release) = match without_build.split_once('-') {
        Some((core, pre_release)) => (core, parse_pre_release(pre_release)?),
        None => (without_build, Vec::new()),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(PackageVersion {
        core: (major, minor, patch),
        pre_release,
    })
}

fn parse_pre_release(value: &str) -> Option<Vec<PreReleaseIdentifier>> {
    value
        .split('.')
        .map(|field| {
            if field.is_empty() || !field.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return None;
            }
            Some(match field.parse() {
                Ok(number) => PreReleaseIdentifier::Number(number),
                Err(_) => PreReleaseIdentifier::Text(field.to_owned()),
            })
        })
        .collect()
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
            AgentVersionStatus::UpdateAvailable
        );
    }

    #[test]
    fn fr_pr_09_a_release_candidate_is_lower_than_its_release() {
        assert_eq!(
            status_against("0.1.0", "0.1.0-rc.4", CURRENT),
            AgentVersionStatus::UpdateAvailable
        );
        assert_eq!(
            status_against("0.1.0", "0.1.0-rc.10", CURRENT),
            AgentVersionStatus::UpdateAvailable
        );
        assert_eq!(
            status_against("1.4.0", "1.4.0-0", CURRENT),
            AgentVersionStatus::UpdateAvailable
        );
    }

    #[test]
    fn fr_pr_09_the_pre_release_identifiers_compare_field_by_field() {
        assert_eq!(
            status_against("0.1.0-rc.10", "0.1.0-rc.4", CURRENT),
            AgentVersionStatus::UpdateAvailable
        );
        assert_eq!(
            status_against("0.1.0-rc.6", "0.1.0-rc.6", CURRENT),
            AgentVersionStatus::Current
        );
        assert_eq!(
            status_against("0.1.0-rc.6", "0.1.0-rc.6.1", CURRENT),
            AgentVersionStatus::Current
        );
        assert_eq!(
            status_against("0.1.0-rc.6", "0.1.0-beta.6", CURRENT),
            AgentVersionStatus::UpdateAvailable
        );
        assert_eq!(
            status_against("0.1.0-rc.6", "0.1.0-rc.6+3b21922", CURRENT),
            AgentVersionStatus::Current
        );
    }

    #[test]
    fn fr_pr_09_a_numeric_identifier_is_lower_than_an_alphanumeric_one() {
        assert_eq!(
            status_against("0.1.0-rc.beta", "0.1.0-rc.1", CURRENT),
            AgentVersionStatus::UpdateAvailable
        );
        assert_eq!(
            status_against("0.1.0-rc.1", "0.1.0-rc.beta", CURRENT),
            AgentVersionStatus::Current
        );
    }

    #[test]
    fn fr_pr_09_a_release_candidate_control_plane_compares_by_precedence() {
        assert_eq!(
            status_against("0.1.0-rc.6", "0.1.0", CURRENT),
            AgentVersionStatus::Current
        );
        assert_eq!(
            status_against("0.1.0-rc.6", "0.1.0-rc.4", CURRENT),
            AgentVersionStatus::UpdateAvailable
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
    fn fr_pr_09_the_build_revision_does_not_change_the_comparison() {
        assert_eq!(
            status_against("0.1.0+3b21922", "0.1.0+dev", CURRENT),
            AgentVersionStatus::Current
        );
        assert_eq!(
            status_against("0.2.0+3b21922", "0.1.0+3b21922", CURRENT),
            AgentVersionStatus::UpdateAvailable
        );
    }

    #[test]
    fn fr_pr_09_the_control_plane_version_carries_the_build_revision() {
        let (package, revision) = super::CONTROL_PLANE_VERSION
            .split_once('+')
            .expect("a build revision");
        assert_ne!(package, "0.0.0");
        assert!(!revision.is_empty());
    }

    #[test]
    fn ac_pr_06_a_refused_protocol_version_is_unsupported() {
        assert_eq!(
            status_against("1.4.0", "1.4.0", Some((PROTOCOL_VERSION_MAJOR + 1, 0))),
            AgentVersionStatus::Unsupported
        );
    }

    #[test]
    fn fr_pr_09_an_invalid_pre_release_is_unknown() {
        for version in ["0.1.0-", "0.1.0-rc..1", "0.1.0-rc.1_2", "0.1.0-rc.1 2"] {
            assert_eq!(
                status_against("1.4.0", version, CURRENT),
                AgentVersionStatus::Unknown,
                "{version}"
            );
        }
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
