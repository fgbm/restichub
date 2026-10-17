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
use crate::enums::{AgentStatus, ServerScriptsState};
use crate::ids::{AgentId, UserId};
use crate::tx::TenantTx;

pub const DEFAULT_PAGE_LIMIT: i64 = 50;

macro_rules! agent_columns {
    () => {
        "a.id, a.hostname, a.os, a.arch, a.agent_version, a.restic_version, a.labels, \
         a.capabilities, a.restore_roots, a.server_scripts, \
         a.script_interpreters, a.certificate_serial, a.certificate_expires_at, \
         a.status, a.last_seen_at, a.first_connected_at, a.stream_id IS NOT NULL AS stream_open, \
         a.created_at, a.revoked_at, \
         a.revoked_by_user_id, a.pending_certificate_serial, \
         a.pending_certificate_expires_at, a.last_certificate_rotation_at, \
         a.last_rotation_request_at, a.previous_certificate_serial, \
         a.superseded_certificate_used_at, a.protocol_version_major, \
         a.protocol_version_minor, d.id AS duplicate_id, d.hostname AS duplicate_hostname, \
         d.created_at AS duplicate_created_at, d.last_seen_at AS duplicate_last_seen_at"
    };
}

pub(crate) use agent_columns;

/// Joins the oldest other active host of the tenant with the same machine hash. Both records of a
/// pair get the join, so both show the warning (FR-ID-11).
macro_rules! duplicate_join {
    () => {
        "LEFT JOIN LATERAL ( \
             SELECT o.id, o.hostname, o.created_at, o.last_seen_at FROM agents o \
             WHERE a.machine_id_hash IS NOT NULL AND a.status = 'active' \
               AND o.tenant_id = a.tenant_id AND o.machine_id_hash = a.machine_id_hash \
               AND o.id <> a.id AND o.status = 'active' \
             ORDER BY o.created_at, o.id LIMIT 1) d ON true"
    };
}

pub(crate) use duplicate_join;

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
    /// Whether the host trusts scripts from the control plane, from the last `AgentHello`
    /// (FR-HK-08).
    pub server_scripts: ServerScriptsState,
    /// The interpreters the agent found, as wire names such as `bash` (FR-HK-08).
    pub script_interpreters: Vec<String>,
    pub certificate_serial: Vec<u8>,
    pub certificate_expires_at: OffsetDateTime,
    pub status: AgentStatus,
    pub last_seen_at: Option<OffsetDateTime>,
    /// The time of the first stream open. `None` while the host waits for its first connection.
    pub first_connected_at: Option<OffsetDateTime>,
    /// A control plane instance holds a stream of the agent that passed the hello.
    pub stream_open: bool,
    pub created_at: OffsetDateTime,
    pub revoked_at: Option<OffsetDateTime>,
    pub revoked_by_user_id: Option<UserId>,
    /// A rotated certificate that the agent has not confirmed yet. The old one stays valid
    /// (ADR-0026).
    pub pending_certificate_serial: Option<Vec<u8>>,
    pub pending_certificate_expires_at: Option<OffsetDateTime>,
    pub last_certificate_rotation_at: Option<OffsetDateTime>,
    /// The time of the last rotation request, answered or refused. The control plane refuses the
    /// next request until the cooldown has passed (FR-ID-03).
    pub last_rotation_request_at: Option<OffsetDateTime>,
    pub previous_certificate_serial: Option<Vec<u8>>,
    /// The last time a connection presented [`Self::previous_certificate_serial`] (T17).
    pub superseded_certificate_used_at: Option<OffsetDateTime>,
    /// The protocol version of the last `AgentHello` (FR-PR-09).
    pub protocol_version_major: Option<u32>,
    pub protocol_version_minor: Option<u32>,
    /// Another active host of the tenant that reported the same machine identifier.
    pub possible_duplicate_of: Option<DuplicateOf>,
}

/// Matched by the machine identifier hash within the tenant.
#[derive(Debug, Clone)]
pub struct DuplicateOf {
    pub id: AgentId,
    pub hostname: String,
    pub created_at: OffsetDateTime,
    pub last_seen_at: Option<OffsetDateTime>,
}

/// The facts of an `AgentHello` that the registry keeps (FR-PR-09, FR-RS-03).
#[derive(Debug, Clone)]
pub struct HelloFacts<'a> {
    pub capabilities: &'a [String],
    pub restore_roots: &'a [String],
    pub agent_version: &'a str,
    pub restic_version: &'a str,
    pub protocol_version_major: u32,
    pub protocol_version_minor: u32,
    pub server_scripts: ServerScriptsState,
    pub script_interpreters: &'a [String],
}

