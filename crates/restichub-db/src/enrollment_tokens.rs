//! The enrollment tokens (ADR-0026, FR-ID-01, FR-ID-05, FR-TN-03).
//!
//! A token belongs to one tenant and carries optional agent labels. The database keeps only the
//! SHA-256 hash and a short display prefix, in the manner of the API tokens of ADR-0023. The value
//! appears once, at the create.
//!
//! A token is single use and it expires: [`consume`] marks the row used and refuses an expired or a
//! used token. The enrollment exchange (#48) calls [`consume`]. The current lookup runs in the
//! tenant transaction of the caller; #48 adds the cross-tenant lookup for the enrollment endpoint.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::ids::{EnrollmentTokenId, UserId};
use crate::tx::TenantTx;

/// Default time to live of a token.
pub const DEFAULT_TTL_SECONDS: i64 = 15 * 60;

/// Longest accepted time to live of a token.
pub const MAX_TTL_SECONDS: i64 = 24 * 60 * 60;

pub const DEFAULT_PAGE_LIMIT: i64 = 50;

pub const MAX_PAGE_LIMIT: i64 = 200;

#[derive(Debug, Clone)]
pub struct NewEnrollmentToken<'a> {
    /// SHA-256 of the token value.
    pub token_hash: &'a [u8],
    pub display_prefix: &'a str,
    pub labels: &'a [String],
    pub expires_at: OffsetDateTime,
    pub created_by: UserId,
}

/// The metadata of one token. It holds no token value.
#[derive(Debug, Clone)]
pub struct EnrollmentToken {
    pub id: EnrollmentTokenId,
    pub display_prefix: String,
    pub labels: Vec<String>,
    pub expires_at: OffsetDateTime,
    pub used_at: Option<OffsetDateTime>,
    pub created_by: UserId,
    pub created_at: OffsetDateTime,
}

/// The result of a token exchange (FR-ID-01, AC-ID-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumeOutcome {
    /// The token is valid. The row is marked used.
    Consumed,
    /// Validity window passed.
    Expired,
    /// Used before.
    AlreadyUsed,
    /// No token holds this hash (a revoked, an unknown or a foreign token).
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnrollmentTokenCursor {
    pub created_at: OffsetDateTime,
    pub id: EnrollmentTokenId,
}

#[derive(Debug, Clone, Copy)]
pub struct EnrollmentTokenPage {
    /// Clamped to [`MAX_PAGE_LIMIT`].
    pub limit: i64,
    pub after: Option<EnrollmentTokenCursor>,
}

