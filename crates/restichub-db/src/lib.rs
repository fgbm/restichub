//! ResticHub database support.
//!
//! This crate holds the connection pool, the migrations and the readiness check. It uses the
//! `sqlx` runtime query functions, not the compile-time query macros, so the build needs no
//! database (ADR-0023).
//!
//! This crate is the only crate that runs SQL. An API crate calls the typed repository functions
//! ([`tenants`], [`users`]). The scoped transactions ([`TenantTx`], [`PlatformTx`]) are the only
//! way to get a connection. `TenantTx::as_conn` and `PlatformTx::as_conn` are crate-internal (H1).

use std::time::Duration;

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

pub mod api_tokens;
pub mod audit;
pub mod auth;
pub mod delivery;
pub mod ids;
pub mod kek;
pub mod memberships;
pub mod secrets;
pub mod sessions;
pub mod setup;
pub mod tenants;
pub mod tx;
pub mod users;

pub use ids::{
    ApiTokenId, AuditLogId, PLATFORM_TENANT_ID, SecretId, SessionId, SetupTokenId, TenantId, UserId,
};
pub use tx::{PlatformTx, TenantTx};

/// The embedded migrations of the control plane database.
static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// The configuration of the database connection.
///
/// The URL holds no password. The caller gives the password to [`Database::connect`] and to
/// [`Database::connect_lazy`] separately, so a `Debug` of this type shows no secret.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// The connection URL.
    pub url: String,
    /// The maximum number of connections in the pool.
    pub max_connections: u32,
    /// The time to wait for a connection from the pool.
    pub acquire_timeout: Duration,
}

/// The error of a database operation.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// The connection to the database failed.
    #[error("cannot connect to the database")]
    Connect(#[source] sqlx::Error),
    /// The URL is not valid.
    #[error("the database URL is not valid")]
    InvalidUrl(#[source] sqlx::Error),
    /// The migrations failed.
    #[error("cannot apply the database migrations")]
    Migrate(#[source] sqlx::migrate::MigrateError),
    /// A database query failed.
    #[error("the database query failed")]
    Query(#[source] sqlx::Error),
    /// The runtime role of the database is not safe.
    #[error("the runtime database role is not safe: {cause}")]
    InsecureRuntimeRole {
        /// The cause of the refusal.
        cause: String,
    },
    /// The platform transaction did not create this tenant.
    #[error("the platform transaction did not create the tenant {0}")]
    TenantNotCreated(TenantId),
    /// The database schema is behind the embedded migrations.
    #[error(
        "the database schema is behind (applied version {applied}, expected {expected}); run `restichub-cp migrate`"
    )]
    MigrationsBehind {
        /// The applied version.
        applied: i64,
        /// The expected version.
        expected: i64,
    },
    /// The database schema is ahead of the embedded migrations.
    #[error(
        "the database schema is ahead (applied version {applied}, expected {expected}); the binary is older than the schema"
    )]
    MigrationsAhead {
        /// The applied version.
        applied: i64,
        /// The expected version.
        expected: i64,
    },
}

impl DbError {
    /// Returns true if the error is a unique constraint violation (SQLSTATE 23505).
    pub fn is_unique_violation(&self) -> bool {
        self.sqlstate().as_deref() == Some("23505")
    }

    /// Returns true if the error is a check constraint violation (SQLSTATE 23514).
    pub fn is_check_violation(&self) -> bool {
        self.sqlstate().as_deref() == Some("23514")
    }

    /// Returns the SQLSTATE of a query error.
    fn sqlstate(&self) -> Option<String> {
        match self {
            Self::Query(error) => error
                .as_database_error()
                .and_then(|error| error.code())
                .map(|code| code.to_string()),
            _ => None,
        }
    }
}

