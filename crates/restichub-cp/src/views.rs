//! The response schemas that the API returns (ADR-0009).

use restichub_db::secrets::SecretMetadata;
use restichub_db::tenants::TenantSummary;
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use utoipa::ToSchema;

use restichub_db::{tenants::Tenant, users::User};

/// A user in a response.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserView {
    /// The identifier of the user.
    pub id: String,
    /// The username.
    pub username: String,
    /// The name for the user interface.
    pub display_name: String,
    /// True if the user is disabled.
    pub disabled: bool,
    /// The role of the user in the tenant.
    pub role: String,
}

impl UserView {
    /// Builds the view from a user and a role.
    pub fn new(user: &User, role: &str) -> Self {
        Self {
            id: user.id.to_string(),
            username: user.username.clone(),
            display_name: user.display_name.clone(),
            disabled: user.disabled,
            role: role.to_owned(),
        }
    }
}

/// A tenant in a response.
#[derive(Debug, Serialize, ToSchema)]
pub struct TenantView {
    /// The identifier of the tenant.
    pub id: String,
    /// The name of the tenant.
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

/// A tenant in a platform administration response (issue #20, FR-TN-02).
///
/// The view holds the user count only. It never holds a user list or a secret of the tenant.
#[derive(Debug, Serialize, ToSchema)]
pub struct PlatformTenantView {
    /// The identifier of the tenant.
    pub id: String,
    /// The name of the tenant.
    pub name: String,
    /// The status: `active`, `suspended` or `deleted`.
    pub status: String,
    /// The time the tenant was created in RFC 3339.
    pub created_at: String,
    /// The number of users of the tenant.
    pub user_count: i64,
}

impl From<&TenantSummary> for PlatformTenantView {
    fn from(tenant: &TenantSummary) -> Self {
        Self {
            id: tenant.id.to_string(),
            name: tenant.name.clone(),
            status: tenant.status.clone(),
            created_at: format_time(tenant.created_at),
            user_count: tenant.user_count,
        }
    }
}

/// Formats a time as RFC 3339.
fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

/// The metadata of a secret in a response (issue #17, FR-RP-05).
///
/// The view never holds the value. The API cannot return a secret value after a user saves it
/// (AC-RP-02).
#[derive(Debug, Serialize, ToSchema)]
pub struct SecretView {
    /// The identifier of the secret.
    pub id: String,
    /// The name of the secret.
    pub name: String,
    /// The stable purpose of the value.
    pub purpose: String,
    /// The time the secret was created in RFC 3339.
    pub created_at: String,
    /// The time the value was last replaced in RFC 3339.
    pub updated_at: String,
    /// The identifier of the user that created the secret. It is null for an API token actor.
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

/// The metadata of one secret with the marker that the value is set (issue #17, FR-RP-05).
///
/// The field `value` holds the marker `set` only. The API never returns the value (AC-RP-02).
#[derive(Debug, Serialize, ToSchema)]
pub struct SecretDetailView {
    /// The identifier of the secret.
    pub id: String,
    /// The name of the secret.
    pub name: String,
    /// The stable purpose of the value.
    pub purpose: String,
    /// The time the secret was created in RFC 3339.
    pub created_at: String,
    /// The time the value was last replaced in RFC 3339.
    pub updated_at: String,
    /// The identifier of the user that created the secret. It is null for an API token actor.
    pub created_by: Option<String>,
    /// The marker `set`. The value itself never leaves the control plane.
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
