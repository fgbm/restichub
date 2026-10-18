//! The host list endpoint: filters, sort and pages on the server (FR-UI-01, ADR-0009).
//!
//! A client that reads the whole list follows `next_cursor` in the default order. The Hosts screen
//! reads numbered pages with `offset` and asks for the state counts and the filter values only
//! when its filters change.

use axum::Json;
use axum::extract::State;
use restichub_db::AgentStatus;
use restichub_db::agent_search::{
    self as search, AgentFilter, AgentQuery, AgentSort, AgentWindow, HostState,
};
use restichub_db::agents as repo;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::agents::{AgentView, decode_cursor, encode_cursor};
use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::list_query::{
    ApiQuery, FacetValue, SortOrder, check_window, parse_list, parse_search, split_list,
};
use crate::server::AppState;
use crate::validation::Reason;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentSortKey {
    CreatedAt,
    Hostname,
    LastSeenAt,
    AgentVersion,
    Os,
}

impl From<AgentSortKey> for AgentSort {
    fn from(key: AgentSortKey) -> Self {
        match key {
            AgentSortKey::CreatedAt => Self::Created,
            AgentSortKey::Hostname => Self::Hostname,
            AgentSortKey::LastSeenAt => Self::LastSeen,
            AgentSortKey::AgentVersion => Self::AgentVersion,
            AgentSortKey::Os => Self::Os,
        }
    }
}

/// Whether the host trusts the scripts of the control plane (FR-HK-08).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServerScriptsFilter {
    Enabled,
    /// Every host without the trust mode, also a host that reports no state.
    Disabled,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct AgentListQuery {
    #[param(inline)]
    pub status: Option<AgentStatus>,
    pub cursor: Option<String>,
    pub limit: Option<i64>,
    /// The first row of a numbered page, at most 10000. Do not combine with `cursor`.
    pub offset: Option<i64>,
    /// A part of the host name or of a label, in any case; at most 200 characters.
    pub q: Option<String>,
    /// Comma-separated, at most 50 values in each list: `online`, `offline`, `waiting`, `revoked`.
    pub state: Option<String>,
    /// Comma-separated operating systems.
    pub os: Option<String>,
    /// Comma-separated, for example `0.4.1,0.4.2`.
    pub agent_version: Option<String>,
    /// Comma-separated; a host with one of them matches.
    pub label: Option<String>,
    #[param(inline)]
    pub server_scripts: Option<ServerScriptsFilter>,
    /// Default: `created_at`. A cursor takes only the default order.
    #[param(inline)]
    pub sort: Option<AgentSortKey>,
    /// Default: `desc`.
    #[param(inline)]
    pub order: Option<SortOrder>,
    /// Adds the state counts and the filter values to the response.
    pub facets: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentStateCounts {
    pub online: i64,
    pub offline: i64,
    pub waiting: i64,
    pub revoked: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentListFacets {
    /// Counted under the other filters, so each state shows what it adds.
    pub states: AgentStateCounts,
    /// The values of the tenant, each with its number of hosts.
    pub os: Vec<FacetValue>,
    pub agent_versions: Vec<FacetValue>,
    pub labels: Vec<FacetValue>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentListResponse {
    pub items: Vec<AgentView>,
    /// Null when the list holds no later agent, and for a numbered page or another order.
    pub next_cursor: Option<String>,
    /// The number of agents that match the filters on all pages, at most 10000.
    pub total: i64,
    /// More agents match than `total` counts.
    pub total_capped: bool,
    /// Present when the request asks for it with `facets=true`.
    pub facets: Option<AgentListFacets>,
}

/// Lists the agents of the tenant with filters, a sort order and a cursor or numbered pages.
#[utoipa::path(
    get,
    path = "/api/v1/agents",
    security(("cookie" = []), ("bearer" = [])),
    params(AgentListQuery),
    responses(
        (status = 200, description = "The agents of the tenant.", body = AgentListResponse),
        (status = 400, description = "A query parameter is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_agents(
    State(state): State<AppState>,
    context: RequestContext,
    ApiQuery(query): ApiQuery<AgentListQuery>,
) -> Result<Json<AgentListResponse>, ApiError> {
    context.authorize(Operation::AgentsRead)?;
    let filter = AgentFilter {
        status: query.status,
        search: parse_search(query.q.as_deref())?,
        states: parse_list("state", query.state.as_deref(), HostState::parse)?,
        os: split_list("os", query.os.as_deref())?,
        agent_versions: split_list("agent_version", query.agent_version.as_deref())?,
        labels: split_list("label", query.label.as_deref())?,
        server_scripts: query
            .server_scripts
            .map(|value| value == ServerScriptsFilter::Enabled),
    };
    let sort = query.sort.map_or(AgentSort::Created, AgentSort::from);
    let order = query.order.unwrap_or_default();
    let default_order = sort == AgentSort::Created && order.descending();
    check_window(query.cursor.as_deref(), query.offset, default_order)?;
    let window = match query.cursor.as_deref() {
        Some(cursor) => AgentWindow::After(decode_cursor(cursor).map_err(|cause| {
            ApiError::invalid_field_cause("cursor", Reason::InvalidFormat, cause)
        })?),
        None => AgentWindow::Offset(query.offset.unwrap_or(0)),
    };
    let now = state.auth().clock.now();
    let online_since = now - crate::dashboard::ONLINE_WINDOW;
    let page = AgentQuery {
        filter: &filter,
        sort,
        descending: order.descending(),
        window,
        limit: query.limit.unwrap_or(repo::DEFAULT_PAGE_LIMIT),
        online_since,
    };
    let with_facets = query.facets.unwrap_or(false);
    let (result, facets) = context
        .tenant_tx(&state, async |tx| {
            let result = search::search(tx, &page).await?;
            let facets = if with_facets {
                Some(search::facets(tx, &filter, online_since).await?)
            } else {
                None
            };
            Ok((result, facets))
        })
        .await?;
    let next_cursor = (result.has_more && default_order && query.offset.is_none())
        .then(|| {
            result.agents.last().map(|agent| {
                encode_cursor(repo::AgentCursor {
                    created_at: agent.created_at,
                    id: agent.id,
                })
            })
        })
        .flatten();
    Ok(Json(AgentListResponse {
        items: result
            .agents
            .iter()
            .map(|agent| AgentView::new(agent, now))
            .collect(),
        next_cursor,
        total: result.total,
        total_capped: result.total_capped,
        facets: facets.map(|facets| {
            let count = |wanted: HostState| {
                facets
                    .states
                    .iter()
                    .find(|(state, _)| *state == wanted)
                    .map_or(0, |(_, count)| *count)
            };
            AgentListFacets {
                states: AgentStateCounts {
                    online: count(HostState::Online),
                    offline: count(HostState::Offline),
                    waiting: count(HostState::Waiting),
                    revoked: count(HostState::Revoked),
                },
                os: facets.os.into_iter().map(FacetValue::from).collect(),
                agent_versions: facets
                    .agent_versions
                    .into_iter()
                    .map(FacetValue::from)
                    .collect(),
                labels: facets.labels.into_iter().map(FacetValue::from).collect(),
            }
        }),
    }))
}
