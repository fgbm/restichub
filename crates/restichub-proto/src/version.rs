//! The protocol version rule of the connection (FR-PR-09, AC-PR-06).

use crate::{PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR};

/// The answer of the control plane to the protocol version of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionCheck {
    Compatible,
    /// Agent major differs from the control plane major.
    MajorMismatch,
    /// The agent minor version is older than the previous minor version of the control plane.
    MinorTooOld,
}

/// A major and minor number pair of the wire protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
}

impl Version {
    /// Protocol version this build speaks.
    pub const CURRENT: Self = Self {
        major: PROTOCOL_VERSION_MAJOR,
        minor: PROTOCOL_VERSION_MINOR,
    };

    /// The smallest agent minor version that a control plane of this version accepts: N-1.
    #[must_use]
    pub const fn minimum_minor(self) -> u32 {
        self.minor.saturating_sub(1)
    }

    /// Checks the protocol version of an agent against this control plane version.
    ///
    /// The same major version and the minor versions N-1 and N work. A newer minor version works
    /// too: the control plane ignores the fields that it does not know, and the capabilities of
    /// the hello gate every optional feature (FR-PR-09).
    #[must_use]
    pub const fn check_agent(self, agent: Self) -> VersionCheck {
        if agent.major != self.major {
            VersionCheck::MajorMismatch
        } else if agent.minor < self.minimum_minor() {
            VersionCheck::MinorTooOld
        } else {
            VersionCheck::Compatible
        }
    }
}

/// Checks the protocol version of an agent against the version of this build.
#[must_use]
pub const fn check_agent_version(major: u32, minor: u32) -> VersionCheck {
    Version::CURRENT.check_agent(Version { major, minor })
}

#[cfg(test)]
mod tests {
    use super::{Version, VersionCheck, check_agent_version};
    use crate::{PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR};

    const SERVER: Version = Version { major: 3, minor: 4 };

    #[test]
    fn ac_pr_06_the_previous_and_the_current_minor_work() {
        for minor in [3, 4, 5] {
            assert_eq!(
                SERVER.check_agent(Version { major: 3, minor }),
                VersionCheck::Compatible,
                "minor {minor}"
            );
        }
    }

    #[test]
    fn ac_pr_06_the_next_and_the_previous_major_are_refused() {
        for major in [2, 4] {
            assert_eq!(
                SERVER.check_agent(Version { major, minor: 4 }),
                VersionCheck::MajorMismatch,
                "major {major}"
            );
        }
    }

    #[test]
    fn ac_pr_06_a_minor_before_the_previous_one_is_refused() {
        assert_eq!(
            SERVER.check_agent(Version { major: 3, minor: 2 }),
            VersionCheck::MinorTooOld
        );
    }

    #[test]
    fn ac_pr_06_the_build_accepts_its_own_version() {
        assert_eq!(
            check_agent_version(PROTOCOL_VERSION_MAJOR, PROTOCOL_VERSION_MINOR),
            VersionCheck::Compatible
        );
        assert_eq!(
            check_agent_version(PROTOCOL_VERSION_MAJOR + 1, 0),
            VersionCheck::MajorMismatch
        );
    }
}
