//! ResticHub database support.
//!
//! This crate holds the connection pool, the migrations and the readiness check. It uses the
//! `sqlx` runtime query functions, not the compile-time query macros, so the build needs no
//! database (ADR-0023).

use std::time::Duration;

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

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
}

/// The database of the control plane: a connection pool and the migrations.
#[derive(Debug, Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    /// Connects to the database and returns the pool.
    ///
    /// The function uses `password` if it is set.
    pub async fn connect(config: &DatabaseConfig, password: Option<&str>) -> Result<Self, DbError> {
        let options = connect_options(config, password)?;
        let pool = PgPoolOptions::new()
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
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .acquire_timeout(config.acquire_timeout)
            .connect_lazy_with(options);
        Ok(Self { pool })
    }

    /// Returns the connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Applies the embedded migrations and logs the applied version.
    pub async fn migrate(&self) -> Result<(), DbError> {
        MIGRATOR.run(&self.pool).await.map_err(DbError::Migrate)?;
        let version = MIGRATOR.iter().map(|migration| migration.version).max();
        tracing::info!(version = ?version, "database migrations applied");
        Ok(())
    }

    /// Checks that the database answers a query.
    pub async fn is_ready(&self) -> bool {
        match sqlx::query("SELECT 1").execute(&self.pool).await {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(%error, "the database readiness check failed");
                false
            }
        }
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

#[cfg(feature = "test-util")]
pub mod test_util {
    //! A test database with the migrations applied.
    //!
    //! The helper reads `RESTICHUB_TEST_DATABASE_URL`. If the variable is not set, the test
    //! prints a message and skips.

    use sqlx::PgPool;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use uuid::Uuid;

    use super::Database;

    /// A fresh test database.
    ///
    /// The value removes the database on drop (best effort). Call [`TestDatabase::cleanup`] at the
    /// end of the test for a correct removal.
    #[derive(Debug)]
    pub struct TestDatabase {
        database: Database,
        admin: PgConnectOptions,
        name: String,
    }

    impl TestDatabase {
        /// Returns the connection pool of the test database.
        pub fn pool(&self) -> &PgPool {
            self.database.pool()
        }

        /// Returns the database handle of the test database.
        pub fn database(&self) -> &Database {
            &self.database
        }

        /// Removes the test database.
        pub async fn cleanup(&self) {
            self.pool().close().await;
            drop_database(&self.admin, &self.name).await;
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let admin = self.admin.clone();
                let name = self.name.clone();
                handle.spawn(async move {
                    drop_database(&admin, &name).await;
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
        let name = format!("restichub_test_{}", Uuid::now_v7().simple());
        sqlx::query(&format!("CREATE DATABASE \"{name}\""))
            .execute(&admin_pool)
            .await
            .ok()?;
        admin_pool.close().await;

        let options = admin.clone().database(&name);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .ok()?;
        let database = Database { pool };
        if database.migrate().await.is_err() {
            return None;
        }
        Some(TestDatabase {
            database,
            admin,
            name,
        })
    }

    async fn drop_database(admin: &PgConnectOptions, name: &str) {
        let Ok(pool) = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(admin.clone())
            .await
        else {
            return;
        };
        let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"))
            .execute(&pool)
            .await;
        pool.close().await;
    }
}
