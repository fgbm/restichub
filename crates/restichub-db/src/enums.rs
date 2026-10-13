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
