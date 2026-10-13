//! The closed sets that the control plane stores (ADR-0023).
//!
//! The database and the control plane share these types. The string value of each variant is the
//! stored value of a text column. It is stable: do not change a string.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The role of an actor in a tenant (requirements section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum Role {
    PlatformAdmin,
    Admin,
    Operator,
    Viewer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlatformAdmin => "platform_admin",
            Self::Admin => "admin",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "platform_admin" => Some(Self::PlatformAdmin),
            "admin" => Some(Self::Admin),
            "operator" => Some(Self::Operator),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }

    /// Higher roles include the lower roles.
    pub fn rank(self) -> u8 {
        match self {
            Self::PlatformAdmin => 4,
            Self::Admin => 3,
            Self::Operator => 2,
            Self::Viewer => 1,
        }
    }

    pub fn allows(self, minimum: Self) -> bool {
        self.rank() >= minimum.rank()
    }

    /// Used by the count of the last Admin.
    pub fn is_admin(self) -> bool {
        matches!(self, Self::Admin | Self::PlatformAdmin)
    }
}

/// Tenant status (FR-TN-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum TenantStatus {
    Active,
    /// Cannot sign in; its API tokens fail (AC-TN-03).
    Suspended,
    /// Keeps its rows; the retention task removes them later.
    Deleted,
}

impl TenantStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Deleted => "deleted",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "suspended" => Some(Self::Suspended),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

/// The derived administrator state of a tenant (ADR-0028).
///
/// The control plane computes the state from the memberships: an active Admin count of zero gives
/// [`AdminState::None`]. The state is not stored, so it cannot drift from the memberships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdminState {
    None,
    Present,
}

impl AdminState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Present => "present",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "present" => Some(Self::Present),
            _ => None,
        }
    }

    /// Zero active administrators give [`AdminState::None`].
    pub fn from_active_admins(count: i64) -> Self {
        if count == 0 {
            Self::None
        } else {
            Self::Present
        }
    }
}

/// The role of a certificate authority (ADR-0026). The string value is part of the associated
/// data of the wrapped key; do not change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum CaRole {
    Root,
    Issuing,
}

impl CaRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Issuing => "issuing",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "root" => Some(Self::Root),
            "issuing" => Some(Self::Issuing),
            _ => None,
        }
    }
}

/// The role of a job signing key (ADR-0026, section Job signing keys). The string value is part of
/// the associated data of the wrapped key; do not change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum SigningKeyRole {
    /// Signs the jobs now.
    Current,
    /// Delivered to the agents and promoted on the next rotation.
    Next,
}

impl SigningKeyRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Next => "next",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "current" => Some(Self::Current),
            "next" => Some(Self::Next),
            _ => None,
        }
    }
}

/// Audit actor type (ADR-0023, section Audit log).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum ActorType {
    User,
    ApiToken,
    Agent,
    System,
}

impl ActorType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::ApiToken => "api_token",
            Self::Agent => "agent",
            Self::System => "system",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "api_token" => Some(Self::ApiToken),
            "agent" => Some(Self::Agent),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ActorType, AdminState, CaRole, Role, SigningKeyRole, TenantStatus};

    #[test]
    fn the_role_strings_are_stable() {
        for role in [
            Role::PlatformAdmin,
            Role::Admin,
            Role::Operator,
            Role::Viewer,
        ] {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse("nope"), None);
    }

    #[test]
    fn the_tenant_status_strings_are_stable() {
        for status in [
            TenantStatus::Active,
            TenantStatus::Suspended,
            TenantStatus::Deleted,
        ] {
            assert_eq!(TenantStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(TenantStatus::parse("nope"), None);
    }

    #[test]
    fn the_admin_state_strings_are_stable() {
        for state in [AdminState::None, AdminState::Present] {
            assert_eq!(AdminState::parse(state.as_str()), Some(state));
        }
        assert_eq!(AdminState::parse("nope"), None);
        assert_eq!(AdminState::from_active_admins(0), AdminState::None);
        assert_eq!(AdminState::from_active_admins(2), AdminState::Present);
    }

    #[test]
    fn the_ca_role_strings_are_stable() {
        for role in [CaRole::Root, CaRole::Issuing] {
            assert_eq!(CaRole::parse(role.as_str()), Some(role));
        }
        assert_eq!(CaRole::parse("nope"), None);
    }

    #[test]
    fn the_signing_key_role_strings_are_stable() {
        for role in [SigningKeyRole::Current, SigningKeyRole::Next] {
            assert_eq!(SigningKeyRole::parse(role.as_str()), Some(role));
        }
        assert_eq!(SigningKeyRole::parse("nope"), None);
    }

    #[test]
    fn the_actor_type_strings_are_stable() {
        for actor_type in [
            ActorType::User,
            ActorType::ApiToken,
            ActorType::Agent,
            ActorType::System,
        ] {
            assert_eq!(ActorType::parse(actor_type.as_str()), Some(actor_type));
        }
        assert_eq!(ActorType::parse("nope"), None);
    }
}
