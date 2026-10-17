//! The filtered, sorted and paged host list (FR-UI-01).
//!
//! The list filters, sorts and pages on the server, so a tenant with many hosts reads one page
//! (ADR-0009). The connection state of a host follows the same rule as the dashboard: an open
//! stream, and a `last_seen_at` after the start of the online window that the caller passes.

use time::OffsetDateTime;

use crate::DbError;
use crate::agents::{Agent, AgentCursor, AgentRow, MAX_PAGE_LIMIT, agent_columns, duplicate_join};
use crate::enums::AgentStatus;
use crate::job_search::COUNT_LIMIT;
use crate::tx::TenantTx;

/// Derived with the online rule of the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostState {
    Online,
    Offline,
    /// The host never opened a stream since its enrollment.
    Waiting,
    Revoked,
}

impl HostState {
    pub const ALL: [Self; 4] = [Self::Online, Self::Offline, Self::Waiting, Self::Revoked];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Waiting => "waiting",
            Self::Revoked => "revoked",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == value)
    }
}

/// The column that orders the list. Each order ends with the identifier, so a page is stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentSort {
    #[default]
    Created,
    Hostname,
    LastSeen,
    AgentVersion,
    Os,
}

/// The rows that the list holds. An empty vector does not narrow the list.
#[derive(Debug, Clone, Default)]
pub struct AgentFilter {
    pub status: Option<AgentStatus>,
    /// Matches a part of the host name or of a label, ignoring case.
    pub search: Option<String>,
    pub states: Vec<HostState>,
    pub os: Vec<String>,
    pub agent_versions: Vec<String>,
    /// Any one of them matches.
    pub labels: Vec<String>,
    /// `true` keeps only the hosts in the trust mode, `false` the rest.
    pub server_scripts: Option<bool>,
}

/// Where the page starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentWindow {
    Offset(i64),
    /// The keyset cursor of the default order, newest first.
    After(AgentCursor),
}

#[derive(Debug, Clone)]
pub struct AgentQuery<'a> {
    pub filter: &'a AgentFilter,
    pub sort: AgentSort,
    pub descending: bool,
    pub window: AgentWindow,
    /// Clamped to [`MAX_PAGE_LIMIT`].
    pub limit: i64,
    /// A host that the control plane saw at or after this time is online.
    pub online_since: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct AgentSearchResult {
    pub agents: Vec<Agent>,
    pub has_more: bool,
    /// The number of hosts that match the filter on all pages, at most [`COUNT_LIMIT`].
    pub total: i64,
    /// More hosts match than [`COUNT_LIMIT`].
    pub total_capped: bool,
}

/// The values that the filter bar offers, each with its number of hosts.
#[derive(Debug, Clone, Default)]
pub struct AgentFacets {
    /// Counted under the filter without its states, so each state shows what it adds.
    pub states: Vec<(HostState, i64)>,
    /// The other values count the hosts that the state filter can show, so a revoked host counts
    /// only when the filter holds no state or holds `revoked`.
    pub os: Vec<(String, i64)>,
    pub agent_versions: Vec<(String, i64)>,
    pub labels: Vec<(String, i64)>,
}

macro_rules! state_expr {
    () => {
        "(CASE WHEN a.status = 'revoked' THEN 'revoked' \
               WHEN a.first_connected_at IS NULL THEN 'waiting' \
               WHEN a.stream_id IS NOT NULL AND a.last_seen_at >= $8 THEN 'online' \
               ELSE 'offline' END)"
    };
}

// The binds are the same in each query: $1 status, $2 search pattern, $3 states, $4 systems,
// $5 versions, $6 labels, $7 scripts, $8 the start of the online window.
macro_rules! filter_sql {
    () => {
        concat!(
            " WHERE ($1::text IS NULL OR a.status = $1) \
               AND ($2::text IS NULL OR a.hostname ILIKE $2 \
                    OR EXISTS (SELECT 1 FROM unnest(a.labels) l WHERE l ILIKE $2)) \
               AND (cardinality($3::text[]) = 0 OR ",
            state_expr!(),
            " = ANY($3)) \
               AND (cardinality($4::text[]) = 0 OR a.os = ANY($4)) \
               AND (cardinality($5::text[]) = 0 OR a.agent_version = ANY($5)) \
               AND (cardinality($6::text[]) = 0 OR a.labels && $6) \
               AND ($7::bool IS NULL OR (a.server_scripts = 'enabled') = $7)"
        )
    };
}