/// The keyset cursor of the agent list. It orders by `(created_at, id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentCursor {
    pub created_at: OffsetDateTime,
    pub id: AgentId,
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

/// Reads one agent of the tenant. An agent of another tenant is absent (AC-TN-01).
pub async fn get(tx: &mut TenantTx<'_>, id: AgentId) -> Result<Option<Agent>, DbError> {
    let row = sqlx::query_as::<_, AgentRow>(concat!(
        "SELECT ",
        agent_columns!(),
        " FROM agents a ",
        duplicate_join!(),
        " WHERE a.id = $1"
    ))
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(Agent::from))
}

/// Reads one agent of the tenant and locks its row until the end of the transaction, so a
/// rotation, a revoke and a confirmation of one agent run one after the other (FR-ID-03).
pub async fn get_for_update(tx: &mut TenantTx<'_>, id: AgentId) -> Result<Option<Agent>, DbError> {
    let row = sqlx::query_as::<_, AgentRow>(concat!(
        "SELECT ",
        agent_columns!(),
        " FROM agents a ",
        duplicate_join!(),
        " WHERE a.id = $1 FOR UPDATE OF a"
    ))
    .bind(id)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(row.map(Agent::from))
}

/// Keeps a rotated certificate as pending. A later request replaces an earlier pending one
/// (ADR-0026).
pub async fn set_pending_certificate(
    tx: &mut TenantTx<'_>,
    id: AgentId,
    serial: &[u8],
    expires_at: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE agents SET pending_certificate_serial = $2, pending_certificate_expires_at = $3 \
         WHERE id = $1 AND status = 'active'",
    )
    .bind(id)
    .bind(serial)
    .bind(expires_at)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Records a rotation request. The control plane refuses the next request until the cooldown has
/// passed, whatever the stream or the instance (FR-ID-03).
pub async fn arm_rotation_cooldown(
    tx: &mut TenantTx<'_>,
    id: AgentId,
    now: OffsetDateTime,
) -> Result<(), DbError> {
    sqlx::query("UPDATE agents SET last_rotation_request_at = $2 WHERE id = $1")
        .bind(id)
        .bind(now)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(())
}

/// Marks a connection that presented the serial the last rotation replaced.
///
/// `true` when the mark is new: the serial matches the previous one and the last mark is older
/// than `quiet_since` (or absent), so the caller writes one audit event for each quiet period.
pub async fn mark_superseded_certificate_used(
    tx: &mut TenantTx<'_>,
    id: AgentId,
    serial: &[u8],
    now: OffsetDateTime,
    quiet_since: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE agents SET superseded_certificate_used_at = $3 \
         WHERE id = $1 AND status = 'active' \
           AND encode(ltrim(previous_certificate_serial, '\\x00'::bytea), 'hex') = $2 \
           AND (superseded_certificate_used_at IS NULL OR superseded_certificate_used_at < $4)",
    )
    .bind(id)
    .bind(hex_serial(serial))
    .bind(now)
    .bind(quiet_since)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Makes the pending certificate the enrolled one and refuses the old serial from now on
/// (ADR-0026).
///
/// `serial` must equal the pending serial, so a confirmation of another certificate changes
/// nothing. `true` when the row changed.
pub async fn promote_pending_certificate(
    tx: &mut TenantTx<'_>,
    id: AgentId,
    serial: &[u8],
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE agents SET previous_certificate_serial = certificate_serial, \
                certificate_serial = pending_certificate_serial, \
                certificate_expires_at = pending_certificate_expires_at, \
                pending_certificate_serial = NULL, pending_certificate_expires_at = NULL, \
                last_certificate_rotation_at = $3 \
         WHERE id = $1 AND status = 'active' \
           AND encode(ltrim(pending_certificate_serial, '\\x00'::bytea), 'hex') = $2 \
           AND pending_certificate_expires_at > $3",
    )
    .bind(id)
    .bind(hex_serial(serial))
    .bind(now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// The identifiers of the listed agents whose open stream the registry no longer allows: the row
/// is revoked, or the serial of the stream is neither the enrolled nor the pending one
/// (FR-ID-04). The serials are lower-case hexadecimal, one for each identifier.
pub async fn disallowed_streams(
    tx: &mut TenantTx<'_>,
    ids: &[Uuid],
    serials_hex: &[String],
) -> Result<Vec<Uuid>, DbError> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT u.id FROM unnest($1::uuid[], $2::text[]) AS u(id, serial) \
         LEFT JOIN agents AS a ON a.id = u.id \
         WHERE a.id IS NULL OR a.status <> 'active' \
            OR (encode(ltrim(a.certificate_serial, '\\x00'::bytea), 'hex') <> u.serial \
                AND encode(ltrim(coalesce(a.pending_certificate_serial, ''::bytea), '\\x00'::bytea), 'hex') <> u.serial)",
    )
    .bind(ids)
    .bind(serials_hex)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

