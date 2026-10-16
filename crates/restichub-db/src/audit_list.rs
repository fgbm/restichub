//! The audit list of a tenant and of the platform (ADR-0023, section Audit log; FR-AU-01).
//!
//! The list resolves the name of the object and of the actor when it reads a page, so a renamed
//! object shows its current name and a removed object shows only its identifier. Each name comes
//! from a table under its tenant policy: a tenant list sees the names of its own tenant only, and
//! the platform list sees the tenant names and the installation accounts, not the objects inside a
//! tenant.

use std::net::IpAddr;

use serde_json::Value;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::DbError;
use crate::agent_search::search_pattern;
use crate::audit::{Actor, AuditAction};
use crate::ids::{AuditLogId, TenantId};
use crate::job_search::COUNT_LIMIT;
use crate::tx::{PlatformTx, TenantTx};

pub const DEFAULT_PAGE_LIMIT: i64 = 50;

pub const MAX_PAGE_LIMIT: i64 = 200;

// Each member function returns rows in one scope only: the tenant members in a tenant transaction,
// the installation accounts in a platform transaction. `MATERIALIZED` calls each one once for the
// page, not once for each row.
macro_rules! names_cte {
    () => {
        "WITH people AS MATERIALIZED ( \
             SELECT user_id AS id, COALESCE(NULLIF(display_name, ''), username) AS name \
             FROM auth_tenant_members() \
             UNION ALL \
             SELECT user_id, COALESCE(NULLIF(display_name, ''), username) \
             FROM auth_platform_accounts()), \
         named AS MATERIALIZED ( \
             SELECT id FROM people WHERE $9::text IS NOT NULL AND name ILIKE $9 \
             UNION ALL SELECT id FROM agents WHERE $9::text IS NOT NULL AND hostname ILIKE $9 \
             UNION ALL SELECT id FROM api_tokens WHERE $9::text IS NOT NULL AND name ILIKE $9 \
             UNION ALL SELECT id FROM notification_channels \
                       WHERE $9::text IS NOT NULL AND name ILIKE $9 \
             UNION ALL SELECT id FROM job_templates WHERE $9::text IS NOT NULL AND name ILIKE $9 \
             UNION ALL SELECT id FROM script_templates WHERE $9::text IS NOT NULL AND name ILIKE $9 \
             UNION ALL SELECT id FROM plans WHERE $9::text IS NOT NULL AND name ILIKE $9 \
             UNION ALL SELECT j.id FROM jobs j \
                       JOIN plans p ON p.tenant_id = j.tenant_id AND p.id = j.plan_id \
                       WHERE $9::text IS NOT NULL AND p.name ILIKE $9 \
             UNION ALL SELECT id FROM profiles WHERE $9::text IS NOT NULL AND name ILIKE $9 \
             UNION ALL SELECT id FROM repositories WHERE $9::text IS NOT NULL AND name ILIKE $9 \
             UNION ALL SELECT id FROM secrets WHERE $9::text IS NOT NULL AND name ILIKE $9 \
             UNION ALL SELECT id FROM tenants WHERE $9::text IS NOT NULL AND name ILIKE $9) "
    };
}

/// `$tenant` is the condition on `$8`: a plain `tenant_id = $8` keeps the
/// `(tenant_id, occurred_at)` index usable in a generic plan of the prepared statement. The search
/// matches the action code or a name; an identifier is unique across the tables, so the object
/// type needs no check.
macro_rules! filter_sql {
    ($tenant:literal) => {
        concat!(
            " WHERE ($1::timestamptz IS NULL OR a.occurred_at >= $1) \
               AND ($2::timestamptz IS NULL OR a.occurred_at <= $2) \
               AND ($3::text IS NULL OR a.action = $3) \
               AND ($4::text IS NULL OR a.actor_type = $4) \
               AND ($5::uuid IS NULL OR a.actor_id = $5) \
               AND ($6::text IS NULL OR a.object_type = $6) \
               AND ($7::uuid IS NULL OR a.object_id = $7) \
               AND ($9::text IS NULL OR a.action ILIKE $9 \
                    OR a.object_id IN (SELECT id FROM named) \
                    OR a.actor_id IN (SELECT id FROM named)) \
               AND ",
            $tenant
        )
    };
}

