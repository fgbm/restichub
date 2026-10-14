/// An optional feature that an agent announces in `AgentHello.capabilities` (FR-PR-09).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentCapability {
    /// The agent reads `SnapshotsFilter.all_tags`. An older agent drops that field.
    ForgetAllTags,
}

impl AgentCapability {
    /// Every capability that this build supports.
    pub const ALL: [Self; 1] = [Self::ForgetAllTags];

    /// The wire name in `AgentHello.capabilities`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ForgetAllTags => "forget-all-tags",
        }
    }

    /// True when the list names the capability.
    #[must_use]
    pub fn is_listed(self, capabilities: &[String]) -> bool {
        capabilities.iter().any(|name| name == self.as_str())
    }
}