/// The database of the control plane: a connection pool and the migrations.
#[derive(Debug, Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    /// Connects to the database and returns the pool.
    ///
    /// The function uses `password` if it is set. The pool runs `RESET ALL` when a connection goes
    /// back to the pool, so a session-level setting never survives (M2).
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

    /// Returns the connection pool.
    ///
    /// The method is not public. A caller uses [`Database::begin_tenant`] or
    /// [`Database::begin_platform`]. Only the migrations, the readiness check and the role check
    /// use the pool directly.
    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Begins a transaction for one tenant.
    ///
    /// The transaction sets the runtime role and the tenant scope. Row-Level Security shows only
    /// the rows of this tenant.
    pub async fn begin_tenant(&self, tenant: TenantId) -> Result<TenantTx<'_>, DbError> {
        TenantTx::begin(&self.pool, tenant).await
    }

    /// Begins a transaction for a platform operation.
    ///
    /// The transaction sets the runtime role and the platform scope.
    pub async fn begin_platform(&self) -> Result<PlatformTx<'_>, DbError> {
        PlatformTx::begin(&self.pool).await
    }

    /// Applies the embedded migrations and logs the applied version.
    ///
    /// The migration login is the owner of the installation database. The control plane uses a
    /// short-lived owner connection for this call and closes it (D1).
    pub async fn migrate(&self) -> Result<(), DbError> {
        MIGRATOR.run(&self.pool).await.map_err(DbError::Migrate)?;
        let version = MIGRATOR.iter().map(|migration| migration.version).max();
        tracing::info!(version = ?version, "database migrations applied");
        Ok(())
    }

    /// Checks that the database answers a query.
    pub async fn is_ready(&self) -> bool {
        match sqlx::query("SELECT 1").execute(self.pool()).await {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(%error, "the database readiness check failed");
                false
            }
        }
    }

    /// Checks that the runtime login is safe (D1, N1).
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
    /// group role is cluster-wide (N1).
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

    /// Checks that the applied migrations match the embedded migrations (N4).
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

    /// Closes the pool.
    ///
    /// The control plane uses a short-lived migration database. It closes that pool after the
    /// migrations (D1).
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
/// A session-level `SET ROLE` and a session-level `SET` then never survive on a pooled connection
/// (M2, N7).
fn with_session_reset(options: PgPoolOptions) -> PgPoolOptions {
    options.after_release(|connection, _metadata| {
        Box::pin(async move {
            sqlx::query("RESET ROLE").execute(&mut *connection).await?;
            sqlx::query("RESET ALL").execute(&mut *connection).await?;
            Ok(true)
        })
    })
}

/// Returns true if the error says that the relation does not exist.
fn is_undefined_table(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .is_some_and(|code| code == "42P01")
}

#[cfg(feature = "test-util")]
pub mod test_util {
    //! A test database with the migrations applied.
    //!
    //! The helper reads `RESTICHUB_TEST_DATABASE_URL`. If the variable is not set, the test prints
    //! a message and skips.
    //!
    //! The helper uses the URL as an administrator (superuser) connection only to set up the
    //! cluster roles, to create the per-test logins and to create the database. The migrations run
    //! as the per-test owner login. The tests read through the per-test runtime login.

    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use sqlx::{AssertSqlSafe, PgPool};
    use uuid::Uuid;

    use super::{Database, MIGRATOR};

    /// The advisory lock key that serializes the cluster-wide role setup across test processes.
    const ROLE_SETUP_LOCK: i64 = 0x7265_7374_6963_6875;

    /// A fresh test database.
    ///
    /// The value removes the database and the two logins on drop (best effort). Call
    /// [`TestDatabase::cleanup`] at the end of the test for a correct removal.
    #[derive(Debug)]
    pub struct TestDatabase {
        database: Database,
        owner: PgPool,
        admin: PgConnectOptions,
        runtime_options: PgConnectOptions,
        owner_role: String,
        runtime_role: String,
        name: String,
    }

    impl TestDatabase {
        /// Returns the runtime database handle.
        ///
        /// The pool connects as the runtime login.
        pub fn database(&self) -> &Database {
            &self.database
        }

        /// Returns the runtime connection pool.
        pub fn pool(&self) -> &PgPool {
            self.database.pool()
        }

        /// Returns the owner connection pool.
        ///
        /// The tests use it for the owner-level checks, for example the audit trigger.
        pub fn owner_pool(&self) -> &PgPool {
            &self.owner
        }

        /// Returns the runtime connection options.
        pub fn connect_options(&self) -> PgConnectOptions {
            self.runtime_options.clone()
        }

        /// Returns the name of the per-test owner login.
        pub fn owner_role(&self) -> &str {
            &self.owner_role
        }

        /// Returns the name of the per-test runtime login.
        pub fn runtime_role(&self) -> &str {
            &self.runtime_role
        }

