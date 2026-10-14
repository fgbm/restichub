//! The agent registry (ADR-0026, FR-ID-04, FR-TN-01).
//!
//! Each enrolled agent has one row under the tenant policy of ADR-0023. The enrollment exchange
//! (#48) creates the row; the stream layer (#52) reads the status and the certificate serial at
//! each handshake and each stream open. A revoke sets the status to `revoked` and keeps the row.
//!
//! The identity of an agent is its row in this table: the tenant of each message never comes from a
//! message field (FR-PR-08, AC-TN-02).

use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::enums::AgentStatus;
use crate::ids::{AgentId, UserId};
use crate::tx::TenantTx;

pub const DEFAULT_PAGE_LIMIT: i64 = 50;

pub const MAX_PAGE_LIMIT: i64 = 200;

/// One registry row that the enrollment exchange creates and the stream layer reads.
#[derive(Debug, Clone)]
pub struct Agent {
    pub id: AgentId,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub agent_version: String,
    pub restic_version: String,
    pub labels: Vec<String>,
    pub certificate_serial: Vec<u8>,
    pub certificate_expires_at: OffsetDateTime,
    pub status: AgentStatus,
    pub last_seen_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    pub revoked_by_user_id: Option<UserId>,
}

/// The keyset cursor of the agent list. It orders by `(created_at, id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentCursor {
    pub created_at: OffsetDateTime,
    pub id: AgentId,
}

#[derive(Debug, Clone, Copy)]
pub struct AgentPage {
    /// Clamped to [`MAX_PAGE_LIMIT`].
    pub limit: i64,
    pub after: Option<AgentCursor>,
}

