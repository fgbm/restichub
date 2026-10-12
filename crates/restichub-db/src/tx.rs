//! The tenant-scoped and platform-scoped transactions (ADR-0023, section Tenant isolation).
//!
//! A handler receives only a [`TenantTx`] or a [`PlatformTx`]. A query outside a scoped
//! transaction is therefore a compile-time impossibility in the API layer.
//!
//! Each transaction sets the runtime role `restichub_app`. The role is not the table owner and has
//! no `BYPASSRLS`, so Row-Level Security applies to every query.
//!
//! The connection is crate-internal ([`TenantTx::as_conn`] and [`PlatformTx::as_conn`] are
//! `pub(crate)`). Only the repository functions of this crate run SQL with bind parameters. An API
//! crate calls typed repository functions, never raw SQL (H1).

use sqlx::{PgConnection, PgPool, Postgres, Transaction};

use crate::DbError;
use crate::ids::TenantId;

/// A transaction for one tenant.
///
/// The transaction holds the tenant id in `restichub.tenant_id`. Row-Level Security shows only the
/// rows of this tenant.
pub struct TenantTx<'a> {
    tx: Transaction<'a, Postgres>,
    tenant: TenantId,
}

impl<'a> TenantTx<'a> {
    /// Begins a transaction for one tenant.
    pub async fn begin(pool: &'a PgPool, tenant: TenantId) -> Result<Self, DbError> {
        let mut tx = pool.begin().await.map_err(DbError::Query)?;
        set_role(&mut tx).await?;
        set_config(&mut tx, "restichub.tenant_id", &tenant.to_string()).await?;
        // Clear the platform scope. A session-level value never survives (M2).
        set_config(&mut tx, "restichub.scope", "").await?;
        Ok(Self { tx, tenant })
    }

    /// Returns the tenant of the transaction.
    pub fn tenant(&self) -> TenantId {
        self.tenant
    }

    /// Returns the connection for a query.
    ///
    /// The method is crate-internal. Only the repository functions of this crate use it. The
    /// transaction is the only way to get a connection, so a query always runs with the tenant
    /// scope.
    pub(crate) fn as_conn(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    /// Commits the transaction.
    pub async fn commit(self) -> Result<(), DbError> {
        self.tx.commit().await.map_err(DbError::Query)
    }
}

/// A transaction for a platform operation.
///
/// The transaction sets `restichub.scope` to `platform`. Only the `tenants` table and the platform
/// audit path accept this scope (ADR-0023, section Tenant isolation).
pub struct PlatformTx<'a> {
    tx: Transaction<'a, Postgres>,
    created_tenant: Option<TenantId>,
}

impl<'a> PlatformTx<'a> {
    /// Begins a platform transaction.
    pub async fn begin(pool: &'a PgPool) -> Result<Self, DbError> {
        let mut tx = pool.begin().await.map_err(DbError::Query)?;
        set_role(&mut tx).await?;
        set_config(&mut tx, "restichub.scope", "platform").await?;
        // Clear the tenant id. A session-level value never survives (M2).
        set_config(&mut tx, "restichub.tenant_id", "").await?;
        Ok(Self {
            tx,
            created_tenant: None,
        })
    }

    /// Records the tenant that this transaction created (N2).
    ///
    /// Only [`crate::tenants::create`] and [`crate::tenants::create_with_bootstrap`] call this
    /// method.
    pub(crate) fn mark_created(&mut self, tenant: TenantId) {
        self.created_tenant = Some(tenant);
    }

    /// Selects the tenant of the next writes in this transaction (N2).
    ///
    /// The function keeps the platform scope and sets `restichub.tenant_id`. The tenant must be the
    /// one that this transaction created. Otherwise the function returns an error. A tenant
    /// creation and its key row, its first user and its audit record then commit in one
    /// transaction (D3).
    pub(crate) async fn enter_tenant(&mut self, tenant: TenantId) -> Result<(), DbError> {
        if self.created_tenant != Some(tenant) {
            return Err(DbError::TenantNotCreated(tenant));
        }
        set_config(&mut self.tx, "restichub.tenant_id", &tenant.to_string()).await
    }

    /// Returns the connection for a query.
    ///
    /// The method is crate-internal (H1).
    pub(crate) fn as_conn(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    /// Commits the transaction.
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