fn hex_serial(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let start = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len());
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in &bytes[start..] {
        let _ = write!(text, "{byte:02x}");
    }
    text
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
    pub now: OffsetDateTime,
}

/// Replaces the certificate and the host facts of an active agent and keeps its identifier, labels
/// and hostname (FR-ID-01).
///
/// The row waits for a connection again, and its silence counts from `now`, so the alert for an
/// unreachable host does not open before the first connection (FR-ID-10). The hash is written as
/// the agent reports it, also when the agent reports none. The old serial no longer matches the row, so the stream
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
                machine_id_hash = $8, \
                last_seen_at = $9, first_connected_at = NULL, \
                pending_certificate_serial = NULL, pending_certificate_expires_at = NULL, \
                previous_certificate_serial = NULL, superseded_certificate_used_at = NULL \
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
    .bind(facts.now)
    .execute(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// What [`replace`] moved from the old record to the new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replaced {
    pub plans: u64,
    pub repositories: u64,
    pub queued_jobs: u64,
}

/// How a replacement ended: done, or refused for a reason the caller maps to a status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceOutcome {
    Replaced(Replaced),
    /// The pair does not qualify: a record is missing or revoked, the hashes differ, or the first
    /// record is not newer than the second.
    NotPair,
    /// The old record had a sign of life inside the online window.
    OldRecentlyOnline,
}

