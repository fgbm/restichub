//! The tenant-scoped and platform-scoped transactions (ADR-0023, section Tenant isolation).
//!
//! A handler receives only a [`TenantTx`] or a [`PlatformTx`]. A query outside a scoped
//! transaction is therefore a compile-time impossibility in the API layer.
//!
//! Each transaction sets the runtime role `restichub_app`. The role is not the table owner and has
//! no `BYPASSRLS`, so Row-Level Security applies to every query.
//!
//! The connection is crate-internal (`TenantTx::as_conn` and `PlatformTx::as_conn` are
//! `pub(crate)`). Only the repository functions of this crate run SQL with bind parameters. An API
//! crate calls typed repository functions, never raw SQL.

use sqlx::{PgConnection, PgPool, Postgres, Transaction};

use crate::DbError;
use crate::ids::TenantId;

/// The transaction holds the tenant id in `restichub.tenant_id`. Row-Level Security shows only the
/// rows of this tenant.
pub struct TenantTx<'a> {
    tx: Transaction<'a, Postgres>,
    tenant: TenantId,
}

impl<'a> TenantTx<'a> {
    pub async fn begin(pool: &'a PgPool, tenant: TenantId) -> Result<Self, DbError> {
        let mut tx = pool.begin().await.map_err(DbError::Query)?;
        set_role(&mut tx).await?;
        set_config(&mut tx, "restichub.tenant_id", &tenant.to_string()).await?;
        // A session-level value does not survive a pooled connection.
        set_config(&mut tx, "restichub.scope", "").await?;
        Ok(Self { tx, tenant })
    }

    /// Continues the open transaction of an [`EnrollmentTx`] under one tenant.
    pub(crate) fn from_parts(tx: Transaction<'a, Postgres>, tenant: TenantId) -> Self {
        Self { tx, tenant }
    }

    pub fn tenant(&self) -> TenantId {
        self.tenant
    }

    /// Takes a transaction-scoped advisory lock for the tenant and a scope.
    ///
    /// The lock serializes the changes that read a tenant-wide invariant and then write, for
    /// example the rule "an Admin cannot remove the last Admin of a tenant". The scope separates
    /// the purposes. PostgreSQL releases the lock at the end of the transaction.
    pub async fn lock(&mut self, scope: &str) -> Result<(), DbError> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1 || $2::text, 0))")
            .bind(scope)
            .bind(self.tenant.to_string())
            .execute(&mut *self.tx)
            .await
            .map_err(DbError::Query)?;
        Ok(())
    }

    /// The method is crate-internal. Only the repository functions of this crate use it. The
    /// transaction is the only way to get a connection, so a query always runs with the tenant
    /// scope.
    pub(crate) fn as_conn(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    /// Marks a point inside the open transaction.
    ///
    /// A caller that runs several independent units of work in one transaction uses a savepoint, so
    /// a failure of one unit does not roll back the whole transaction. The name is formatted into the
    /// statement, because `SAVEPOINT` takes no bind parameter. The `'static` bound makes a runtime
    /// string unlikely. Callers pass a literal.
    pub async fn savepoint(&mut self, name: &'static str) -> Result<(), DbError> {
        sqlx::query(sqlx::AssertSqlSafe(format!("SAVEPOINT {name}")))
            .execute(&mut *self.tx)
            .await
            .map_err(DbError::Query)?;
        Ok(())
    }

    /// Rolls the transaction back to a savepoint. The savepoint stays defined.
    pub async fn rollback_to_savepoint(&mut self, name: &'static str) -> Result<(), DbError> {
        sqlx::query(sqlx::AssertSqlSafe(format!("ROLLBACK TO SAVEPOINT {name}")))
            .execute(&mut *self.tx)
            .await
            .map_err(DbError::Query)?;
        Ok(())
    }

    /// Releases a savepoint. The work inside it stays part of the transaction.
    pub async fn release_savepoint(&mut self, name: &'static str) -> Result<(), DbError> {
        sqlx::query(sqlx::AssertSqlSafe(format!("RELEASE SAVEPOINT {name}")))
            .execute(&mut *self.tx)
            .await
            .map_err(DbError::Query)?;
        Ok(())
    }

    /// A statement of the rest of the transaction that runs longer is cancelled with SQLSTATE
    /// 57014, and the transaction no longer commits.
    pub async fn statement_timeout(&mut self, timeout: time::Duration) -> Result<(), DbError> {
        let value = format!("{}ms", timeout.whole_milliseconds().max(1));
        set_config(&mut self.tx, "statement_timeout", &value).await
    }

    pub async fn commit(self) -> Result<(), DbError> {
        self.tx.commit().await.map_err(DbError::Query)
    }
}