macro_rules! object_name_sql {
    () => {
        "CASE a.object_type \
             WHEN 'agent' THEN (SELECT hostname FROM agents WHERE id = a.object_id) \
             WHEN 'api_token' THEN (SELECT name FROM api_tokens WHERE id = a.object_id) \
             WHEN 'channel' THEN (SELECT name FROM notification_channels WHERE id = a.object_id) \
             WHEN 'job' THEN (SELECT p.name FROM jobs j \
                              JOIN plans p ON p.tenant_id = j.tenant_id AND p.id = j.plan_id \
                              WHERE j.id = a.object_id) \
             WHEN 'job_template' THEN (SELECT name FROM job_templates WHERE id = a.object_id) \
             WHEN 'plan' THEN (SELECT name FROM plans WHERE id = a.object_id) \
             WHEN 'profile' THEN (SELECT name FROM profiles WHERE id = a.object_id) \
             WHEN 'repository' THEN (SELECT name FROM repositories WHERE id = a.object_id) \
             WHEN 'script_template' THEN (SELECT name FROM script_templates WHERE id = a.object_id) \
             WHEN 'secret' THEN (SELECT name FROM secrets WHERE id = a.object_id) \
             WHEN 'tenant' THEN (SELECT name FROM tenants WHERE id = a.object_id) \
             WHEN 'user' THEN (SELECT name FROM people WHERE id = a.object_id LIMIT 1) \
         END"
    };
}

macro_rules! actor_name_sql {
    () => {
        "CASE a.actor_type \
             WHEN 'user' THEN (SELECT name FROM people WHERE id = a.actor_id LIMIT 1) \
             WHEN 'api_token' THEN (SELECT name FROM api_tokens WHERE id = a.actor_id) \
             WHEN 'agent' THEN (SELECT hostname FROM agents WHERE id = a.actor_id) \
         END"
    };
}

macro_rules! list_sql {
    ($tenant:literal) => {
        concat!(
            names_cte!(),
            "SELECT a.id, a.tenant_id, a.occurred_at, a.actor_type, a.actor_id, a.source_ip, \
                    a.action, a.object_type, a.object_id, a.diff, ",
            object_name_sql!(),
            " AS object_name, ",
            actor_name_sql!(),
            " AS actor_name FROM audit_log a",
            filter_sql!($tenant),
            " AND ($10::timestamptz IS NULL \
                  OR (a.occurred_at, a.id) < ($10::timestamptz, $11::uuid)) \
             ORDER BY a.occurred_at DESC, a.id DESC \
             LIMIT $12 OFFSET $13",
        )
    };
}

macro_rules! count_sql {
    ($tenant:literal) => {
        concat!(
            names_cte!(),
            "SELECT count(*) FROM (SELECT 1 FROM audit_log a",
            filter_sql!($tenant),
            " LIMIT $10) s"
        )
    };
}

const OWN_TENANT: (&str, &str) = (
    list_sql!("a.tenant_id = $8"),
    count_sql!("a.tenant_id = $8"),
);
const ANY_TENANT: (&str, &str) = (
    list_sql!("($8::uuid IS NULL OR a.tenant_id = $8)"),
    count_sql!("($8::uuid IS NULL OR a.tenant_id = $8)"),
);

#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub id: AuditLogId,
    pub tenant_id: TenantId,
    pub occurred_at: OffsetDateTime,
    pub actor: Actor,
    pub source_ip: Option<IpAddr>,
    pub action: AuditAction,
    pub object_type: String,
    pub object_id: Option<Uuid>,
    pub diff: Option<Value>,
    /// The current name of the object. `None` for a removed object, an object without a name and
    /// an object that the scope of the reader does not see.
    pub object_name: Option<String>,
    /// The current name of a user, an API token or an agent actor, with the same rule.
    pub actor_name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    /// The platform list filters by it; a tenant list replaces it with its own tenant.
    pub tenant_id: Option<TenantId>,
    /// Start of the time range, inclusive.
    pub from: Option<OffsetDateTime>,
    /// End of the time range, inclusive.
    pub to: Option<OffsetDateTime>,
    pub action: Option<AuditAction>,
    pub actor: Option<Actor>,
    pub object_type: Option<String>,
    pub object_id: Option<Uuid>,
    /// A part of the action code, the object name or the actor name, in any case.
    pub search: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditCursor {
    pub occurred_at: OffsetDateTime,
    pub id: AuditLogId,
}

#[derive(Debug, Clone, Copy)]
pub struct AuditPage {
    /// Clamped to [`MAX_PAGE_LIMIT`].
    pub limit: i64,
    pub after: Option<AuditCursor>,
    /// Skipped rows of a numbered page; the caller keeps it in `0..=COUNT_LIMIT` and sends no
    /// cursor with it.
    pub offset: i64,
}

