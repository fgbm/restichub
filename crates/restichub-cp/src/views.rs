//! The response schemas that the API returns (ADR-0009).

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
