//! The filtered, sorted and paged job list (FR-SC-09, FR-UI-01).
//!
//! The list filters, sorts and pages on the server, so a tenant with a long history reads one page
//! (ADR-0009). The browse jobs of the Restore screen (`snapshots` and `ls`) are internal and stay
//! out.

use time::OffsetDateTime;

use crate::DbError;
use crate::agent_search::search_pattern;
use crate::enums::{JobKind, JobStatus};
use crate::ids::{AgentId, JobId, PlanId, RepositoryId};
use crate::jobs::job_columns;
use crate::queue::{DetailRow, JobDetail, MAX_PAGE_LIMIT};
use crate::tx::TenantTx;

/// A list counts at most this many rows, so a long history does not make each page read all of it.
pub const COUNT_LIMIT: i64 = 10_000;

/// The rows that the list holds. An empty vector does not narrow the list.
#[derive(Debug, Clone, Default)]
pub struct JobFilter {
    pub plan_ids: Vec<PlanId>,
    pub repository_ids: Vec<RepositoryId>,
    pub agent_ids: Vec<AgentId>,
    pub kinds: Vec<JobKind>,
    pub status: Option<JobStatus>,
    pub group: Option<JobGroup>,
    /// A part of the plan, host or repository name, in any case.
    pub search: Option<String>,
    /// Both creation bounds are inclusive.
    pub created_from: Option<OffsetDateTime>,
    pub created_to: Option<OffsetDateTime>,
}

/// A status tab of the list. The groups do not overlap; a queued or sent job, a cancelled job and
/// an expired job are in none of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobGroup {
    Running,
    /// Failed or dead letter, without the cancelled jobs, as the dashboard counts them.
    Failed,
    /// Succeeded without warnings.
    Succeeded,
    SucceededWithWarnings,
}

impl JobGroup {
    pub const ALL: [Self; 4] = [
        Self::Running,
        Self::Failed,
        Self::Succeeded,
        Self::SucceededWithWarnings,
    ];

    fn condition(self) -> &'static str {
        match self {
            Self::Running => " AND j.status = 'running'",
            Self::Failed => {
                " AND j.status IN ('failed', 'dead_letter') \
                  AND j.result_code IS DISTINCT FROM 'cancelled'"
            }
            Self::Succeeded => {
                " AND j.status = 'succeeded' \
                  AND j.result_code IS DISTINCT FROM 'succeeded_with_warnings'"
            }
            Self::SucceededWithWarnings => {
                " AND j.status = 'succeeded' AND j.result_code = 'succeeded_with_warnings'"
            }
        }
    }
}

/// The column that orders the list. Each order ends with the identifier, so a page is stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JobSort {
    #[default]
    Created,
    Started,
    Finished,
    Duration,
    Status,
    Plan,
    Host,
}

/// The keyset cursor of the default order. It orders by `(created_at, id)`, newest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobCursor {
    pub created_at: OffsetDateTime,
    pub id: JobId,
}

/// Where the page starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobWindow {
    Offset(i64),
    After(JobCursor),
}

#[derive(Debug, Clone)]
pub struct JobQuery<'a> {
    pub filter: &'a JobFilter,
    pub sort: JobSort,
    pub descending: bool,
    pub window: JobWindow,
    /// Clamped to [`MAX_PAGE_LIMIT`].
    pub limit: i64,
}

#[derive(Debug, Clone)]
pub struct JobSearchResult {
    pub jobs: Vec<JobDetail>,
    pub has_more: bool,
    /// The number of jobs that match the filter on all pages, at most [`COUNT_LIMIT`].
    pub total: i64,
    /// More jobs match than [`COUNT_LIMIT`].
    pub total_capped: bool,
}

/// A number of rows that a count read only up to [`COUNT_LIMIT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CappedCount {
    pub count: i64,
    pub capped: bool,
}

impl CappedCount {
    /// `counted` is the result of a count that stops after `COUNT_LIMIT + 1` rows.
    pub fn of(counted: i64) -> Self {
        Self {
            count: counted.min(COUNT_LIMIT),
            capped: counted > COUNT_LIMIT,
        }
    }
}

// The binds are the same in each query: $1 plans, $2 repositories, $3 agents, $4 kinds, $5 status,
// $6 the first and $7 the last creation time, $8 the search pattern. The name subqueries are small
// tables of the tenant, so the search adds no join to the count.
macro_rules! filter_sql {
    () => {
        " WHERE j.kind NOT IN ('snapshots', 'ls') \
            AND (cardinality($1::uuid[]) = 0 OR j.plan_id = ANY($1)) \
            AND (cardinality($2::uuid[]) = 0 OR j.repository_id = ANY($2)) \
            AND (cardinality($3::uuid[]) = 0 OR j.agent_id = ANY($3)) \
            AND (cardinality($4::text[]) = 0 OR j.kind = ANY($4)) \
            AND ($5::text IS NULL OR j.status = $5) \
            AND ($6::timestamptz IS NULL OR j.created_at >= $6) \
            AND ($7::timestamptz IS NULL OR j.created_at <= $7) \
            AND ($8::text IS NULL \
                 OR j.plan_id IN (SELECT id FROM plans WHERE name ILIKE $8) \
                 OR j.agent_id IN (SELECT id FROM agents WHERE hostname ILIKE $8) \
                 OR j.repository_id IN (SELECT id FROM repositories WHERE name ILIKE $8))"
    };
}

