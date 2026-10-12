//! The response schemas that the API returns (ADR-0009).

use serde::Serialize;
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
