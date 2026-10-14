//! Facts about the target agent (FR-PF-03, FR-HK-04).

use std::collections::BTreeMap;
use std::fmt;

use time::OffsetDateTime;

/// Operating system of a target agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperatingSystem {
    Windows,
    Linux,
    Macos,
}

impl OperatingSystem {
    /// The value of the `{{os}}` variable and of a `paths.by_os` key (FR-PF-03).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Macos => "macos",
        }
    }
}

impl fmt::Display for OperatingSystem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// CPU kind of a target agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Architecture {
    X86_64,
    Aarch64,
}

/// Semantic version of restic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResticVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl ResticVersion {
    /// The oldest restic version ResticHub supports (FR-AG-11, NFR-CMP-03).
    pub const MIN_SUPPORTED: Self = Self {
        major: 0,
        minor: 18,
        patch: 1,
    };

    /// Parses `major.minor.patch`. A leading `v` is allowed.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.strip_prefix('v').unwrap_or(value);
        let mut parts = value.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = match parts.next() {
            Some(patch) => patch.parse().ok()?,
            None => 0,
        };
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for ResticVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// One parameter of a hook in the agent allow-list (FR-HK-02).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookParameterSpec {
    /// True when the value is a secret, so the preview masks it (AC-PF-01).
    pub secret: bool,
}

/// One hook of the agent allow-list that the agent reported (FR-HK-04).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowListedHook {
    /// Accepted arguments, by name (FR-HK-02).
    pub parameters: BTreeMap<String, HookParameterSpec>,
}

/// Facts that the compiler uses for one target agent (FR-PF-03).
#[derive(Debug, Clone)]
pub struct TargetFacts {
    /// Identifier of the agent in the control plane.
    pub agent_id: String,
    /// Human-readable label of the machine.
    pub agent_name: String,
    /// This machine's short name on the network.
    pub hostname: String,
    pub os: OperatingSystem,
    pub arch: Architecture,
    /// Key-value pairs for the `{{agent.label.<key>}}` variables.
    pub labels: BTreeMap<String, String>,
    pub restic_version: ResticVersion,
    /// Plan-local time for the date variables.
    pub now: OffsetDateTime,
    /// Hook allow-list metadata of this agent (FR-HK-04).
    pub allow_list: BTreeMap<String, AllowListedHook>,
}

#[cfg(test)]
mod tests {
    use super::ResticVersion;

    #[test]
    fn parses_and_orders_versions() {
        assert_eq!(
            ResticVersion::parse("0.18.1"),
            Some(ResticVersion {
                major: 0,
                minor: 18,
                patch: 1
            })
        );
        assert_eq!(
            ResticVersion::parse("v0.19.0"),
            Some(ResticVersion {
                major: 0,
                minor: 19,
                patch: 0
            })
        );
        assert!(ResticVersion::parse("0.19") < ResticVersion::parse("0.19.1"));
        assert!(ResticVersion::parse("0.18.1") < ResticVersion::parse("0.19.0"));
        assert_eq!(ResticVersion::parse("nonsense"), None);
    }
}