/// The transaction sets `restichub.scope` to `platform`. Only the `tenants` table and the platform
/// audit path accept this scope (ADR-0023, section Tenant isolation).
pub struct PlatformTx<'a> {
    tx: Transaction<'a, Postgres>,
    created_tenant: Option<TenantId>,
}

impl<'a> PlatformTx<'a> {
    pub async fn begin(pool: &'a PgPool) -> Result<Self, DbError> {
        let mut tx = pool.begin().await.map_err(DbError::Query)?;
        set_role(&mut tx).await?;
        set_config(&mut tx, "restichub.scope", "platform").await?;
        // A session-level value does not survive a pooled connection.
        set_config(&mut tx, "restichub.tenant_id", "").await?;
        Ok(Self {
            tx,
            created_tenant: None,
        })
    }

    /// Only [`crate::tenants::create`] and [`crate::tenants::create_with_bootstrap`] call this
    /// method.
    pub(crate) fn mark_created(&mut self, tenant: TenantId) {
        self.created_tenant = Some(tenant);
    }

    /// Selects the tenant of the next writes in this transaction.
    ///
    /// The function keeps the platform scope and sets `restichub.tenant_id`. The tenant must be the
    /// one that this transaction created. Otherwise the function returns an error. A tenant
    /// creation and its key row, its first user and its audit record then commit in one
    /// transaction.
    pub(crate) async fn enter_tenant(&mut self, tenant: TenantId) -> Result<(), DbError> {
        if self.created_tenant != Some(tenant) {
            return Err(DbError::TenantNotCreated(tenant));
        }
        set_config(&mut self.tx, "restichub.tenant_id", &tenant.to_string()).await
    }

    /// Crate-internal.
    pub(crate) fn as_conn(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    pub async fn commit(self) -> Result<(), DbError> {
        self.tx.commit().await.map_err(DbError::Query)
    }
}

/// The transaction of the enrollment exchange (ADR-0026, FR-ID-01, FR-TN-03).
///
/// The endpoint reads the one-time token before it knows the tenant, so the transaction starts
/// without a tenant scope. The token lookup runs through the `SECURITY DEFINER` function of
/// migration 0025, which returns the tenant. `enter_tenant` then sets the scope and
/// [`EnrollmentTx::into_tenant_tx`] continues as a [`TenantTx`]. The consume and the agent row
/// commit in one transaction, so a consumed token never stays without an agent.
pub struct EnrollmentTx<'a> {
    tx: Transaction<'a, Postgres>,
    tenant: Option<TenantId>,
}

impl<'a> EnrollmentTx<'a> {
    pub async fn begin(pool: &'a PgPool) -> Result<Self, DbError> {
        let mut tx = pool.begin().await.map_err(DbError::Query)?;
        set_role(&mut tx).await?;
        // A session-level value does not survive a pooled connection.
        set_config(&mut tx, "restichub.scope", "").await?;
        set_config(&mut tx, "restichub.tenant_id", "").await?;
        Ok(Self { tx, tenant: None })
    }

    /// Sets the tenant scope of the transaction after the token lookup.
    pub(crate) async fn enter_tenant(&mut self, tenant: TenantId) -> Result<(), DbError> {
        set_config(&mut self.tx, "restichub.tenant_id", &tenant.to_string()).await?;
        self.tenant = Some(tenant);
        Ok(())
    }

    /// Crate-internal.
    pub(crate) fn as_conn(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    /// Continues the transaction as a tenant transaction.
    ///
    /// Fails when the token lookup did not enter a tenant yet.
    pub fn into_tenant_tx(self) -> Result<TenantTx<'a>, DbError> {
        let tenant = self.tenant.ok_or(DbError::EnrollmentTenantUnset)?;
        Ok(TenantTx::from_parts(self.tx, tenant))
    }

    pub async fn commit(self) -> Result<(), DbError> {
        self.tx.commit().await.map_err(DbError::Query)
    }
}

async fn set_role(tx: &mut Transaction<'_, Postgres>) -> Result<(), DbError> {
    sqlx::query("SET LOCAL ROLE restichub_app")
        .execute(&mut **tx)
        .await
        .map_err(DbError::Query)?;
    Ok(())
}

async fn set_config(
    tx: &mut Transaction<'_, Postgres>,
    name: &str,
    value: &str,
) -> Result<(), DbError> {
    sqlx::query("SELECT set_config($1, $2, true)")
        .bind(name)
        .bind(value)
        .execute(&mut **tx)
        .await
        .map_err(DbError::Query)?;
    Ok(())
}
