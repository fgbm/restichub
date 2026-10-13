//! Response schemas of the API (ADR-0009).

use restichub_db::secrets::SecretMetadata;
use restichub_db::tenants::TenantSummary;
use restichub_db::{Role, TenantStatus};
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;

use restichub_db::{tenants::Tenant, users::Member};

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
}

impl From<&TenantSummary> for PlatformTenantView {
    fn from(tenant: &TenantSummary) -> Self {
        Self {
            id: tenant.id.to_string(),
            name: tenant.name.clone(),
            status: tenant.status,
            created_at: format_time(tenant.created_at),
            user_count: tenant.user_count,
        }
    }
}

fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
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
    /// In RFC 3339.
    pub updated_at: String,
    /// Null for an API token actor.
    pub created_by: Option<String>,
}

impl From<&SecretMetadata> for SecretView {
    fn from(secret: &SecretMetadata) -> Self {
        Self {
            id: secret.id.to_string(),
            name: secret.name.clone(),
            purpose: secret.purpose.clone(),
            created_at: format_time(secret.created_at),
            updated_at: format_time(secret.updated_at),
            created_by: secret.created_by.map(|id| id.to_string()),
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