/// Moves the plans, the maintenance host references, the queued jobs and the labels of the old
/// agent to the new one, then revokes the old agent (FR-ID-11).
///
/// The caller runs it in one transaction. The pair qualifies when both rows are active, differ,
/// carry the same machine hash, and the new row was created after the old row. The old row must have
/// been silent for `online_window`. The checks live in the statement, so a request cannot move data
/// between unrelated hosts or in the wrong direction.
pub async fn replace(
    tx: &mut TenantTx<'_>,
    new: AgentId,
    old: AgentId,
    actor: UserId,
    now: OffsetDateTime,
    online_window: time::Duration,
) -> Result<ReplaceOutcome, DbError> {
    let paired: Option<(Option<OffsetDateTime>,)> = sqlx::query_as(
        "SELECT o.last_seen_at FROM agents n JOIN agents o ON o.tenant_id = n.tenant_id \
         WHERE n.id = $1 AND o.id = $2 AND n.id <> o.id \
           AND n.status = 'active' AND o.status = 'active' \
           AND n.machine_id_hash IS NOT NULL AND n.machine_id_hash = o.machine_id_hash \
           AND n.created_at > o.created_at \
         FOR UPDATE OF n, o",
    )
    .bind(new)
    .bind(old)
    .fetch_optional(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let Some((old_last_seen,)) = paired else {
        return Ok(ReplaceOutcome::NotPair);
    };
    if old_last_seen.is_some_and(|seen| seen > now - online_window) {
        return Ok(ReplaceOutcome::OldRecentlyOnline);
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
    let queued_jobs = sqlx::query(
        "UPDATE jobs SET agent_id = $1, updated_at = now() \
         WHERE agent_id = $2 AND status = 'queued'",
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
    Ok(ReplaceOutcome::Replaced(Replaced {
        plans,
        repositories,
        queued_jobs,
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

/// Records that a stream of an active agent passed the hello. A newer stream replaces the value,
/// so the close of an older stream clears nothing (FR-ID-08).
pub async fn set_stream_open(
    tx: &mut TenantTx<'_>,
    id: AgentId,
    stream: Uuid,
) -> Result<bool, DbError> {
    let result =
        sqlx::query("UPDATE agents SET stream_id = $2 WHERE id = $1 AND status = 'active'")
            .bind(id)
            .bind(stream)
            .execute(tx.as_conn())
            .await
            .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Clears the stream of an agent, unless another stream replaced it.
pub async fn set_stream_closed(
    tx: &mut TenantTx<'_>,
    id: AgentId,
    stream: Uuid,
) -> Result<bool, DbError> {
    let result = sqlx::query("UPDATE agents SET stream_id = NULL WHERE id = $1 AND stream_id = $2")
        .bind(id)
        .bind(stream)
        .execute(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(result.rows_affected() == 1)
}

/// Stores the facts that the agent announced in its `AgentHello` (FR-PR-09, FR-RS-03).
pub async fn set_hello_facts(
    tx: &mut TenantTx<'_>,
    id: AgentId,
    facts: &HelloFacts<'_>,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE agents SET capabilities = $2, restore_roots = $3, \
                agent_version = COALESCE(NULLIF($4, ''), agent_version), \
                restic_version = COALESCE(NULLIF($5, ''), restic_version), \
                protocol_version_major = $6, protocol_version_minor = $7, \
                server_scripts = $8, script_interpreters = $9 \
         WHERE id = $1 AND (capabilities <> $2 OR restore_roots <> $3 \
                OR agent_version <> COALESCE(NULLIF($4, ''), agent_version) \
                OR restic_version <> COALESCE(NULLIF($5, ''), restic_version) \
                OR protocol_version_major IS DISTINCT FROM $6 \
                OR protocol_version_minor IS DISTINCT FROM $7 \
                OR server_scripts <> $8 OR script_interpreters <> $9)",
    )
    .bind(id)
    .bind(facts.capabilities)
    .bind(facts.restore_roots)
    .bind(facts.agent_version)
    .bind(facts.restic_version)
    .bind(i32::try_from(facts.protocol_version_major).unwrap_or(i32::MAX))
    .bind(i32::try_from(facts.protocol_version_minor).unwrap_or(i32::MAX))
    .bind(facts.server_scripts.as_str())
    .bind(facts.script_interpreters)
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
/// call on a revoked agent returns `false` and changes nothing (idempotent). A pending rotated
/// certificate is dropped with the revoke. The stream layer closes the open stream.
pub async fn revoke(
    tx: &mut TenantTx<'_>,
    id: AgentId,
    actor: UserId,
    now: OffsetDateTime,
) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE agents SET status = 'revoked', revoked_at = $2, revoked_by_user_id = $3, \
                pending_certificate_serial = NULL, pending_certificate_expires_at = NULL \
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
pub(crate) struct AgentRow {
    id: Uuid,
    hostname: String,
    os: String,
    arch: String,
    agent_version: String,
    restic_version: String,
    labels: Vec<String>,
    capabilities: Vec<String>,
    restore_roots: Vec<String>,
    server_scripts: String,
    script_interpreters: Vec<String>,
    certificate_serial: Vec<u8>,
    certificate_expires_at: OffsetDateTime,
    status: AgentStatus,
    last_seen_at: Option<OffsetDateTime>,
    first_connected_at: Option<OffsetDateTime>,
    stream_open: bool,
    created_at: OffsetDateTime,
    revoked_at: Option<OffsetDateTime>,
    revoked_by_user_id: Option<Uuid>,
    pending_certificate_serial: Option<Vec<u8>>,
    pending_certificate_expires_at: Option<OffsetDateTime>,
    last_certificate_rotation_at: Option<OffsetDateTime>,
    last_rotation_request_at: Option<OffsetDateTime>,
    previous_certificate_serial: Option<Vec<u8>>,
    superseded_certificate_used_at: Option<OffsetDateTime>,
    protocol_version_major: Option<i32>,
    protocol_version_minor: Option<i32>,
    duplicate_id: Option<Uuid>,
    duplicate_hostname: Option<String>,
    duplicate_created_at: Option<OffsetDateTime>,
    duplicate_last_seen_at: Option<OffsetDateTime>,
}

impl From<AgentRow> for Agent {
    fn from(row: AgentRow) -> Self {
        let possible_duplicate_of = match (
            row.duplicate_id,
            row.duplicate_hostname,
            row.duplicate_created_at,
        ) {
            (Some(id), Some(hostname), Some(created_at)) => Some(DuplicateOf {
                id: AgentId::from_uuid(id),
                hostname,
                created_at,
                last_seen_at: row.duplicate_last_seen_at,
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
            server_scripts: ServerScriptsState::parse(&row.server_scripts)
                .unwrap_or(ServerScriptsState::Unknown),
            script_interpreters: row.script_interpreters,
            certificate_serial: row.certificate_serial,
            certificate_expires_at: row.certificate_expires_at,
            status: row.status,
            last_seen_at: row.last_seen_at,
            first_connected_at: row.first_connected_at,
            stream_open: row.stream_open,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
            revoked_by_user_id: row.revoked_by_user_id.map(UserId::from_uuid),
            pending_certificate_serial: row.pending_certificate_serial,
            pending_certificate_expires_at: row.pending_certificate_expires_at,
            last_certificate_rotation_at: row.last_certificate_rotation_at,
            last_rotation_request_at: row.last_rotation_request_at,
            previous_certificate_serial: row.previous_certificate_serial,
            superseded_certificate_used_at: row.superseded_certificate_used_at,
            protocol_version_major: row
                .protocol_version_major
                .and_then(|v| u32::try_from(v).ok()),
            protocol_version_minor: row
                .protocol_version_minor
                .and_then(|v| u32::try_from(v).ok()),
            possible_duplicate_of,
        }
    }
}
