//! ResticHub database support.
//!
//! This crate holds the connection pool, the migrations and the readiness check. It uses the
//! `sqlx` runtime query functions, not the compile-time query macros, so the build needs no
//! database (ADR-0023).
//!
//! This crate is the only crate that runs SQL. An API crate calls the typed repository functions
//! ([`tenants`], [`users`]). The scoped transactions ([`TenantTx`], [`PlatformTx`]) are the only
//! way to get a connection. `TenantTx::as_conn` and `PlatformTx::as_conn` are crate-internal.

use std::time::Duration;

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

pub mod agents;
pub mod alerts;
pub mod api_tokens;
pub mod audit;
pub mod auth;
pub mod ca;
pub mod channels;
pub mod dashboard;
pub mod delivery;
pub mod enrollment_tokens;
pub mod enums;
pub mod ids;
pub mod job_logs;
pub mod job_steps;
pub mod jobs;
pub mod kek;
pub mod maintenance;
pub mod memberships;
pub mod metrics;
pub mod plans;
pub mod profiles;
pub mod queue;
pub mod repositories;
pub mod restore;
pub mod secrets;
pub mod sessions;
pub mod setup;
pub mod signing;
pub mod tenants;
pub mod tx;
pub mod users;

#[cfg(feature = "test-util")]
pub mod test_util;

pub use enums::{
    ActorType, AdminState, AgentStatus, BackendKind, CaRole, HookAllowListState, JobKind,
    JobResultCode, JobStatus, LogLevel, LogStream, MaintenanceTask, MissedRunPolicy, Role,
    SigningKeyRole, StepStatus, TenantStatus,
};
pub use ids::{
    AgentId, AlertId, ApiTokenId, AuditLogId, ChannelId, DeliveryId, EnrollmentTokenId, JobId,
    MaintenanceScheduleId, PLATFORM_TENANT_ID, PlanId, ProfileId, RepositoryId, SecretId,
    SessionId, SetupTokenId, TenantId, UserId,
};
pub use tx::{EnrollmentTx, PlatformTx, TenantTx};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// The configuration of the database connection.
///
/// The URL holds no password. The caller gives the password to [`Database::connect`] and to
/// [`Database::connect_lazy`] separately, so a `Debug` of this type shows no secret.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub acquire_timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("cannot connect to the database")]
    Connect(#[source] sqlx::Error),
    #[error("the database URL is not valid")]
    InvalidUrl(#[source] sqlx::Error),
    #[error("cannot apply the database migrations")]
    Migrate(#[source] sqlx::migrate::MigrateError),
    #[error("the database query failed")]
    Query(#[source] sqlx::Error),
    #[error("the runtime database role is not safe: {cause}")]
    InsecureRuntimeRole { cause: String },
    #[error("the platform transaction did not create the tenant {0}")]
    TenantNotCreated(TenantId),
    #[error("the enrollment transaction holds no tenant; the token lookup must run first")]
    EnrollmentTenantUnset,
    #[error("the database function returned an unknown write status: {0}")]
    UnknownWriteStatus(String),
    #[error("the enrollment function returned an unknown outcome: {0}")]
    UnknownEnrollmentOutcome(String),
    #[error("the database refused the account create")]
    AccountCreateRefused,
    #[error(
        "the database schema is behind (applied version {applied}, expected {expected}); run `restichub-cp migrate`"
    )]
    MigrationsBehind { applied: i64, expected: i64 },
    #[error(
        "the database schema is ahead (applied version {applied}, expected {expected}); the binary is older than the schema"
    )]
    MigrationsAhead { applied: i64, expected: i64 },
}

