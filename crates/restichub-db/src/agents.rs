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

macro_rules! agent_columns {
    () => {
        "a.id, a.hostname, a.os, a.arch, a.agent_version, a.restic_version, a.labels, \
         a.capabilities, a.restore_roots, a.certificate_serial, a.certificate_expires_at, \
         a.status, a.last_seen_at, a.first_connected_at, a.created_at, a.revoked_at, \
         a.revoked_by_user_id, d.id AS duplicate_id, d.hostname AS duplicate_hostname"
    };
}

/// Joins the oldest other active host of the tenant with the same machine hash. Both records of a
/// pair get the join, so both show the warning (FR-ID-01).
macro_rules! duplicate_join {
    () => {
        "LEFT JOIN LATERAL ( \
             SELECT o.id, o.hostname FROM agents o \
             WHERE a.machine_id_hash IS NOT NULL AND a.status = 'active' \
               AND o.tenant_id = a.tenant_id AND o.machine_id_hash = a.machine_id_hash \
               AND o.id <> a.id AND o.status = 'active' \
             ORDER BY o.created_at, o.id LIMIT 1) d ON true"
    };
}

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
    /// The capabilities that the agent announced in its last `AgentHello` (FR-PR-09).
    pub capabilities: Vec<String>,
    /// The directories where the agent restores files, from its last `AgentHello` (FR-RS-03).
    pub restore_roots: Vec<String>,
    pub certificate_serial: Vec<u8>,
    pub certificate_expires_at: OffsetDateTime,
    pub status: AgentStatus,
    pub last_seen_at: Option<OffsetDateTime>,
    /// The time of the first stream open. `None` while the host waits for its first connection.
    pub first_connected_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    pub revoked_by_user_id: Option<UserId>,
    /// Another active host of the tenant that reported the same machine identifier.
    pub possible_duplicate_of: Option<DuplicateOf>,
}

/// The active host that a record may duplicate.
#[derive(Debug, Clone)]
pub struct DuplicateOf {
    pub id: AgentId,
    pub hostname: String,
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
    /// The tenant-keyed hash of the machine identifier. The identifier itself is never stored.
    pub machine_id_hash: Option<&'a [u8]>,
}

/// Creates an active registry row for the enrollment exchange (#48).
pub async fn create(tx: &mut TenantTx<'_>, new: &NewAgent<'_>) -> Result<AgentId, DbError> {
    let id = new.id;
    sqlx::query(
        "INSERT INTO agents \
         (id, tenant_id, hostname, os, arch, agent_version, restic_version, labels, \
          certificate_serial, certificate_expires_at, status, last_seen_at, machine_id_hash) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'active', $11, $12)",
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
    .bind(new.machine_id_hash)
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
        concat!(
            "SELECT ", agent_columns!(), " FROM agents a ", duplicate_join!(),
            " WHERE ($1::text IS NULL OR a.status = $1) \
               AND ($2::timestamptz IS NULL OR (a.created_at, a.id) < ($2::timestamptz, $3::uuid)) \
             ORDER BY a.created_at DESC, a.id DESC \
             LIMIT $4"
        ),
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
        concat!(
            "SELECT ", agent_columns!(), " FROM agents a ", duplicate_join!(), " WHERE a.id = $1"
        ),
    )
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(Agent::from))
}

/// The facts that a re-registration replaces on a registry row (FR-ID-01).
#[derive(Debug, Clone)]
pub struct Reenrollment<'a> {
    pub os: &'a str,
    pub arch: &'a str,
    pub agent_version: &'a str,
    pub restic_version: &'a str,
    pub certificate_serial: &'a [u8],
    pub certificate_expires_at: OffsetDateTime,
    pub machine_id_hash: Option<&'a [u8]>,
}

/// Replaces the certificate and the host facts of an active agent and keeps its identifier, labels
/// and hostname (FR-ID-01).
///
/// The row waits for a connection again. The old serial no longer matches the row, so the stream
/// layer refuses the old certificate at once. `true` when the row changed; a missing or revoked
/// agent gives `false`.
pub async fn reenroll(
    tx: &mut TenantTx<'_>,
    id: AgentId,
    facts: &Reenrollment<'_>,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE agents SET os = $2, arch = $3, agent_version = $4, restic_version = $5, \
                certificate_serial = $6, certificate_expires_at = $7, \
                machine_id_hash = COALESCE($8, machine_id_hash), \
                last_seen_at = NULL, first_connected_at = NULL \
         WHERE id = $1 AND status = 'active'",
    )
    .bind(id)
    .bind(facts.os)
    .bind(facts.arch)
    .bind(facts.agent_version)
    .bind(facts.restic_version)
    .bind(facts.certificate_serial)
    .bind(facts.certificate_expires_at)
    .bind(facts.machine_id_hash)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// What [`replace`] moved from the old record to the new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Replaced {
    pub plans: u64,
    pub repositories: u64,
}

