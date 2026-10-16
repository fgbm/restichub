//! A test database with the migrations applied.
//!
//! The helper reads `RESTICHUB_TEST_DATABASE_URL`. If the variable is not set, the test prints a
//! message and skips. A setup error panics with the cause; it never skips.
//!
//! The helper uses the URL as an administrator (superuser) connection only to set up the cluster
//! roles, to create the template database and to clone it. The migrations run once per test
//! process into the template database, as the template owner login. Each test clones the template
//! and reads through a per-test runtime login.

#![allow(clippy::expect_used)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use super::{Database, MIGRATOR};

/// The advisory lock key that serializes the cluster-wide setup across test processes.
const ROLE_SETUP_LOCK: i64 = 0x7265_7374_6963_6875;

struct Template {
    name: String,
    owner_role: String,
    owner_password: String,
    admin: PgConnectOptions,
}

/// Created by the first test and reused by every test.
static TEMPLATE: tokio::sync::OnceCell<Template> = tokio::sync::OnceCell::const_new();

/// The value removes the clone database and the runtime login on drop (best effort). Call
/// [`TestDatabase::cleanup`] at the end of the test for a correct removal. The template database
/// and the template owner login stay for the next test.
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
    /// Connects as the runtime login.
    pub fn database(&self) -> &Database {
        &self.database
    }

    pub fn pool(&self) -> &PgPool {
        self.database.pool()
    }

    /// The tests use it for the owner-level checks, for example the audit trigger.
    pub fn owner_pool(&self) -> &PgPool {
        &self.owner
    }

    pub fn connect_options(&self) -> PgConnectOptions {
        self.runtime_options.clone()
    }

    pub fn owner_role(&self) -> &str {
        &self.owner_role
    }

    pub fn runtime_role(&self) -> &str {
        &self.runtime_role
    }

    /// A test uses the options to create a scratch role or a scratch database.
    pub fn admin_options(&self) -> PgConnectOptions {
        self.admin.clone()
    }

    /// A test uses the options to open a pool that points to another database.
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

    pub async fn cleanup(&self) {
        self.database.pool().close().await;
        self.owner.close().await;
        cleanup_cluster(&self.admin, &self.name, &self.runtime_role).await;
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let admin = self.admin.clone();
            let name = self.name.clone();
            let runtime_role = self.runtime_role.clone();
            handle.spawn(async move {
                cleanup_cluster(&admin, &name, &runtime_role).await;
            });
        }
    }
}

/// `None` if `RESTICHUB_TEST_DATABASE_URL` is not set. Any setup error panics with the cause.
pub async fn test_database() -> Option<TestDatabase> {
    let Ok(url) = std::env::var("RESTICHUB_TEST_DATABASE_URL") else {
        println!("skipped: RESTICHUB_TEST_DATABASE_URL is not set");
        return None;
    };
    let template = template(&url).await;

    let admin = template.admin.clone();
    let admin_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(admin.clone())
        .await
        .expect("cannot connect to the test database as the administrator");

    let suffix = Uuid::now_v7().simple().to_string();
    let runtime_role = format!("restichub_runtime_{suffix}");
    let runtime_password = Uuid::now_v7().simple().to_string();
    create_login(&admin_pool, &runtime_role, &runtime_password)
        .await
        .expect("cannot create the runtime login");
    grant_member(&admin_pool, "restichub_app", &runtime_role)
        .await
        .expect("cannot grant restichub_app to the runtime login");

    let name = format!("restichub_test_{suffix}");
    clone_database(&admin_pool, &name, &template.name, &template.owner_role)
        .await
        .expect("cannot clone the template database");
    admin_pool.close().await;

    let owner_options = admin
        .clone()
        .database(&name)
        .username(template.owner_role.as_str())
        .password(template.owner_password.as_str());
    let runtime_options = admin
        .clone()
        .database(&name)
        .username(runtime_role.as_str())
        .password(runtime_password.as_str());
    let runtime_pool = super::with_session_reset(PgPoolOptions::new().max_connections(5))
        .connect_with(runtime_options.clone())
        .await
        .expect("cannot open the runtime pool");
    let owner_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(owner_options)
        .await
        .expect("cannot open the owner pool");

    Some(TestDatabase {
        database: Database { pool: runtime_pool },
        owner: owner_pool,
        admin,
        runtime_options,
        owner_role: template.owner_role.clone(),
        runtime_role,
        name,
    })
}

async fn template(url: &str) -> &'static Template {
    TEMPLATE
        .get_or_init(|| async {
            let admin = url
                .parse::<PgConnectOptions>()
                .expect("RESTICHUB_TEST_DATABASE_URL is not a valid PostgreSQL URL");
            let admin_pool = PgPoolOptions::new()
                .max_connections(1)
                .connect_with(admin.clone())
                .await
                .expect("cannot connect to the test database as the administrator");

            // The advisory lock serializes the cluster-wide setup across test processes.
            sqlx::query("SELECT pg_advisory_lock($1)")
                .bind(ROLE_SETUP_LOCK)
                .execute(&admin_pool)
                .await
                .expect("cannot take the advisory lock");
            let template = setup_template(&admin_pool, &admin).await;
            sqlx::query("SELECT pg_advisory_unlock($1)")
                .bind(ROLE_SETUP_LOCK)
                .execute(&admin_pool)
                .await
                .expect("cannot release the advisory lock");
            admin_pool.close().await;
            template
        })
        .await
}

