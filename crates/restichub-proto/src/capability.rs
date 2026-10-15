/// An optional feature that an agent announces in `AgentHello.capabilities` (FR-PR-09).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCapability {
    /// The agent reads `SnapshotsFilter.all_tags`. An older agent drops that field.
    ForgetAllTags,
    /// The agent runs the `ls` and `restore` steps. An older agent rejects both (FR-RS-02, FR-RS-03).
    RestoreV1,
}

impl AgentCapability {
    /// Every capability that this build supports.
    pub const ALL: [Self; 2] = [Self::ForgetAllTags, Self::RestoreV1];

    /// The wire name in `AgentHello.capabilities`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ForgetAllTags => "forget-all-tags",
            Self::RestoreV1 => "restore-v1",
        }
    }

    /// The capability with this wire name. An unknown name gives `None`.
    #[must_use]
    pub fn from_wire(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|known| known.as_str() == name)
    }

    /// The known capabilities in a wire list, without duplicates. Unknown names are ignored, so the
    /// result never holds more than [`Self::ALL`].
    #[must_use]
    pub fn parse_list(names: &[String]) -> Vec<Self> {
        let mut parsed = Vec::new();
        for name in names {
            if let Some(capability) = Self::from_wire(name)
                && !parsed.contains(&capability)
            {
                parsed.push(capability);
                if parsed.len() == Self::ALL.len() {
                    break;
                }
            }
        }
        parsed
    }

    /// True when the list names the capability.
    #[must_use]
    pub fn is_listed(self, capabilities: &[String]) -> bool {
        capabilities.iter().any(|name| name == self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_pr_09_a_hello_with_junk_gives_the_known_capabilities_once() {
        let mut names: Vec<String> = (0..10_000).map(|index| format!("junk-{index}")).collect();
        names.push("forget-all-tags".to_owned());
        names.push("forget-all-tags".to_owned());
        names.push("restore-v1".to_owned());
        assert_eq!(
            AgentCapability::parse_list(&names),
            [AgentCapability::ForgetAllTags, AgentCapability::RestoreV1]
        );
        assert!(AgentCapability::parse_list(&names[..10_000]).is_empty());
    }
}