impl Default for EnrollmentTokenPage {
    fn default() -> Self {
        Self {
            limit: DEFAULT_PAGE_LIMIT,
            after: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EnrollmentTokenPageResult {
    pub tokens: Vec<EnrollmentToken>,
    pub has_more: bool,
}

pub async fn create(
    tx: &mut TenantTx<'_>,
    new: &NewEnrollmentToken<'_>,
) -> Result<EnrollmentTokenId, DbError> {
    let id = EnrollmentTokenId::new();
    sqlx::query(
        "INSERT INTO enrollment_tokens \
         (id, tenant_id, token_hash, display_prefix, labels, expires_at, created_by_user_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(tx.tenant())
    .bind(new.token_hash)
    .bind(new.display_prefix)
    .bind(new.labels)
    .bind(new.expires_at)
    .bind(new.created_by)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(id)
}

/// Newest first. The value appears only at the create.
pub async fn list(
    tx: &mut TenantTx<'_>,
    page: &EnrollmentTokenPage,
) -> Result<EnrollmentTokenPageResult, DbError> {
    let limit = page.limit.clamp(1, MAX_PAGE_LIMIT);
    let (cursor_time, cursor_id) = match page.after {
        Some(cursor) => (Some(cursor.created_at), Some(cursor.id.as_uuid())),
        None => (None, None),
    };
    let mut tokens: Vec<EnrollmentToken> = sqlx::query_as::<_, EnrollmentTokenRow>(
        "SELECT id, display_prefix, labels, expires_at, used_at, created_by_user_id, created_at \
         FROM enrollment_tokens \
         WHERE ($1::timestamptz IS NULL OR (created_at, id) < ($1::timestamptz, $2::uuid)) \
         ORDER BY created_at DESC, id DESC \
         LIMIT $3",
    )
    .bind(cursor_time)
    .bind(cursor_id)
    .bind(limit + 1)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?
    .into_iter()
    .map(EnrollmentToken::from)
    .collect();
    let has_more = tokens.len() as i64 > limit;
    tokens.truncate(limit as usize);
    Ok(EnrollmentTokenPageResult { tokens, has_more })
}

/// Reads one token of the tenant. A token of another tenant is absent (AC-TN-01).
pub async fn get(
    tx: &mut TenantTx<'_>,
    id: EnrollmentTokenId,
) -> Result<Option<EnrollmentToken>, DbError> {
    let row = sqlx::query_as::<_, EnrollmentTokenRow>(
        "SELECT id, display_prefix, labels, expires_at, used_at, created_by_user_id, created_at \
         FROM enrollment_tokens WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(EnrollmentToken::from))
}

/// Removes an unused token of the tenant.
///
/// `true` when the row was removed. A used token is not removed (`false`); the caller reads the
/// metadata first to tell a used token from a missing one.
pub async fn revoke_unused(tx: &mut TenantTx<'_>, id: EnrollmentTokenId) -> Result<bool, DbError> {
    let result = sqlx::query("DELETE FROM enrollment_tokens WHERE id = $1 AND used_at IS NULL")
        .bind(id)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Spends a token (FR-ID-01, AC-ID-01).
///
/// Marks the row used and returns [`ConsumeOutcome::Consumed`]. An expired token gives
/// [`ConsumeOutcome::Expired`] and a used token [`ConsumeOutcome::AlreadyUsed`]. A concurrent
/// exchange loses the guard `used_at IS NULL` and gets [`ConsumeOutcome::AlreadyUsed`], so a token
/// is spent once.
pub async fn consume(
    tx: &mut TenantTx<'_>,
    token_hash: &[u8],
    now: OffsetDateTime,
) -> Result<ConsumeOutcome, DbError> {
    let existing: Option<(Option<OffsetDateTime>, OffsetDateTime)> =
        sqlx::query_as("SELECT used_at, expires_at FROM enrollment_tokens WHERE token_hash = $1")
            .bind(token_hash)
            .fetch_optional(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    match existing {
        None => Ok(ConsumeOutcome::Unknown),
        Some((Some(_), _)) => Ok(ConsumeOutcome::AlreadyUsed),
        Some((None, expires_at)) if expires_at <= now => Ok(ConsumeOutcome::Expired),
        Some((None, _)) => {
            let result = sqlx::query(
                "UPDATE enrollment_tokens SET used_at = $2 \
                 WHERE token_hash = $1 AND used_at IS NULL AND expires_at > $2",
            )
            .bind(token_hash)
            .bind(now)
            .execute(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
            if result.rows_affected() == 1 {
                Ok(ConsumeOutcome::Consumed)
            } else {
                Ok(ConsumeOutcome::AlreadyUsed)
            }
        }
    }
}

type EnrollmentTokenRow = (
    Uuid,
    String,
    Vec<String>,
    OffsetDateTime,
    Option<OffsetDateTime>,
    Uuid,
    OffsetDateTime,
);

impl From<EnrollmentTokenRow> for EnrollmentToken {
    fn from(row: EnrollmentTokenRow) -> Self {
        let (id, display_prefix, labels, expires_at, used_at, created_by, created_at) = row;
        Self {
            id: EnrollmentTokenId::from_uuid(id),
            display_prefix,
            labels,
            expires_at,
            used_at,
            created_by: UserId::from_uuid(created_by),
            created_at,
        }
    }
}
