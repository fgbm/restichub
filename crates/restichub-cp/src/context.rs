//! The request context of an authenticated request (ADR-0023, section Roles).
//!
//! Authentication does not exist yet. This module holds a placeholder until issue #18. The
//! extractor [`RequestContext`] rejects every request with HTTP 401 unless a value is in the
//! request extensions. Issue #18 replaces the placeholder with the real session and API token
//! lookups.
//!
//! The module also holds the authorization table "operation to minimum role" (ADR-0023, section
//! Roles). Each handler declares its operation. The table test of AC-AU-02 comes with issue #18.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use restichub_db::TenantId;
use restichub_db::audit::Actor;

use crate::error::ApiError;
use crate::server::{AppState, request_id_of};

/// The role of an authenticated actor (requirements section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Returns the stable name of the role.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlatformAdmin => "platform_admin",
            Self::Admin => "admin",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
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
}

/// An operation of the API.
///
/// The authorization table gives the minimum role of each operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Read the audit records of one tenant.
    AuditRead,
    /// Read the audit records of the platform.
    PlatformAuditRead,
}

impl Operation {
    /// Returns the stable name of the operation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AuditRead => "audit.read",
            Self::PlatformAuditRead => "platform.audit.read",
        }
    }

    /// Returns true if the operation acts on the platform.
    pub fn is_platform(self) -> bool {
        matches!(self, Self::PlatformAuditRead)
    }
}

/// Returns the minimum role of an operation (ADR-0023, section Roles).
pub fn minimum_role(operation: Operation) -> Role {
    match operation {
        Operation::AuditRead => Role::Viewer,
        Operation::PlatformAuditRead => Role::PlatformAdmin,
    }
}

/// The tenant, the actor and the role of a request.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// The tenant of the request.
    pub tenant_id: TenantId,
    /// The actor of the request.
    pub actor: Actor,
    /// The role of the actor.
    pub role: Role,
}

impl RequestContext {
    /// Creates a request context.
    pub fn new(tenant_id: TenantId, actor: Actor, role: Role) -> Self {
        Self {
            tenant_id,
            actor,
            role,
        }
    }

    /// Checks the role of the context against the minimum role of the operation.
    ///
    /// A platform operation needs the platform tenant too.
    pub fn authorize(&self, operation: Operation, request_id: &str) -> Result<(), ApiError> {
        if operation.is_platform() && self.tenant_id != restichub_db::PLATFORM_TENANT_ID {
            return Err(ApiError::forbidden(request_id));
        }
        if self.role.allows(minimum_role(operation)) {
            Ok(())
        } else {
            Err(ApiError::forbidden(request_id))
        }
    }
}

impl FromRequestParts<AppState> for RequestContext {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match parts.extensions.get::<Self>() {
            Some(context) => Ok(context.clone()),
            None => Err(ApiError::unauthenticated(&request_id_of(parts))),
        }
    }
}
