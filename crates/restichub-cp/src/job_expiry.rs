//! Persists the end of an overdue job on a path that only answers a read (FR-SC-07).
//!
//! The list, the detail and the browse reads end an overdue job before they read it, so every
//! reader shows one state. That write is auxiliary: a database that the application role may only
//! read must still answer the read, with the end of the job shown from memory.

use restichub_db::{DbError, JobId, TenantId};
use time::OffsetDateTime;

use crate::server::AppState;

/// Persists the end of an overdue job in its own transaction. `false` when the database took no
/// write, so the caller shows the end from memory.
pub async fn end_overdue(
    state: &AppState,
    tenant: TenantId,
    job: JobId,
    now: OffsetDateTime,
) -> bool {
    match persist(state, tenant, job, now).await {
        Ok(persisted) => persisted,
        Err(error) => {
            tracing::warn!(
                %error,
                %job,
                "the expiry of an overdue job reached no write, so the answer shows it from memory"
            );
            false
        }
    }
}

async fn persist(
    state: &AppState,
    tenant: TenantId,
    job: JobId,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let Some(mut tx) = state
        .db()
        .begin_tenant_or_log(tenant, "the expiry of an overdue job")
        .await
    else {
        return Ok(false);
    };
    restichub_db::jobs::mark_expired(&mut tx, job, now).await?;
    tx.commit().await?;
    Ok(true)
}
