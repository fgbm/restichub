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
    /// The Platform Admin manages tenants.
    PlatformAdmin,
    /// The Admin manages one tenant.
    Admin,
    /// The Operator runs jobs.
    Operator,
    /// The Viewer reads data.
    Viewer,
}

impl Role {
    /// Returns the stored string of the role.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlatformAdmin => "platform_admin",
            Self::Admin => "admin",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
        }
    }

    /// Parses the stored string of a role. The function returns `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "platform_admin" => Some(Self::PlatformAdmin),
            "admin" => Some(Self::Admin),
            "operator" => Some(Self::Operator),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }

    /// Returns the rank of the role. A higher rank includes the lower roles.
    pub fn rank(self) -> u8 {
        match self {
            Self::PlatformAdmin => 4,
            Self::Admin => 3,
            Self::Operator => 2,
            Self::Viewer => 1,
        }
    }

    /// Returns true if the role includes the minimum role.
    pub fn allows(self, minimum: Self) -> bool {
        self.rank() >= minimum.rank()
    }

    /// Returns true for the administrator roles. The count of the last Admin uses it.
    pub fn is_admin(self) -> bool {
        matches!(self, Self::Admin | Self::PlatformAdmin)
    }
}

/// The status of a tenant (FR-TN-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum TenantStatus {
    /// The tenant signs in and its API tokens work.
    Active,
    /// A suspended tenant cannot sign in and its API tokens fail (AC-TN-03).
    Suspended,
    /// A deleted tenant keeps its rows; the retention task removes them later.
    Deleted,
}

impl TenantStatus {
    /// Returns the stored string of the status.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
            Self::Deleted => "deleted",
        }
    }

    /// Parses the stored string of a status. The function returns `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "suspended" => Some(Self::Suspended),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

/// The type of an audit actor (ADR-0023, section Audit log).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum ActorType {
    /// A local user.
    User,
    /// An API token.
    ApiToken,
    /// An agent.
    Agent,
    /// The control plane.
    System,
}

impl ActorType {
    /// Returns the stored string of the actor type.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::ApiToken => "api_token",
            Self::Agent => "agent",
            Self::System => "system",
        }
    }

    /// Parses the stored string of an actor type. The function returns `None` on an unknown string.
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
    use super::{ActorType, Role, TenantStatus};

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
