//! Session operations (ADR-0023, section Sessions).
//!
//! The database keeps only the SHA-256 hash of a session token. The table has the general tenant
//! policy. The control plane looks a session up with the `SECURITY DEFINER` function
//! `auth_find_session` before it knows the tenant ([`crate::auth::find_session`]).

use std::net::IpAddr;

use time::OffsetDateTime;

use crate::DbError;
use crate::ids::{SessionId, UserId};
use crate::tx::TenantTx;

/// Skips the `last_seen_at` update below this many seconds.
pub const TOUCH_INTERVAL_SECONDS: i64 = 60;

pub async fn create(
    tx: &mut TenantTx<'_>,
    user_id: UserId,
    token_hash: &[u8],
    expires_at: OffsetDateTime,
    last_seen_at: OffsetDateTime,
    ip: Option<IpAddr>,
    user_agent: Option<&str>,
) -> Result<SessionId, DbError> {
    let id = SessionId::new();
    sqlx::query(
        "INSERT INTO sessions \
         (id, tenant_id, user_id, token_hash, created_at, last_seen_at, expires_at, ip, user_agent) \
         VALUES ($1, $2, $3, $4, $5, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(tx.tenant())
    .bind(user_id)
    .bind(token_hash)
    .bind(last_seen_at)
    .bind(expires_at)
    .bind(ip)
    .bind(user_agent)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(id)
}

/// True if the session existed.
pub async fn delete(tx: &mut TenantTx<'_>, id: SessionId) -> Result<bool, DbError> {
    let result = sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

pub async fn delete_for_user(tx: &mut TenantTx<'_>, user_id: UserId) -> Result<u64, DbError> {
    let result = sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}

/// Used by a password change, which keeps the current session and removes the others.
pub async fn delete_for_user_except(
    tx: &mut TenantTx<'_>,
    user_id: UserId,
    keep: SessionId,
) -> Result<u64, DbError> {
    let result = sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND id <> $2")
        .bind(user_id)
        .bind(keep)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}

/// Updates `last_seen_at` when the stored value is older than [`TOUCH_INTERVAL_SECONDS`].
///
/// The current time is a parameter, so a test uses the same clock as the idle check.
pub async fn touch(
    tx: &mut TenantTx<'_>,
    id: SessionId,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE sessions SET last_seen_at = $2 \
         WHERE id = $1 AND last_seen_at < $2 - make_interval(secs => $3)",
    )
    .bind(id)
    .bind(now)
    .bind(TOUCH_INTERVAL_SECONDS)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
}

/// Removes the sessions of the tenant of the transaction that are past the absolute lifetime.
///
/// The control plane maintenance task calls this function. A session that is only idle is not
/// removed here; the idle check refuses it on use.
pub async fn delete_expired(tx: &mut TenantTx<'_>, now: OffsetDateTime) -> Result<u64, DbError> {
    let result = sqlx::query("DELETE FROM sessions WHERE expires_at < $1")
        .bind(now)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected())
}