impl Default for AgentPage {
    fn default() -> Self {
        Self {
            limit: DEFAULT_PAGE_LIMIT,
            after: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentPageResult {
    pub agents: Vec<Agent>,
    pub has_more: bool,
}

/// The fields of a new registry row. The enrollment exchange (#48) fills them from the CSR and the
/// token labels.
#[derive(Debug, Clone)]
pub struct NewAgent<'a> {
    /// The certificate SAN URI carries this id, so the caller chooses it before the issue.
    pub id: AgentId,
    pub hostname: &'a str,
    pub os: &'a str,
    pub arch: &'a str,
    pub agent_version: &'a str,
    pub restic_version: &'a str,
    pub labels: &'a [String],
    pub certificate_serial: &'a [u8],
    pub certificate_expires_at: OffsetDateTime,
    pub last_seen_at: Option<OffsetDateTime>,
}

/// Creates an active registry row for the enrollment exchange (#48).
pub async fn create(tx: &mut TenantTx<'_>, new: &NewAgent<'_>) -> Result<AgentId, DbError> {
    let id = new.id;
    sqlx::query(
        "INSERT INTO agents \
         (id, tenant_id, hostname, os, arch, agent_version, restic_version, labels, \
          certificate_serial, certificate_expires_at, status, last_seen_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'active', $11)",
    )
    .bind(id)
    .bind(tx.tenant())
    .bind(new.hostname)
    .bind(new.os)
    .bind(new.arch)
    .bind(new.agent_version)
    .bind(new.restic_version)
    .bind(new.labels)
    .bind(new.certificate_serial)
    .bind(new.certificate_expires_at)
    .bind(new.last_seen_at)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(id)
}

/// Newest first, narrowed by an optional status filter.
pub async fn list(
    tx: &mut TenantTx<'_>,
    status: Option<AgentStatus>,
    page: &AgentPage,
) -> Result<AgentPageResult, DbError> {
    let limit = page.limit.clamp(1, MAX_PAGE_LIMIT);
    let (cursor_time, cursor_id) = match page.after {
        Some(cursor) => (Some(cursor.created_at), Some(cursor.id.as_uuid())),
        None => (None, None),
    };
    let mut agents: Vec<Agent> = sqlx::query_as::<_, AgentRow>(
        "SELECT id, hostname, os, arch, agent_version, restic_version, labels, \
                certificate_serial, certificate_expires_at, status, last_seen_at, created_at, \
                revoked_at, revoked_by_user_id \
         FROM agents \
         WHERE ($1::text IS NULL OR status = $1) \
           AND ($2::timestamptz IS NULL OR (created_at, id) < ($2::timestamptz, $3::uuid)) \
         ORDER BY created_at DESC, id DESC \
         LIMIT $4",
    )
    .bind(status.map(AgentStatus::as_str))
    .bind(cursor_time)
    .bind(cursor_id)
    .bind(limit + 1)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?
    .into_iter()
    .map(Agent::from)
    .collect();
    let has_more = agents.len() as i64 > limit;
    agents.truncate(limit as usize);
    Ok(AgentPageResult { agents, has_more })
}

/// Reads one agent of the tenant. An agent of another tenant is absent (AC-TN-01).
pub async fn get(tx: &mut TenantTx<'_>, id: AgentId) -> Result<Option<Agent>, DbError> {
    let row = sqlx::query_as::<_, AgentRow>(
        "SELECT id, hostname, os, arch, agent_version, restic_version, labels, \
                certificate_serial, certificate_expires_at, status, last_seen_at, created_at, \
                revoked_at, revoked_by_user_id \
         FROM agents WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(Agent::from))
}

/// Records the last activity of one agent under the registry guard (FR-ID-04, FR-ID-07).
///
/// The update runs only while the row is active, lists the certificate serial and is not expired, so
/// a revoke or an expiry in the window changes no row. `true` when the row was written; `false`
/// refuses the stream.
pub async fn touch_last_seen_guarded(
    tx: &mut TenantTx<'_>,
    id: AgentId,
    serial_hex: &str,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE agents SET last_seen_at = $3 \
         WHERE id = $1 AND status = 'active' \
           AND encode(ltrim(certificate_serial, '\\x00'::bytea), 'hex') = $2 \
           AND certificate_expires_at > $3",
    )
    .bind(id)
    .bind(serial_hex)
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Writes `last_seen_at` for a batch of agents of one tenant under the registry guard (FR-ID-07).
///
/// The serials are lower-case hexadecimal. The function returns the identifiers of the rows it
/// updated; an identifier that is absent is revoked, expired or no longer the enrolled serial, and
/// the caller closes its stream.
pub async fn touch_last_seen_batch(
    tx: &mut TenantTx<'_>,
    ids: &[Uuid],
    serials_hex: &[String],
    now: OffsetDateTime,
) -> Result<Vec<Uuid>, DbError> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "UPDATE agents AS a SET last_seen_at = $1 \
         FROM unnest($2::uuid[], $3::text[]) AS u(id, serial) \
         WHERE a.id = u.id AND a.status = 'active' \
           AND encode(ltrim(a.certificate_serial, '\\x00'::bytea), 'hex') = u.serial \
           AND a.certificate_expires_at > $1 \
         RETURNING a.id",
    )
    .bind(now)
    .bind(ids)
    .bind(serials_hex)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Renames an agent of the tenant.
///
/// `true` when the row changed. A missing agent or an unchanged name returns `false`.
pub async fn rename(tx: &mut TenantTx<'_>, id: AgentId, hostname: &str) -> Result<bool, DbError> {
    let result = sqlx::query("UPDATE agents SET hostname = $2 WHERE id = $1 AND hostname <> $2")
        .bind(id)
        .bind(hostname)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Revokes an agent of the tenant (FR-ID-04).
///
/// Sets the status to `revoked`, the time and the actor. `true` when the agent changed. A second
/// call on a revoked agent returns `false` and changes nothing (idempotent). The stream close
/// follows in #52.
pub async fn revoke(
    tx: &mut TenantTx<'_>,
    id: AgentId,
    actor: UserId,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE agents SET status = 'revoked', revoked_at = $2, revoked_by_user_id = $3 \
         WHERE id = $1 AND status = 'active'",
    )
    .bind(id)
    .bind(now)
    .bind(actor)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

type AgentRow = (
    Uuid,
    String,
    String,
    String,
    String,
    String,
    Vec<String>,
    Vec<u8>,
    OffsetDateTime,
    AgentStatus,
    Option<OffsetDateTime>,
    OffsetDateTime,
    Option<OffsetDateTime>,
    Option<Uuid>,
);

impl From<AgentRow> for Agent {
    fn from(row: AgentRow) -> Self {
        let (
            id,
            hostname,
            os,
            arch,
            agent_version,
            restic_version,
            labels,
            certificate_serial,
            certificate_expires_at,
            status,
            last_seen_at,
            created_at,
            revoked_at,
            revoked_by_user_id,
        ) = row;
        Self {
            id: AgentId::from_uuid(id),
            hostname,
            os,
            arch,
            agent_version,
            restic_version,
            labels,
            certificate_serial,
            certificate_expires_at,
            status,
            last_seen_at,
            created_at,
            revoked_at,
            revoked_by_user_id: revoked_by_user_id.map(UserId::from_uuid),
        }
    }
}