macro_rules! joins_sql {
    () => {
        " FROM jobs j \
          LEFT JOIN plans p ON p.tenant_id = j.tenant_id AND p.id = j.plan_id \
          LEFT JOIN agents a ON a.tenant_id = j.tenant_id AND a.id = j.agent_id \
          LEFT JOIN repositories r ON r.tenant_id = j.tenant_id AND r.id = j.repository_id"
    };
}

fn order_sql(sort: JobSort, descending: bool) -> String {
    let dir = if descending { "DESC" } else { "ASC" };
    match sort {
        JobSort::Created => format!("j.created_at {dir}, j.id {dir}"),
        JobSort::Started => format!("j.started_at {dir} NULLS LAST, j.created_at DESC, j.id DESC"),
        JobSort::Finished => {
            format!("j.finished_at {dir} NULLS LAST, j.created_at DESC, j.id DESC")
        }
        JobSort::Duration => {
            format!("(j.finished_at - j.started_at) {dir} NULLS LAST, j.created_at DESC, j.id DESC")
        }
        JobSort::Status => format!("j.status {dir}, j.created_at DESC, j.id DESC"),
        JobSort::Plan => format!("lower(p.name) {dir} NULLS LAST, j.created_at DESC, j.id DESC"),
        JobSort::Host => {
            format!("lower(a.hostname) {dir} NULLS LAST, j.created_at DESC, j.id DESC")
        }
    }
}

struct Binds {
    plans: Vec<uuid::Uuid>,
    repositories: Vec<uuid::Uuid>,
    agents: Vec<uuid::Uuid>,
    kinds: Vec<&'static str>,
    status: Option<&'static str>,
    search: Option<String>,
}

fn binds(filter: &JobFilter) -> Binds {
    Binds {
        plans: filter.plan_ids.iter().map(|id| id.as_uuid()).collect(),
        repositories: filter
            .repository_ids
            .iter()
            .map(|id| id.as_uuid())
            .collect(),
        agents: filter.agent_ids.iter().map(|id| id.as_uuid()).collect(),
        kinds: filter.kinds.iter().map(|kind| kind.as_str()).collect(),
        status: filter.status.map(JobStatus::as_str),
        search: filter.search.as_deref().map(search_pattern),
    }
}

/// Reads one page of the jobs that match the filter, and their total.
pub async fn search(
    tx: &mut TenantTx<'_>,
    query: &JobQuery<'_>,
) -> Result<JobSearchResult, DbError> {
    let limit = query.limit.clamp(1, MAX_PAGE_LIMIT);
    let filter = query.filter;
    let bound = binds(filter);
    let (offset, after_time, after_id) = match query.window {
        JobWindow::Offset(offset) => (offset.max(0), None, None),
        JobWindow::After(cursor) => (0, Some(cursor.created_at), Some(cursor.id)),
    };
    let sql = format!(
        "SELECT {columns}, p.name AS plan_name, a.hostname AS agent_hostname, \
                r.name AS repository_name {joins} {filter}{group} \
           AND ($9::timestamptz IS NULL OR (j.created_at, j.id) < ($9, $10)) \
         ORDER BY {order} LIMIT $11 OFFSET $12",
        columns = job_columns!("j."),
        joins = joins_sql!(),
        filter = filter_sql!(),
        group = filter.group.map_or("", JobGroup::condition),
        order = order_sql(query.sort, query.descending),
    );
    let rows = sqlx::query_as::<_, DetailRow>(sqlx::AssertSqlSafe(sql))
        .bind(&bound.plans)
        .bind(&bound.repositories)
        .bind(&bound.agents)
        .bind(&bound.kinds)
        .bind(bound.status)
        .bind(filter.created_from)
        .bind(filter.created_to)
        .bind(bound.search.as_deref())
        .bind(after_time)
        .bind(after_id)
        .bind(limit + 1)
        .bind(offset)
        .fetch_all(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    let has_more = rows.len() as i64 > limit;
    let jobs = rows
        .into_iter()
        .take(limit as usize)
        .map(JobDetail::from)
        .collect();
    let total = count(tx, &bound, filter, filter.group).await?;
    Ok(JobSearchResult {
        jobs,
        has_more,
        total: total.count,
        total_capped: total.capped,
    })
}

/// Counts the jobs of each group under the filter. The status and the group of the filter do not
/// apply: a tab shows what it would hold.
pub async fn group_counts(
    tx: &mut TenantTx<'_>,
    filter: &JobFilter,
) -> Result<Vec<(JobGroup, CappedCount)>, DbError> {
    let unfiltered = JobFilter {
        status: None,
        group: None,
        ..filter.clone()
    };
    let bound = binds(&unfiltered);
    let mut counts = Vec::with_capacity(JobGroup::ALL.len());
    for group in JobGroup::ALL {
        counts.push((group, count(tx, &bound, &unfiltered, Some(group)).await?));
    }
    Ok(counts)
}

async fn count(
    tx: &mut TenantTx<'_>,
    bound: &Binds,
    filter: &JobFilter,
    group: Option<JobGroup>,
) -> Result<CappedCount, DbError> {
    let sql = format!(
        "SELECT count(*) FROM (SELECT 1 FROM jobs j {filter}{group} LIMIT $9) s",
        filter = filter_sql!(),
        group = group.map_or("", JobGroup::condition),
    );
    let counted: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .bind(&bound.plans)
        .bind(&bound.repositories)
        .bind(&bound.agents)
        .bind(&bound.kinds)
        .bind(bound.status)
        .bind(filter.created_from)
        .bind(filter.created_to)
        .bind(bound.search.as_deref())
        .bind(COUNT_LIMIT + 1)
        .fetch_one(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    Ok(CappedCount::of(counted))
}