/// Moves the plans, the maintenance host references and the labels of the old agent to the new
/// one, then revokes the old agent (FR-ID-01).
///
/// The caller runs it in one transaction. It returns `None` when the pair does not qualify: both
/// rows must be active, differ, and carry the same machine hash. The check lives in the statement,
/// so a request cannot move data between unrelated hosts.
pub async fn replace(
    tx: &mut TenantTx<'_>,
    new: AgentId,
    old: AgentId,
    actor: UserId,
    now: OffsetDateTime,
) -> Result<Option<Replaced>, DbError> {
    let paired: Option<(Uuid,)> = sqlx::query_as(
        "SELECT n.id FROM agents n JOIN agents o ON o.tenant_id = n.tenant_id \
         WHERE n.id = $1 AND o.id = $2 AND n.id <> o.id \
           AND n.status = 'active' AND o.status = 'active' \
           AND n.machine_id_hash IS NOT NULL AND n.machine_id_hash = o.machine_id_hash \
         FOR UPDATE OF n, o",
    )
    .bind(new)
    .bind(old)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    if paired.is_none() {
        return Ok(None);
    }
    let plans = sqlx::query("UPDATE plans SET agent_id = $1 WHERE agent_id = $2")
        .bind(new)
        .bind(old)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?
        .rows_affected();
    let repositories = sqlx::query(
        "UPDATE repositories SET maintenance_agent_id = $1 WHERE maintenance_agent_id = $2",
    )
    .bind(new)
    .bind(old)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?
    .rows_affected();
    sqlx::query(
        "UPDATE agents AS n SET labels = ARRAY( \
             SELECT DISTINCT label FROM unnest(n.labels || o.labels) AS label ORDER BY label) \
         FROM agents AS o WHERE n.id = $1 AND o.id = $2",
    )
    .bind(new)
    .bind(old)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    revoke(tx, old, actor, now).await?;
    Ok(Some(Replaced {
        plans,
        repositories,
    }))
}

/// Records the last activity of one agent under the registry guard (FR-ID-04, FR-ID-07).
///
/// The first write also sets `first_connected_at`, so the host leaves the waiting state.
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
        "UPDATE agents SET last_seen_at = $3, \
                first_connected_at = COALESCE(first_connected_at, $3) \
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

/// Stores the capabilities and the restore roots that the agent announced in its `AgentHello`
/// (FR-PR-09, FR-RS-03).
pub async fn set_hello_facts(
    tx: &mut TenantTx<'_>,
    id: AgentId,
    capabilities: &[String],
    restore_roots: &[String],
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE agents SET capabilities = $2, restore_roots = $3 \
         WHERE id = $1 AND (capabilities <> $2 OR restore_roots <> $3)",
    )
    .bind(id)
    .bind(capabilities)
    .bind(restore_roots)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(())
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

#[derive(sqlx::FromRow)]
struct AgentRow {
    id: Uuid,
    hostname: String,
    os: String,
    arch: String,
    agent_version: String,
    restic_version: String,
    labels: Vec<String>,
    capabilities: Vec<String>,
    restore_roots: Vec<String>,
    certificate_serial: Vec<u8>,
    certificate_expires_at: OffsetDateTime,
    status: AgentStatus,
    last_seen_at: Option<OffsetDateTime>,
    first_connected_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
    revoked_by_user_id: Option<Uuid>,
    duplicate_id: Option<Uuid>,
    duplicate_hostname: Option<String>,
}

impl From<AgentRow> for Agent {
    fn from(row: AgentRow) -> Self {
        let possible_duplicate_of = match (row.duplicate_id, row.duplicate_hostname) {
            (Some(id), Some(hostname)) => Some(DuplicateOf {
                id: AgentId::from_uuid(id),
                hostname,
            }),
            _ => None,
        };
        Self {
            id: AgentId::from_uuid(row.id),
            hostname: row.hostname,
            os: row.os,
            arch: row.arch,
            agent_version: row.agent_version,
            restic_version: row.restic_version,
            labels: row.labels,
            capabilities: row.capabilities,
            restore_roots: row.restore_roots,
            certificate_serial: row.certificate_serial,
            certificate_expires_at: row.certificate_expires_at,
            status: row.status,
            last_seen_at: row.last_seen_at,
            first_connected_at: row.first_connected_at,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
            revoked_by_user_id: row.revoked_by_user_id.map(UserId::from_uuid),
            possible_duplicate_of,
        }
    }
}
