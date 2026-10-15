//! The enrollment tokens (ADR-0026, ADR-0032, FR-ID-01, FR-ID-05, FR-TN-03).
//!
//! A token belongs to one tenant and carries optional agent labels. The database keeps only the
//! SHA-256 hash and a short display prefix, in the manner of the API tokens of ADR-0023. The value
//! appears once, at the create.
//!
//! A token is single use and it expires: [`consume`] marks the row used and refuses a used, an
//! expired and a revoked token. A revoke is soft: [`revoke`] sets `revoked_at` and keeps the row.
//! The enrollment exchange (#48) calls [`consume`]. The current lookup runs in the tenant
//! transaction of the caller; #48 adds the cross-tenant lookup for the enrollment endpoint.

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::ids::{AgentId, EnrollmentTokenId, TenantId, UserId};
use crate::tx::{EnrollmentTx, TenantTx};

/// Default time to live of a token (ADR-0032: one hour).
pub const DEFAULT_TTL_SECONDS: i64 = 60 * 60;

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
    /// Binds the token to an existing host: the enrollment keeps that agent identifier.
    pub replaces_agent_id: Option<AgentId>,
}

/// The metadata of one token. It holds no token value.
#[derive(Debug, Clone)]
pub struct EnrollmentToken {
    pub id: EnrollmentTokenId,
    pub display_prefix: String,
    pub labels: Vec<String>,
    pub expires_at: OffsetDateTime,
    pub used_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
    pub revoked_by_user_id: Option<UserId>,
    pub created_by: UserId,
    pub created_at: OffsetDateTime,
    pub replaces_agent_id: Option<AgentId>,
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
    /// An Admin revoked the token before the exchange.
    Revoked,
    /// No token holds this hash (an unknown or a foreign token).
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
         (id, tenant_id, token_hash, display_prefix, labels, expires_at, created_by_user_id, \
          replaces_agent_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(id)
    .bind(tx.tenant())
    .bind(new.token_hash)
    .bind(new.display_prefix)
    .bind(new.labels)
    .bind(new.expires_at)
    .bind(new.created_by)
    .bind(new.replaces_agent_id)
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
        "SELECT id, display_prefix, labels, expires_at, used_at, revoked_at, revoked_by_user_id, \
                created_by_user_id, created_at, replaces_agent_id \
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
        "SELECT id, display_prefix, labels, expires_at, used_at, revoked_at, revoked_by_user_id, \
                created_by_user_id, created_at, replaces_agent_id \
         FROM enrollment_tokens WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(EnrollmentToken::from))
}

/// Revokes an unused token of the tenant (a soft change).
///
/// Sets `revoked_at` and `revoked_by_user_id` in one conditional update. `true` when the row
/// changed. A used, an already revoked or an expired token changes nothing (`false`); the caller
/// reads the metadata to tell the cases apart.
pub async fn revoke(
    tx: &mut TenantTx<'_>,
    id: EnrollmentTokenId,
    actor: UserId,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE enrollment_tokens SET revoked_at = $2, revoked_by_user_id = $3 \
         WHERE id = $1 AND used_at IS NULL AND revoked_at IS NULL AND expires_at > $2",
    )
    .bind(id)
    .bind(now)
    .bind(actor)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Spends a token (FR-ID-01, AC-ID-01).
///
/// Marks the row used and returns [`ConsumeOutcome::Consumed`]. An expired token gives
/// [`ConsumeOutcome::Expired`], a used token [`ConsumeOutcome::AlreadyUsed`] and a revoked token
/// [`ConsumeOutcome::Revoked`]. A concurrent exchange or revoke loses the guard and the function
/// re-reads the row to answer the outcome.
pub async fn consume(
    tx: &mut TenantTx<'_>,
    token_hash: &[u8],
    now: OffsetDateTime,
) -> Result<ConsumeOutcome, DbError> {
    let existing: Option<EnrollmentTokenState> = sqlx::query_as(
        "SELECT used_at, revoked_at, expires_at FROM enrollment_tokens WHERE token_hash = $1",
    )
    .bind(token_hash)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let Some(state) = existing else {
        return Ok(ConsumeOutcome::Unknown);
    };
    if let Some(outcome) = state.refusal(now) {
        return Ok(outcome);
    }
    let result = sqlx::query(
        "UPDATE enrollment_tokens SET used_at = $2 \
         WHERE token_hash = $1 AND used_at IS NULL AND revoked_at IS NULL AND expires_at > $2",
    )
    .bind(token_hash)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    if result.rows_affected() == 1 {
        return Ok(ConsumeOutcome::Consumed);
    }
    // A concurrent exchange or revoke changed the row since the read. Re-read and classify.
    let after: Option<EnrollmentTokenState> = sqlx::query_as(
        "SELECT used_at, revoked_at, expires_at FROM enrollment_tokens WHERE token_hash = $1",
    )
    .bind(token_hash)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(after
        .and_then(|state| state.refusal(now))
        .unwrap_or(ConsumeOutcome::AlreadyUsed))
}

/// Why the enrollment exchange refused a token that exists (FR-ID-01, AC-IN-02).
///
/// The value goes to the audit record of the tenant, never to the client: the client sees one
/// generic refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollmentRefusal {
    Used,
    Expired,
    Revoked,
}