impl Default for AuditPage {
    fn default() -> Self {
        Self {
            limit: DEFAULT_PAGE_LIMIT,
            after: None,
            offset: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuditPageResult {
    pub records: Vec<AuditRecord>,
    pub has_more: bool,
    /// The number of records that match the filter on all pages, at most [`COUNT_LIMIT`].
    pub total: i64,
    /// More records match than [`COUNT_LIMIT`].
    pub total_capped: bool,
}

pub async fn list(
    tx: &mut TenantTx<'_>,
    filter: &AuditFilter,
    page: &AuditPage,
) -> Result<AuditPageResult, DbError> {
    // The two read policies are ORed, so Row-Level Security alone gives the planner no tenant
    // condition for the `(tenant_id, occurred_at)` index; the count would scan every tenant.
    let filter = AuditFilter {
        tenant_id: Some(tx.tenant()),
        ..filter.clone()
    };
    list_rows(tx.as_conn(), OWN_TENANT, &filter, page).await
}

pub async fn list_platform(
    tx: &mut PlatformTx<'_>,
    filter: &AuditFilter,
    page: &AuditPage,
) -> Result<AuditPageResult, DbError> {
    list_rows(tx.as_conn(), ANY_TENANT, filter, page).await
}

/// Lists records with a connection.
///
/// The list orders the records by time and identifier, from the newest to the oldest. It reads one
/// record more than the page holds, to report `has_more` correctly. The keyset cursor of
/// `page.after` continues the list; `page.offset` reads a numbered page. The total ignores the
/// cursor and the offset.
async fn list_rows(
    conn: &mut PgConnection,
    (list_sql, count_sql): (&'static str, &'static str),
    filter: &AuditFilter,
    page: &AuditPage,
) -> Result<AuditPageResult, DbError> {
    let limit = page.limit.clamp(1, MAX_PAGE_LIMIT);
    let action = filter.action.as_ref().map(AuditAction::as_str);
    let actor_type = filter.actor.as_ref().map(Actor::actor_type);
    let actor_id = filter.actor.as_ref().and_then(Actor::actor_id);
    let pattern = filter.search.as_deref().map(search_pattern);
    let (cursor_time, cursor_id) = match page.after {
        Some(cursor) => (Some(cursor.occurred_at), Some(cursor.id.as_uuid())),
        None => (None, None),
    };
    let mut records: Vec<AuditRecord> = sqlx::query_as::<_, Row>(list_sql)
        .bind(filter.from)
        .bind(filter.to)
        .bind(action)
        .bind(actor_type)
        .bind(actor_id)
        .bind(filter.object_type.as_deref())
        .bind(filter.object_id)
        .bind(filter.tenant_id)
        .bind(pattern.as_deref())
        .bind(cursor_time)
        .bind(cursor_id)
        .bind(limit + 1)
        .bind(page.offset.max(0))
        .fetch_all(&mut *conn)
        .await
        .map_err(DbError::Query)?
        .into_iter()
        .map(AuditRecord::from)
        .collect();
    let has_more = records.len() as i64 > limit;
    records.truncate(limit as usize);
    let counted: i64 = sqlx::query_scalar(count_sql)
        .bind(filter.from)
        .bind(filter.to)
        .bind(action)
        .bind(actor_type)
        .bind(actor_id)
        .bind(filter.object_type.as_deref())
        .bind(filter.object_id)
        .bind(filter.tenant_id)
        .bind(pattern.as_deref())
        .bind(COUNT_LIMIT + 1)
        .fetch_one(conn)
        .await
        .map_err(DbError::Query)?;
    Ok(AuditPageResult {
        records,
        has_more,
        total: counted.min(COUNT_LIMIT),
        total_capped: counted > COUNT_LIMIT,
    })
}

#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    tenant_id: Uuid,
    occurred_at: OffsetDateTime,
    actor_type: String,
    actor_id: Option<Uuid>,
    source_ip: Option<IpAddr>,
    action: String,
    object_type: String,
    object_id: Option<Uuid>,
    diff: Option<Value>,
    object_name: Option<String>,
    actor_name: Option<String>,
}

impl From<Row> for AuditRecord {
    fn from(row: Row) -> Self {
        Self {
            id: AuditLogId::from_uuid(row.id),
            tenant_id: TenantId::from_uuid(row.tenant_id),
            occurred_at: row.occurred_at,
            actor: Actor::from_parts(row.actor_type, row.actor_id),
            source_ip: row.source_ip,
            action: AuditAction::from_db(row.action),
            object_type: row.object_type,
            object_id: row.object_id,
            diff: row.diff,
            object_name: row.object_name,
            actor_name: row.actor_name,
        }
    }
}