// $1 status, $2 whether a revoked host counts.
macro_rules! visible_sql {
    () => {
        " WHERE ($1::text IS NULL OR a.status = $1) AND ($2::bool OR a.status <> 'revoked')"
    };
}

/// The pattern of a search text: the text matches anywhere, and `%`, `_` and `\` match themselves.
pub(crate) fn search_pattern(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len() + 2);
    pattern.push('%');
    for char in text.chars() {
        if matches!(char, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(char);
    }
    pattern.push('%');
    pattern
}

/// Orders the versions by their numbers, so `0.10.0` follows `0.9.1`. A version that does not
/// start with three numbers has no key and sorts last, by its text.
const VERSION_KEY: &str =
    r"(regexp_match(a.agent_version, '^(\d{1,9})\.(\d{1,9})\.(\d{1,9})'))::int[]";
// `0.4.1-rc.1` sorts before `0.4.1`, as in semantic versioning.
const VERSION_RELEASE: &str = r"(a.agent_version ~ '^\d+\.\d+\.\d+$')";

fn version_order(dir: &str) -> String {
    format!("{VERSION_KEY} {dir} NULLS LAST, {VERSION_RELEASE} {dir}, a.agent_version {dir}")
}

fn order_sql(sort: AgentSort, descending: bool) -> String {
    let dir = if descending { "DESC" } else { "ASC" };
    match sort {
        AgentSort::Created => format!("a.created_at {dir}, a.id {dir}"),
        AgentSort::Hostname => format!("lower(a.hostname) {dir}, a.id {dir}"),
        AgentSort::LastSeen => format!("a.last_seen_at {dir} NULLS LAST, a.id {dir}"),
        AgentSort::AgentVersion => format!("{}, lower(a.hostname), a.id", version_order(dir)),
        AgentSort::Os => format!("a.os {dir}, lower(a.hostname), a.id"),
    }
}

struct Binds {
    status: Option<&'static str>,
    search: Option<String>,
    states: Vec<String>,
}

fn binds(filter: &AgentFilter) -> Binds {
    Binds {
        status: filter.status.map(AgentStatus::as_str),
        search: filter
            .search
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(search_pattern),
        states: filter
            .states
            .iter()
            .map(|state| state.as_str().to_owned())
            .collect(),
    }
}