impl EnrollmentRefusal {
    /// The stable string of the refusal. It is the `outcome` of the database function.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Used => "used",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
        }
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "used" => Some(Self::Used),
            "expired" => Some(Self::Expired),
            "revoked" => Some(Self::Revoked),
            _ => None,
        }
    }
}

/// The result of the cross-tenant token lookup of the enrollment exchange (FR-ID-01, FR-TN-03).
#[derive(Debug, Clone)]
pub enum EnrollmentConsume {
    /// The token is valid. The row is marked used and the transaction is now scoped to `tenant_id`.
    Consumed {
        tenant_id: TenantId,
        token_id: EnrollmentTokenId,
        labels: Vec<String>,
        replaces_agent_id: Option<AgentId>,
    },
    /// The token exists but is refused. The transaction is scoped to `tenant_id` for the audit.
    Refused {
        tenant_id: TenantId,
        token_id: EnrollmentTokenId,
        reason: EnrollmentRefusal,
    },
    /// No token holds this hash.
    Unknown,
}

/// Consumes a one-time token by its hash across the installation (FR-ID-01, FR-TN-03, AC-ID-01).
///
/// The token belongs to one tenant, which the enrollment endpoint does not know before the lookup.
/// The `SECURITY DEFINER` function `enroll_consume_token` of migrations 0025 and 0042 does the lookup and the
/// atomic consume under one row lock. On a hit the function returns the tenant, so the caller
/// continues in a tenant transaction and creates the agent row in the same transaction; a consumed
/// token therefore never stays without an agent. The caller records the refusal reason in the
/// tenant of the refused token. The function reads the time from the database server, so a clock of
/// a control plane instance never moves the expiry boundary.
pub async fn consume_for_enrollment(
    tx: &mut EnrollmentTx<'_>,
    token_hash: &[u8],
) -> Result<EnrollmentConsume, DbError> {
    let row: Option<(String, Uuid, Uuid, Vec<String>, Option<Uuid>)> = sqlx::query_as(
        "SELECT outcome, tenant_id, token_id, labels, replaces_agent_id \
         FROM enroll_consume_token($1)",
    )
    .bind(token_hash)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let Some((outcome, tenant_id, token_id, labels, replaces_agent_id)) = row else {
        return Ok(EnrollmentConsume::Unknown);
    };
    let tenant_id = TenantId::from_uuid(tenant_id);
    let token_id = EnrollmentTokenId::from_uuid(token_id);
    if outcome == "consumed" {
        tx.enter_tenant(tenant_id).await?;
        return Ok(EnrollmentConsume::Consumed {
            tenant_id,
            token_id,
            labels,
            replaces_agent_id: replaces_agent_id.map(AgentId::from_uuid),
        });
    }
    let Some(reason) = EnrollmentRefusal::parse(&outcome) else {
        return Err(DbError::UnknownEnrollmentOutcome(outcome));
    };
    tx.enter_tenant(tenant_id).await?;
    Ok(EnrollmentConsume::Refused {
        tenant_id,
        token_id,
        reason,
    })
}

/// The fields that decide whether a token is still spendable.
type EnrollmentTokenState = (
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
    OffsetDateTime,
);

trait Refusal {
    /// `None` when the token is still spendable.
    fn refusal(&self, now: OffsetDateTime) -> Option<ConsumeOutcome>;
}

impl Refusal for EnrollmentTokenState {
    fn refusal(&self, now: OffsetDateTime) -> Option<ConsumeOutcome> {
        let (used_at, revoked_at, expires_at) = self;
        if used_at.is_some() {
            Some(ConsumeOutcome::AlreadyUsed)
        } else if revoked_at.is_some() {
            Some(ConsumeOutcome::Revoked)
        } else if *expires_at <= now {
            Some(ConsumeOutcome::Expired)
        } else {
            None
        }
    }
}

type EnrollmentTokenRow = (
    Uuid,
    String,
    Vec<String>,
    OffsetDateTime,
    Option<OffsetDateTime>,
    Option<OffsetDateTime>,
    Option<Uuid>,
    Uuid,
    OffsetDateTime,
    Option<Uuid>,
);

impl From<EnrollmentTokenRow> for EnrollmentToken {
    fn from(row: EnrollmentTokenRow) -> Self {
        let (
            id,
            display_prefix,
            labels,
            expires_at,
            used_at,
            revoked_at,
            revoked_by_user_id,
            created_by,
            created_at,
            replaces_agent_id,
        ) = row;
        Self {
            id: EnrollmentTokenId::from_uuid(id),
            display_prefix,
            labels,
            expires_at,
            used_at,
            revoked_at,
            revoked_by_user_id: revoked_by_user_id.map(UserId::from_uuid),
            created_by: UserId::from_uuid(created_by),
            created_at,
            replaces_agent_id: replaces_agent_id.map(AgentId::from_uuid),
        }
    }
}