async fn setup_template(admin_pool: &PgPool, admin: &PgConnectOptions) -> Template {
    ensure_role(
        admin_pool,
        "restichub_app",
        "NOLOGIN NOSUPERUSER NOBYPASSRLS NOCREATEROLE NOCREATEDB",
    )
    .await
    .expect("cannot create the role restichub_app");
    ensure_role(
        admin_pool,
        "restichub_auth",
        "NOLOGIN NOSUPERUSER BYPASSRLS NOCREATEROLE NOCREATEDB",
    )
    .await
    .expect("cannot create the role restichub_auth");

    let (name, owner_role) = template_names();
    // The password is fixed for a template. Test processes that run side by side share the owner
    // login, and a new random password in one process would lock the others out.
    let owner_password = format!("{owner_role}-password");
    let role_exists = role_exists(admin_pool, &owner_role)
        .await
        .expect("cannot read pg_roles");
    if !role_exists {
        // A template from an earlier run is useless without its owner: its objects have no owner.
        if database_exists(admin_pool, &name)
            .await
            .expect("cannot read pg_database")
        {
            drop_database(admin_pool, &name).await;
        }
        create_login(admin_pool, &owner_role, &owner_password)
            .await
            .expect("cannot create the template owner login");
    } else {
        alter_password(admin_pool, &owner_role, &owner_password)
            .await
            .expect("cannot set the template owner password");
    }
    grant_member(admin_pool, "restichub_app", &owner_role)
        .await
        .expect("cannot grant restichub_app to the template owner");
    grant_member(admin_pool, "restichub_auth", &owner_role)
        .await
        .expect("cannot grant restichub_auth to the template owner");

    if !database_exists(admin_pool, &name)
        .await
        .expect("cannot read pg_database")
    {
        create_database(admin_pool, &name, &owner_role)
            .await
            .expect("cannot create the template database");
        let owner_options = admin
            .clone()
            .database(&name)
            .username(owner_role.as_str())
            .password(owner_password.as_str());
        let migrate_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(owner_options)
            .await
            .expect("cannot connect to the template database as the owner");
        MIGRATOR
            .run(&migrate_pool)
            .await
            .expect("cannot apply the migrations to the template database");
        migrate_pool.close().await;
    }

    Template {
        name,
        owner_role,
        owner_password,
        admin: admin.clone(),
    }
}

/// Returns the template database name and the template owner login name for the embedded
/// migrations.
///
/// The name comes from the migration versions and checksums. A changed migration set uses a new
/// template.
fn template_names() -> (String, String) {
    let mut hasher = DefaultHasher::new();
    for migration in MIGRATOR.iter() {
        migration.version.hash(&mut hasher);
        migration.description.hash(&mut hasher);
        migration.checksum.hash(&mut hasher);
    }
    let hash = hasher.finish();
    (
        format!("restichub_template_{hash:016x}"),
        format!("restichub_template_owner_{hash:016x}"),
    )
}

async fn role_exists(pool: &PgPool, name: &str) -> Result<bool, sqlx::Error> {
    let row: (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = $1)")
        .bind(name)
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}

async fn database_exists(pool: &PgPool, name: &str) -> Result<bool, sqlx::Error> {
    let row: (bool,) =
        sqlx::query_as("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(name)
            .fetch_one(pool)
            .await?;
    Ok(row.0)
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
    // The name and the password come from a UUID and need no bind parameter. PostgreSQL does not
    // accept a bind parameter in `CREATE ROLE`.
    let statement = format!(
        "CREATE ROLE \"{name}\" LOGIN NOSUPERUSER NOBYPASSRLS NOCREATEROLE NOCREATEDB \
         PASSWORD '{password}'"
    );
    sqlx::query(AssertSqlSafe(statement)).execute(pool).await?;
    Ok(())
}

async fn alter_password(pool: &PgPool, name: &str, password: &str) -> Result<(), sqlx::Error> {
    // PostgreSQL does not accept a bind parameter in `ALTER ROLE`.
    let statement = format!("ALTER ROLE \"{name}\" PASSWORD '{password}'");
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

async fn clone_database(
    pool: &PgPool,
    name: &str,
    template: &str,
    owner: &str,
) -> Result<(), sqlx::Error> {
    // The template must hold no connection when the clone starts.
    let statement = format!("CREATE DATABASE \"{name}\" TEMPLATE \"{template}\" OWNER \"{owner}\"");
    sqlx::query(AssertSqlSafe(statement)).execute(pool).await?;
    Ok(())
}

fn is_duplicate(error: &sqlx::Error) -> bool {
    match error.as_database_error().and_then(|error| error.code()) {
        Some(code) => code == "42710" || code == "23505",
        None => false,
    }
}

async fn drop_database(pool: &PgPool, name: &str) {
    // The name comes from a UUID and needs no bind parameter.
    let _ = sqlx::query(AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"
    )))
    .execute(pool)
    .await;
}

async fn cleanup_cluster(admin: &PgConnectOptions, name: &str, runtime_role: &str) {
    let Ok(pool) = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(admin.clone())
        .await
    else {
        return;
    };
    drop_database(&pool, name).await;
    let statement = format!("DROP ROLE IF EXISTS \"{runtime_role}\"");
    let _ = sqlx::query(AssertSqlSafe(statement)).execute(&pool).await;
    pool.close().await;
}

/// A test helper. The table name comes from a fixed list in the test only.
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