impl DbError {
    /// A stable short code of the failure, for a log line. It never holds a value.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Connect(_) => "connect",
            Self::InvalidUrl(_) => "invalid_url",
            Self::Migrate(_) => "migrate",
            Self::Query(_) => "query",
            Self::InsecureRuntimeRole { .. } => "insecure_runtime_role",
            Self::TenantNotCreated(_) => "tenant_not_created",
            Self::EnrollmentTenantUnset => "enrollment_tenant_unset",
            Self::UnknownWriteStatus(_) => "unknown_write_status",
            Self::UnknownEnrollmentOutcome(_) => "unknown_enrollment_outcome",
            Self::AccountCreateRefused => "account_create_refused",
            Self::MigrationsBehind { .. } => "migrations_behind",
            Self::MigrationsAhead { .. } => "migrations_ahead",
        }
    }

    /// True for a unique constraint violation (SQLSTATE 23505).
    pub fn is_unique_violation(&self) -> bool {
        self.sqlstate().as_deref() == Some("23505")
    }

    /// The name of the violated constraint, when the driver reports it.
    ///
    /// A caller uses it to tell two unique indexes apart, for example the username and the email of
    /// the account row.
    pub fn unique_constraint(&self) -> Option<String> {
        match self {
            Self::Query(error) => error
                .as_database_error()
                .and_then(|error| error.constraint())
                .map(str::to_owned),
            _ => None,
        }
    }

    /// True for a check constraint violation (SQLSTATE 23514).
    pub fn is_check_violation(&self) -> bool {
        self.sqlstate().as_deref() == Some("23514")
    }

    /// True for a foreign key violation (SQLSTATE 23503).
    pub fn is_foreign_key_violation(&self) -> bool {
        self.sqlstate().as_deref() == Some("23503")
    }

    pub(crate) fn sqlstate(&self) -> Option<String> {
        match self {
            Self::Query(error) => error
                .as_database_error()
                .and_then(|error| error.code())
                .map(|code| code.to_string()),
            _ => None,
        }
    }

    /// The message of the database error, when the driver reports it.
    pub fn database_message(&self) -> Option<String> {
        match self {
            Self::Query(error) => error
                .as_database_error()
                .map(|error| error.message().to_owned()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    /// The pool runs `RESET ALL` when a connection goes back to the pool, so a session-level
    /// setting never survives.
    pub async fn connect(config: &DatabaseConfig, password: Option<&str>) -> Result<Self, DbError> {
        let options = connect_options(config, password)?;
        let pool = with_session_reset(PgPoolOptions::new())
            .max_connections(config.max_connections)
            .acquire_timeout(config.acquire_timeout)
            .connect_with(options)
            .await
            .map_err(DbError::Connect)?;
        Ok(Self { pool })
    }

    /// Creates a pool that connects to the database on the first use.
    ///
    /// A test uses this function with a URL that points to no database. The readiness check then
    /// fails without a running PostgreSQL.
    pub fn connect_lazy(config: &DatabaseConfig, password: Option<&str>) -> Result<Self, DbError> {
        let options = connect_options(config, password)?;
        let pool = with_session_reset(PgPoolOptions::new())
            .max_connections(config.max_connections)
            .acquire_timeout(config.acquire_timeout)
            .connect_lazy_with(options);
        Ok(Self { pool })
    }

    /// The method is not public. A caller uses [`Database::begin_tenant`] or
    /// [`Database::begin_platform`]. Only the migrations, the readiness check and the role check
    /// use the pool directly.
    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The transaction sets the runtime role and the tenant scope. Row-Level Security shows only
    /// the rows of this tenant.
    pub async fn begin_tenant(&self, tenant: TenantId) -> Result<TenantTx<'_>, DbError> {
        TenantTx::begin(&self.pool, tenant).await
    }

    /// The transaction sets the runtime role and the platform scope.
    pub async fn begin_platform(&self) -> Result<PlatformTx<'_>, DbError> {
        PlatformTx::begin(&self.pool).await
    }

    /// The transaction sets the runtime role and no tenant scope, for the enrollment exchange that
    /// reads the token before it knows the tenant (FR-ID-01, FR-TN-03).
    pub async fn begin_enrollment(&self) -> Result<EnrollmentTx<'_>, DbError> {
        EnrollmentTx::begin(&self.pool).await
    }

    /// Applies the embedded migrations and logs the applied version.
    ///
    /// The migration login is the owner of the installation database. The control plane uses a
    /// short-lived owner connection for this call and closes it.
    pub async fn migrate(&self) -> Result<(), DbError> {
        MIGRATOR.run(&self.pool).await.map_err(DbError::Migrate)?;
        let version = MIGRATOR.iter().map(|migration| migration.version).max();
        tracing::info!(version = ?version, "database migrations applied");
        Ok(())
    }

    pub async fn is_ready(&self) -> bool {
        match sqlx::query("SELECT 1").execute(self.pool()).await {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(%error, "the database readiness check failed");
                false
            }
        }
    }

    /// Refuses a runtime setup that is not safe.
    ///
    /// The check refuses a runtime setup where any of these conditions is true:
    ///
    /// - `restichub_app` has SUPERUSER, BYPASSRLS, CREATEROLE or CREATEDB.
    /// - the runtime login is a superuser or has BYPASSRLS.
    /// - the runtime login can become a role with SUPERUSER or BYPASSRLS.
    /// - the runtime login, or a role it can become, owns a relation, a namespace, a function or
    ///   the current database, or is a member of `pg_database_owner`.
    /// - the runtime login is not a member of `restichub_app`.
    ///
    /// The control plane stops the start if a check fails.
    pub async fn verify_runtime_role(&self) -> Result<(), DbError> {
        self.verify_runtime_role_for("restichub_app").await
    }

    /// Checks the runtime login against a named group role.
    ///
    /// The public API uses `restichub_app`. A test uses a per-test substitute role, because the
    /// group role is cluster-wide.
    pub(crate) async fn verify_runtime_role_for(&self, app_role: &str) -> Result<(), DbError> {
        let app: Option<(bool, bool, bool, bool)> = sqlx::query_as(
            "SELECT rolsuper, rolbypassrls, rolcreaterole, rolcreatedb \
             FROM pg_roles WHERE rolname = $1",
        )
        .bind(app_role)
        .fetch_optional(&self.pool)
        .await
        .map_err(DbError::Query)?;
        let Some((app_super, app_bypass, app_createrole, app_createdb)) = app else {
            return Err(DbError::InsecureRuntimeRole {
                cause: format!("the role {app_role} is missing"),
            });
        };
        if app_super || app_bypass || app_createrole || app_createdb {
            return Err(DbError::InsecureRuntimeRole {
                cause: format!(
                    "the role {app_role} has a forbidden attribute (SUPERUSER, BYPASSRLS, CREATEROLE or CREATEDB)"
                ),
            });
        }

        let (login_unsafe, reachable_unsafe): (bool, i64) = sqlx::query_as(
            "SELECT \
                coalesce((SELECT role.rolsuper OR role.rolbypassrls \
                          FROM pg_roles role WHERE role.rolname = current_user), true), \
                (SELECT count(*) FROM pg_roles role \
                 WHERE (role.rolsuper OR role.rolbypassrls) \
                   AND pg_has_role(current_user, role.oid, 'SET'))",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(DbError::Query)?;
        if login_unsafe {
            return Err(DbError::InsecureRuntimeRole {
                cause: "the runtime login is a superuser or has BYPASSRLS".to_owned(),
            });
        }
        if reachable_unsafe != 0 {
            return Err(DbError::InsecureRuntimeRole {
                cause: "the runtime login can become a role with SUPERUSER or BYPASSRLS".to_owned(),
            });
        }

        let (relations, namespaces, functions, databases, database_owners): (
            i64,
            i64,
            i64,
            i64,
            i64,
        ) = sqlx::query_as(
            "SELECT \
                (SELECT count(*) FROM pg_class c \
                 JOIN pg_roles role ON c.relowner = role.oid \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
                   AND pg_has_role(current_user, role.oid, 'SET')), \
                (SELECT count(*) FROM pg_namespace n \
                 JOIN pg_roles role ON n.nspowner = role.oid \
                 WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
                   AND pg_has_role(current_user, role.oid, 'SET')), \
                (SELECT count(*) FROM pg_proc p \
                 JOIN pg_roles role ON p.proowner = role.oid \
                 JOIN pg_namespace n ON n.oid = p.pronamespace \
                 WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
                   AND pg_has_role(current_user, role.oid, 'SET')), \
                (SELECT count(*) FROM pg_database d \
                 JOIN pg_roles role ON d.datdba = role.oid \
                 WHERE d.datname = current_database() \
                   AND pg_has_role(current_user, role.oid, 'SET')), \
                (SELECT count(*) FROM pg_roles role \
                 WHERE pg_has_role(role.oid, \
                       (SELECT oid FROM pg_roles WHERE rolname = 'pg_database_owner'), 'MEMBER') \
                   AND pg_has_role(current_user, role.oid, 'SET'))",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(DbError::Query)?;
        if relations + namespaces + functions + databases + database_owners != 0 {
            return Err(DbError::InsecureRuntimeRole {
                cause: "the runtime login or a role it can become owns an object, a namespace, a function or the current database, or is a member of pg_database_owner".to_owned(),
            });
        }

        let is_member: (bool,) = sqlx::query_as("SELECT pg_has_role(current_user, $1, 'MEMBER')")
            .bind(app_role)
            .fetch_one(&self.pool)
            .await
            .map_err(DbError::Query)?;
        if !is_member.0 {
            return Err(DbError::InsecureRuntimeRole {
                cause: format!("the runtime login is not a member of {app_role}"),
            });
        }
        Ok(())
    }

    /// Checks that the applied migrations match the embedded migrations.
    ///
    /// The server calls this method at start. The runtime role needs `SELECT` on
    /// `_sqlx_migrations` (migration 0007). The method refuses a schema that is behind (run
    /// `restichub-cp migrate`) or ahead (the binary is older than the schema).
    pub async fn verify_migrations(&self) -> Result<(), DbError> {
        let expected = MIGRATOR
            .iter()
            .map(|migration| migration.version)
            .max()
            .unwrap_or(0);
        let applied: Result<Option<(i64,)>, sqlx::Error> =
            sqlx::query_as::<_, (i64,)>("SELECT max(version) FROM _sqlx_migrations WHERE success")
                .fetch_optional(&self.pool)
                .await;
        match applied {
            Ok(Some((version,))) => {
                if version < expected {
                    Err(DbError::MigrationsBehind {
                        applied: version,
                        expected,
                    })
                } else if version > expected {
                    Err(DbError::MigrationsAhead {
                        applied: version,
                        expected,
                    })
                } else {
                    Ok(())
                }
            }
            Ok(None) => Err(DbError::MigrationsBehind {
                applied: 0,
                expected,
            }),
            Err(error) if is_undefined_table(&error) => Err(DbError::MigrationsBehind {
                applied: 0,
                expected,
            }),
            Err(error) => Err(DbError::Query(error)),
        }
    }

    /// The control plane uses a short-lived migration database. It closes that pool after the
    /// migrations.
    pub async fn close(self) {
        self.pool.close().await;
    }
}

fn connect_options(
    config: &DatabaseConfig,
    password: Option<&str>,
) -> Result<PgConnectOptions, DbError> {
    let options = config
        .url
        .parse::<PgConnectOptions>()
        .map_err(DbError::InvalidUrl)?;
    match password {
        Some(password) => Ok(options.password(password)),
        None => Ok(options),
    }
}

/// Adds a hook that runs `RESET ROLE` and `RESET ALL` when a connection goes back to the pool.
///
/// A session-level `SET ROLE` and a session-level `SET` then never survive on a pooled connection.
fn with_session_reset(options: PgPoolOptions) -> PgPoolOptions {
    options.after_release(|connection, _metadata| {
        Box::pin(async move {
            sqlx::query("RESET ROLE").execute(&mut *connection).await?;
            sqlx::query("RESET ALL").execute(&mut *connection).await?;
            Ok(true)
        })
    })
}

fn is_undefined_table(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code == "42P01")
}

#[cfg(all(test, feature = "test-util"))]
mod tests;