/// Reads one page of the hosts that match the filter, and their total.
pub async fn search(
    tx: &mut TenantTx<'_>,
    query: &AgentQuery<'_>,
) -> Result<AgentSearchResult, DbError> {
    let limit = query.limit.clamp(1, MAX_PAGE_LIMIT);
    let filter = query.filter;
    let bound = binds(filter);
    let (offset, cursor_time, cursor_id) = match query.window {
        AgentWindow::Offset(offset) => (offset.max(0), None, None),
        AgentWindow::After(cursor) => (0, Some(cursor.created_at), Some(cursor.id.as_uuid())),
    };
    let sql = format!(
        "SELECT {columns} FROM agents a {join} {filter} \
           AND ($9::timestamptz IS NULL OR (a.created_at, a.id) < ($9::timestamptz, $10::uuid)) \
         ORDER BY {order} LIMIT $11 OFFSET $12",
        columns = agent_columns!(),
        join = duplicate_join!(),
        filter = filter_sql!(),
        order = order_sql(query.sort, query.descending),
    );
    let mut agents: Vec<Agent> = sqlx::query_as::<_, AgentRow>(sqlx::AssertSqlSafe(sql))
        .bind(bound.status)
        .bind(bound.search.as_deref())
        .bind(&bound.states)
        .bind(&filter.os)
        .bind(&filter.agent_versions)
        .bind(&filter.labels)
        .bind(filter.server_scripts)
        .bind(query.online_since)
        .bind(cursor_time)
        .bind(cursor_id)
        .bind(limit + 1)
        .bind(offset)
        .fetch_all(tx.as_conn())
        .await
        .map_err(DbError::Query)?
        .into_iter()
        .map(Agent::from)
        .collect();
    let has_more = agents.len() as i64 > limit;
    agents.truncate(limit as usize);
    let counted: i64 = sqlx::query_scalar(concat!(
        "SELECT count(*) FROM (SELECT 1 FROM agents a",
        filter_sql!(),
        " LIMIT $9) s"
    ))
    .bind(bound.status)
    .bind(bound.search.as_deref())
    .bind(&bound.states)
    .bind(&filter.os)
    .bind(&filter.agent_versions)
    .bind(&filter.labels)
    .bind(filter.server_scripts)
    .bind(query.online_since)
    .bind(COUNT_LIMIT + 1)
    .fetch_one(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(AgentSearchResult {
        agents,
        has_more,
        total: counted.min(COUNT_LIMIT),
        total_capped: counted > COUNT_LIMIT,
    })
}

/// Counts the hosts of each state under the filter without its states, and lists the systems,
/// the agent versions and the labels of the hosts that the state filter can show.
pub async fn facets(
    tx: &mut TenantTx<'_>,
    filter: &AgentFilter,
    online_since: OffsetDateTime,
) -> Result<AgentFacets, DbError> {
    let bound = binds(filter);
    let no_states: Vec<String> = Vec::new();
    let counted: Vec<(String, i64)> = sqlx::query_as(concat!(
        "SELECT ",
        state_expr!(),
        ", count(*) FROM agents a",
        filter_sql!(),
        " GROUP BY 1"
    ))
    .bind(bound.status)
    .bind(bound.search.as_deref())
    .bind(&no_states)
    .bind(&filter.os)
    .bind(&filter.agent_versions)
    .bind(&filter.labels)
    .bind(filter.server_scripts)
    .bind(online_since)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let states = HostState::ALL
        .into_iter()
        .map(|state| {
            let count = counted
                .iter()
                .find(|(value, _)| value == state.as_str())
                .map_or(0, |(_, count)| *count);
            (state, count)
        })
        .collect();
    let include_revoked = filter.states.is_empty() || filter.states.contains(&HostState::Revoked);
    let os = sqlx::query_as(concat!(
        "SELECT a.os, count(*) FROM agents a",
        visible_sql!(),
        " GROUP BY a.os ORDER BY count(*) DESC, a.os"
    ))
    .bind(bound.status)
    .bind(include_revoked)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    let sql = format!(
        "SELECT a.agent_version, count(*) FROM agents a {visible} GROUP BY a.agent_version \
         ORDER BY {order}",
        visible = visible_sql!(),
        order = version_order("DESC"),
    );
    let agent_versions = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(bound.status)
        .bind(include_revoked)
        .fetch_all(tx.as_conn())
        .await
        .map_err(DbError::Query)?;
    let labels = sqlx::query_as(concat!(
        "SELECT l, count(*) FROM agents a CROSS JOIN unnest(a.labels) l",
        visible_sql!(),
        " GROUP BY l ORDER BY count(*) DESC, l"
    ))
    .bind(bound.status)
    .bind(include_revoked)
    .fetch_all(tx.as_conn())
    .await
    .map_err(DbError::Query)?;
    Ok(AgentFacets {
        states,
        os,
        agent_versions,
        labels,
    })
}

#[cfg(test)]
mod tests {
    use super::search_pattern;

    #[test]
    fn a_search_text_matches_its_wildcard_characters_literally() {
        assert_eq!(search_pattern("web"), "%web%");
        assert_eq!(search_pattern("50%_a\\b"), "%50\\%\\_a\\\\b%");
    }
}
