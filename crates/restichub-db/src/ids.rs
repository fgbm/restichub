//! The identifier newtypes of the control plane database.
//!
//! Each newtype holds a UUID. The IDs are UUIDv7, generated in Rust. The database needs no
//! extension for the generation (ADR-0023, section Database).

use uuid::Uuid;

/// The identifier of a tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub struct TenantId(Uuid);

/// The identifier of a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub struct UserId(Uuid);

/// The identifier of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub struct SessionId(Uuid);

/// The identifier of an API token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub struct ApiTokenId(Uuid);

/// The identifier of a secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub struct SecretId(Uuid);

/// The identifier of an audit record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub struct AuditLogId(Uuid);

/// The identifier of a one-time setup token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, sqlx::Type)]
#[sqlx(transparent)]
pub struct SetupTokenId(Uuid);

/// The reserved platform tenant (ADR-0023, section Tenant isolation).
///
/// The value is the constant `id` of the reserved row in the `tenants` table. The Platform Admin
/// users and the platform audit records belong to this tenant.
pub const PLATFORM_TENANT_ID: TenantId = TenantId(Uuid::from_bytes([
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x70, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
]));

macro_rules! id_type {
    ($type:ident) => {
        impl $type {
            /// Creates the identifier from a UUID.
            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// Generates a new identifier (UUIDv7).
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Returns the UUID.
            pub fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $type {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<Uuid> for $type {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl From<$type> for Uuid {
            fn from(id: $type) -> Self {
                id.0
            }
        }

        impl std::fmt::Display for $type {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0, formatter)
            }
        }

        impl std::str::FromStr for $type {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(value)?))
            }
        }
    };
}

id_type!(TenantId);
id_type!(UserId);
id_type!(SessionId);
id_type!(ApiTokenId);
id_type!(SecretId);
id_type!(AuditLogId);
id_type!(SetupTokenId);
