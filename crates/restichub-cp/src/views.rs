//! Response schemas of the API (ADR-0009).

use restichub_db::SecretUsageKind;
use restichub_db::memberships::PlatformMembership;
use restichub_db::secrets::{SecretMetadata, SecretUsage};
use restichub_db::tenants::TenantSummary;
use restichub_db::users::PlatformAccount;
use restichub_db::{AdminState, Role, TenantStatus};
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;

use restichub_db::{
    tenants::Tenant,
    users::{Account, Member},
};

#[derive(Debug, Serialize, ToSchema)]
pub struct UserView {
    pub id: String,
    pub username: String,
    pub display_name: String,
    /// The membership is disabled in the current tenant, or the account is disabled.
    pub disabled: bool,
    pub role: Role,
    /// The account holds an active membership in another active tenant.
    pub shared: bool,
}

impl UserView {
    pub fn from_member(member: &Member) -> Self {
        Self {
            id: member.id.to_string(),
            username: member.username.clone(),
            display_name: member.display_name.clone(),
            disabled: member.disabled || member.account_disabled,
            role: member.role,
            shared: member.shared,
        }
    }

    /// The account of a session with no organization (ADR-0029).
    pub fn from_account(account: &Account, role: Role) -> Self {
        Self {
            id: account.id.to_string(),
            username: account.username.clone(),
            display_name: account.display_name.clone(),
            disabled: account.disabled,
            role,
            shared: false,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TenantView {
    pub id: String,
    pub name: String,
}

impl From<&Tenant> for TenantView {
    fn from(tenant: &Tenant) -> Self {
        Self {
            id: tenant.id.to_string(),
            name: tenant.name.clone(),
        }
    }
}

/// A tenant in a platform administration response (FR-TN-02).
///
/// The view holds the user count only. It never holds a user list or a secret of the tenant.
#[derive(Debug, Serialize, ToSchema)]
pub struct PlatformTenantView {
    pub id: String,
    pub name: String,
    pub status: TenantStatus,
    /// In RFC 3339.
    pub created_at: String,
    pub user_count: i64,
    /// Derived from the memberships: `none` when the tenant has no active Admin (ADR-0028).
    pub admin_state: AdminState,
}

impl From<&TenantSummary> for PlatformTenantView {
    fn from(tenant: &TenantSummary) -> Self {
        Self {
            id: tenant.id.to_string(),
            name: tenant.name.clone(),
            status: tenant.status,
            created_at: format_time(tenant.created_at),
            user_count: tenant.user_count,
            admin_state: tenant.admin_state,
        }
    }
}

/// One membership of an account in the platform account list (ADR-0028).
#[derive(Debug, Serialize, ToSchema)]
pub struct PlatformMembershipView {
    pub tenant_id: String,
    pub tenant_name: String,
    pub role: Role,
    /// Off in this organization only.
    pub disabled: bool,
}

impl From<&PlatformMembership> for PlatformMembershipView {
    fn from(membership: &PlatformMembership) -> Self {
        Self {
            tenant_id: membership.tenant_id.to_string(),
            tenant_name: membership.tenant_name.clone(),
            role: membership.role,
            disabled: membership.disabled,
        }
    }
}

/// An installation account with its memberships (ADR-0028).
#[derive(Debug, Serialize, ToSchema)]
pub struct PlatformUserView {
    pub id: String,
    pub username: String,
    pub display_name: String,
    /// Unique in the installation when it is set.
    pub email: Option<String>,
    /// False until an invitation verifies the address; the MVP leaves it false.
    pub email_verified: bool,
    /// A Platform Admin set it for the whole installation.
    pub disabled: bool,
    /// True when the account holds the installation-wide right (ADR-0029).
    pub platform_admin: bool,
    pub memberships: Vec<PlatformMembershipView>,
}

impl PlatformUserView {
    pub fn new(account: &PlatformAccount, memberships: Vec<PlatformMembershipView>) -> Self {
        Self {
            id: account.id.to_string(),
            username: account.username.clone(),
            display_name: account.display_name.clone(),
            email: account.email.clone(),
            email_verified: account.email_verified,
            disabled: account.disabled,
            platform_admin: account.platform_admin,
            memberships,
        }
    }
}

fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

/// One object that references a secret.
#[derive(Debug, Serialize, ToSchema)]
pub struct SecretUsageView {
    pub kind: SecretUsageKind,
    pub id: String,
    pub name: String,
}

/// The metadata of a secret in a response (FR-RP-05).
///
/// The view never holds the value. The API cannot return a secret value after a user saves it
/// (AC-RP-02).
#[derive(Debug, Serialize, ToSchema)]
pub struct SecretView {
    pub id: String,
    pub name: String,
    pub purpose: String,
    /// In RFC 3339.
    pub created_at: String,
    /// The time of the last value replace. In RFC 3339.
    pub updated_at: String,
    /// Null for an API token actor.
    pub created_by: Option<String>,
    /// The repositories, channels and profiles that use the secret.
    pub used_by: Vec<SecretUsageView>,
}

impl SecretView {
    pub fn new(secret: &SecretMetadata, usage: &[SecretUsage]) -> Self {
        Self {
            id: secret.id.to_string(),
            name: secret.name.clone(),
            purpose: secret.purpose.clone(),
            created_at: format_time(secret.created_at),
            updated_at: format_time(secret.updated_at),
            created_by: secret.created_by.map(|id| id.to_string()),
            used_by: usage
                .iter()
                .filter(|item| item.secret_id == secret.id)
                .map(|item| SecretUsageView {
                    kind: item.kind,
                    id: item.object_id.to_string(),
                    name: item.object_name.clone(),
                })
                .collect(),
        }
    }
}

/// The metadata of one secret with the marker that the value is set (FR-RP-05).
///
/// The field `value` holds the marker `set` only. The API never returns the value (AC-RP-02).
#[derive(Debug, Serialize, ToSchema)]
pub struct SecretDetailView {
    pub id: String,
    pub name: String,
    pub purpose: String,
    /// In RFC 3339.
    pub created_at: String,
    /// In RFC 3339.
    pub updated_at: String,
    /// Null for an API token actor.
    pub created_by: Option<String>,
    pub value: String,
}

impl From<&SecretMetadata> for SecretDetailView {
    fn from(secret: &SecretMetadata) -> Self {
        Self {
            id: secret.id.to_string(),
            name: secret.name.clone(),
            purpose: secret.purpose.clone(),
            created_at: format_time(secret.created_at),
            updated_at: format_time(secret.updated_at),
            created_by: secret.created_by.map(|id| id.to_string()),
            value: "set".to_owned(),
        }
    }
}