        /// Returns the administrator (superuser) connection options.
        ///
        /// A test uses the options to create a scratch role or a scratch database (N1).
        pub fn admin_options(&self) -> PgConnectOptions {
            self.admin.clone()
        }

        /// Returns the runtime connection options.
        ///
        /// A test uses the options to open a pool that points to another database (N1).
        pub fn runtime_options(&self) -> PgConnectOptions {
            self.runtime_options.clone()
        }

        /// Opens a runtime pool with a given size.
        ///
        /// A test uses this method with size one to check that a setting never survives on one
        /// pooled connection.
        pub async fn runtime_pool_with_max(&self, max: u32) -> Result<PgPool, sqlx::Error> {
            super::with_session_reset(PgPoolOptions::new().max_connections(max))
                .connect_with(self.runtime_options.clone())
                .await
        }

        /// Opens a runtime database with a given pool size.
        ///
        /// A test uses this method to check the behavior of a small connection pool.
        pub async fn database_with_max(&self, max: u32) -> Result<Database, sqlx::Error> {
            let pool = super::with_session_reset(PgPoolOptions::new().max_connections(max))
                .connect_with(self.runtime_options.clone())
                .await?;
            Ok(Database { pool })
        }

        /// Removes the test database and the two logins.
        pub async fn cleanup(&self) {
            self.database.pool().close().await;
            self.owner.close().await;
            cleanup_cluster(
                &self.admin,
                &self.name,
                &self.owner_role,
                &self.runtime_role,
            )
            .await;
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let admin = self.admin.clone();
                let name = self.name.clone();
                let owner_role = self.owner_role.clone();
                let runtime_role = self.runtime_role.clone();
                handle.spawn(async move {
                    cleanup_cluster(&admin, &name, &owner_role, &runtime_role).await;
                });
            }
        }
    }

    /// Creates a new test database and applies the migrations.
    ///
    /// The function returns `None` if `RESTICHUB_TEST_DATABASE_URL` is not set or if the setup
    /// fails.
    pub async fn test_database() -> Option<TestDatabase> {
        let Ok(url) = std::env::var("RESTICHUB_TEST_DATABASE_URL") else {
            println!("skipped: RESTICHUB_TEST_DATABASE_URL is not set");
            return None;
        };
        let admin = url.parse::<PgConnectOptions>().ok()?;
        let admin_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(admin.clone())
            .await
            .ok()?;

        // The cluster roles. The advisory lock serializes the creation across test processes.
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(ROLE_SETUP_LOCK)
            .execute(&admin_pool)
            .await
            .ok()?;
        ensure_role(
            &admin_pool,
            "restichub_app",
            "NOLOGIN NOSUPERUSER NOBYPASSRLS NOCREATEROLE NOCREATEDB",
        )
        .await
        .ok()?;
        ensure_role(
            &admin_pool,
            "restichub_auth",
            "NOLOGIN NOSUPERUSER BYPASSRLS NOCREATEROLE NOCREATEDB",
        )
        .await
        .ok()?;

        // The per-test logins with random names and random passwords.
        let suffix = Uuid::now_v7().simple().to_string();
        let owner_role = format!("restichub_owner_{suffix}");
        let runtime_role = format!("restichub_runtime_{suffix}");
        let owner_password = Uuid::now_v7().simple().to_string();
        let runtime_password = Uuid::now_v7().simple().to_string();
        create_login(&admin_pool, &owner_role, &owner_password)
            .await
            .ok()?;
        create_login(&admin_pool, &runtime_role, &runtime_password)
            .await
            .ok()?;
        grant_member(&admin_pool, "restichub_app", &owner_role)
            .await
            .ok()?;
        grant_member(&admin_pool, "restichub_app", &runtime_role)
            .await
            .ok()?;
        grant_member(&admin_pool, "restichub_auth", &owner_role)
            .await
            .ok()?;

        let name = format!("restichub_test_{suffix}");
        create_database(&admin_pool, &name, &owner_role)
            .await
            .ok()?;

        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(ROLE_SETUP_LOCK)
            .execute(&admin_pool)
            .await
            .ok()?;
        admin_pool.close().await;

        // The migrations run as the owner login.
        let owner_options = admin
            .clone()
            .database(&name)
            .username(owner_role.as_str())
            .password(owner_password.as_str());
        let migrate_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(owner_options.clone())
            .await
            .ok()?;
        if MIGRATOR.run(&migrate_pool).await.is_err() {
            migrate_pool.close().await;
            cleanup_cluster(&admin, &name, &owner_role, &runtime_role).await;
            return None;
        }
        migrate_pool.close().await;

        // The runtime pool and the owner pool.
        let runtime_options = admin
            .clone()
            .database(&name)
            .username(runtime_role.as_str())
            .password(runtime_password.as_str());
        let runtime_pool = super::with_session_reset(PgPoolOptions::new().max_connections(5))
            .connect_with(runtime_options.clone())
            .await
            .ok()?;
        let owner_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(owner_options.clone())
            .await
            .ok()?;

        Some(TestDatabase {
            database: Database { pool: runtime_pool },
            owner: owner_pool,
            admin,
            runtime_options,
            owner_role,
            runtime_role,
            name,
        })
    }

    async fn ensure_role(pool: &PgPool, name: &str, attributes: &str) -> Result<(), sqlx::Error> {
        let statement = format!("CREATE ROLE \"{name}\" {attributes}");
        match sqlx::query(AssertSqlSafe(statement)).execute(pool).await {
            Ok(_) => Ok(()),
            Err(error) if is_duplicate(&error) => Ok(()),
            Err(error) => Err(error),
        }
    }

    async fn create_login(pool: &PgPool, name: &str, password: &str) -> Result<(), sqlx::Error> {
        // The name and the password come from a UUID and need no bind parameter. PostgreSQL does
        // not accept a bind parameter in `CREATE ROLE`.
        let statement = format!(
            "CREATE ROLE \"{name}\" LOGIN NOSUPERUSER NOBYPASSRLS NOCREATEROLE NOCREATEDB \
             PASSWORD '{password}'"
        );
        sqlx::query(AssertSqlSafe(statement)).execute(pool).await?;
        Ok(())
    }

    async fn grant_member(pool: &PgPool, group: &str, member: &str) -> Result<(), sqlx::Error> {
        let statement = format!("GRANT \"{group}\" TO \"{member}\"");
        sqlx::query(AssertSqlSafe(statement)).execute(pool).await?;
        Ok(())
    }

    async fn create_database(pool: &PgPool, name: &str, owner: &str) -> Result<(), sqlx::Error> {
        // The name and the owner come from a UUID and need no bind parameter.
        let statement = format!("CREATE DATABASE \"{name}\" OWNER \"{owner}\"");
        sqlx::query(AssertSqlSafe(statement)).execute(pool).await?;
        Ok(())
    }

    fn is_duplicate(error: &sqlx::Error) -> bool {
        match error.as_database_error().and_then(|error| error.code()) {
            Some(code) => code == "42710" || code == "23505",
            None => false,
        }
    }

    async fn cleanup_cluster(
        admin: &PgConnectOptions,
        name: &str,
        owner_role: &str,
        runtime_role: &str,
    ) {
        let Ok(pool) = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(admin.clone())
            .await
        else {
            return;
        };
        // The name comes from a UUID and needs no bind parameter. PostgreSQL does not accept a
        // bind parameter in `DROP DATABASE`.
        let _ = sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"
        )))
        .execute(&pool)
        .await;
        for role in [owner_role, runtime_role] {
            let statement = format!("DROP ROLE IF EXISTS \"{role}\"");
            let _ = sqlx::query(AssertSqlSafe(statement)).execute(&pool).await;
        }
        pool.close().await;
    }

    /// Counts the rows of one table in a tenant transaction.
    ///
    /// The function is a test helper. The table name comes from a fixed list in the test only.
    pub async fn count_rows(
        database: &Database,
        tenant: super::TenantId,
        table: &str,
    ) -> Result<i64, super::DbError> {
        let mut tx = database.begin_tenant(tenant).await?;
        let query = format!("SELECT count(*) FROM {table}");
        let (count,): (i64,) = sqlx::query_as(AssertSqlSafe(query))
            .fetch_one(tx.as_conn())
            .await
            .map_err(super::DbError::Query)?;
        tx.commit().await?;
        Ok(count)
    }
}

#[cfg(all(test, feature = "test-util"))]
mod tests;
