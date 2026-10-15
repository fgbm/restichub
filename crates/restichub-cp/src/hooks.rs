//! The hooks that the agents report and the catalogue for the profile (FR-HK-04, ADR-0004).
//!
//! A host administrator lists the hooks in a protected file on the host. The agent reports the
//! identifier, the phases and the parameter schema of each hook in `AgentHello`, never a path.
//! The control plane keeps the last report in the agent row. The profile form offers the hooks of
//! the active agents, and the control plane checks a profile against the report of each target
//! agent (AC-HK-05). The control plane cannot add a hook: it only refers to the identifiers that
//! the agents report (threat T2).

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::State;
use restichub_compiler::{AllowListedHook, HookPhase as CompilerPhase, ParameterKind};
use restichub_db::agents::{self as agents_repo, Agent};
use restichub_db::{AgentStatus, HookAllowListState};
use restichub_proto::AgentCapability;
use restichub_proto::agent::v1::{AgentHello, HookAllowListState as ProtoState};
use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;

use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::profiles::HookPhase;
use crate::server::AppState;

/// The most hooks that the control plane keeps for one agent.
pub(crate) const MAX_REPORTED_HOOKS: usize = 64;

/// The hooks of one `AgentHello`, ready to store.
pub(crate) struct HelloHooks {
    /// A JSON object that maps a hook identifier to its schema.
    pub(crate) hooks: Value,
    pub(crate) state: HookAllowListState,
}

/// Reads the hook report of a hello. An agent without the capability `hooks-v1` reports none.
///
/// A hook with metadata that the control plane cannot use is dropped and logged, so one broken
/// entry does not hide the others.
pub(crate) fn from_hello(hello: &AgentHello, capabilities: &[AgentCapability]) -> HelloHooks {
    if !capabilities.contains(&AgentCapability::HooksV1) {
        return HelloHooks {
            hooks: Value::Object(serde_json::Map::new()),
            state: HookAllowListState::Unknown,
        };
    }
    let state = match ProtoState::try_from(hello.hook_allow_list_state) {
        Ok(ProtoState::Absent) => HookAllowListState::Absent,
        Ok(ProtoState::Loaded) => HookAllowListState::Loaded,
        Ok(ProtoState::Insecure) => HookAllowListState::Insecure,
        Ok(ProtoState::Invalid) => HookAllowListState::Invalid,
        Ok(ProtoState::Unspecified) | Err(_) => HookAllowListState::Unknown,
    };
    let mut hooks = serde_json::Map::new();
    // Only a trusted file gives hooks (AC-HK-01).
    if state == HookAllowListState::Loaded {
        for metadata in hello.hooks.iter().take(MAX_REPORTED_HOOKS) {
            match AllowListedHook::from_proto(metadata) {
                Ok((id, hook)) => {
                    if let Ok(value) = serde_json::to_value(&hook) {
                        hooks.insert(id, value);
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "an agent reported a hook that the control plane refuses");
                }
            }
        }
    }
    HelloHooks {
        hooks: Value::Object(hooks),
        state,
    }
}

/// The hooks that an agent reported last, by identifier.
pub(crate) fn allow_list_of(agent: &Agent) -> BTreeMap<String, AllowListedHook> {
    serde_json::from_value(agent.hooks.clone()).unwrap_or_default()
}

/// The type of a hook parameter in the API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HookParameterType {
    String,
    Integer,
    Enum,
    Boolean,
    SecretRef,
}

/// One parameter of a reported hook (FR-HK-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct HookParameterView {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: HookParameterType,
    /// True when every use of the hook must set the parameter.
    pub required: bool,
    /// Longest string in bytes. Only a string parameter has it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    /// Regular expression that the whole string must match.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<i64>,
    /// The allowed values of an enumeration.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
}

/// One hook that an agent reports (FR-HK-04). The view holds no path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct HookView {
    pub hook_id: String,
    /// The phases where the hook may run. An empty list allows every phase.
    pub phases: Vec<HookPhase>,
    pub parameters: Vec<HookParameterView>,
}

impl HookView {
    pub(crate) fn new(hook_id: &str, hook: &AllowListedHook) -> Self {
        Self {
            hook_id: hook_id.to_owned(),
            phases: hook.phases.iter().map(|phase| phase_view(*phase)).collect(),
            parameters: hook
                .parameters
                .iter()
                .map(|(name, spec)| parameter_view(name, spec))
                .collect(),
        }
    }
}

/// The hooks of an agent for its view, in the order of the identifiers.
pub(crate) fn views_of(agent: &Agent) -> Vec<HookView> {
    allow_list_of(agent)
        .iter()
        .map(|(id, hook)| HookView::new(id, hook))
        .collect()
}

fn phase_view(phase: CompilerPhase) -> HookPhase {
    match phase {
        CompilerPhase::Before => HookPhase::Before,
        CompilerPhase::After => HookPhase::After,
        CompilerPhase::OnSuccess => HookPhase::OnSuccess,
        CompilerPhase::OnError => HookPhase::OnError,
        CompilerPhase::Finally => HookPhase::Finally,
    }
}

fn parameter_view(name: &str, spec: &restichub_compiler::HookParameterSpec) -> HookParameterView {
    let mut view = HookParameterView {
        name: name.to_owned(),
        kind: HookParameterType::String,
        required: spec.required,
        max_length: None,
        pattern: None,
        minimum: None,
        maximum: None,
        values: Vec::new(),
    };
    match &spec.kind {
        ParameterKind::String {
            max_length,
            pattern,
        } => {
            view.max_length = Some(*max_length);
            view.pattern.clone_from(pattern);
        }
        ParameterKind::Integer { minimum, maximum } => {
            view.kind = HookParameterType::Integer;
            view.minimum = *minimum;
            view.maximum = *maximum;
        }
        ParameterKind::Enum { values } => {
            view.kind = HookParameterType::Enum;
            view.values.clone_from(values);
        }
        ParameterKind::Boolean => view.kind = HookParameterType::Boolean,
        ParameterKind::SecretRef => view.kind = HookParameterType::SecretRef,
    }
    view
}

/// An active agent in the hook catalogue.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogueAgent {
    pub id: String,
    pub hostname: String,
    pub hook_allow_list: HookAllowListState,
}

/// A hook and the agents that report it with this schema.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogueHook {
    #[serde(flatten)]
    pub hook: HookView,
    /// The active agents that report the hook with exactly this schema.
    pub agent_ids: Vec<String>,
}

/// The hooks that the active agents of the tenant report (FR-HK-04).
#[derive(Debug, Serialize, ToSchema)]
pub struct HookCatalogue {
    pub agents: Vec<CatalogueAgent>,
    pub hooks: Vec<CatalogueHook>,
}

/// The most agent pages that the catalogue reads.
const MAX_PAGES: usize = 10;

/// Lists the hooks that the active agents report.
///
/// A hook is a candidate for a profile when every target agent of the profile reports it. An
/// agent that reports the same identifier with another schema gives a second entry.
#[utoipa::path(
    get,
    path = "/api/v1/hooks",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The hooks that the active agents of the tenant report.", body = HookCatalogue),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_hooks(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<HookCatalogue>, ApiError> {
    context.authorize(Operation::AgentsRead)?;
    let agents = context
        .tenant_tx(&state, async |tx| Ok(active_agents(tx).await?))
        .await?;
    Ok(Json(catalogue(&agents)))
}

/// The active agents of the tenant, newest first, up to ten pages.
pub(crate) async fn active_agents(
    tx: &mut restichub_db::TenantTx<'_>,
) -> Result<Vec<Agent>, restichub_db::DbError> {
    let mut agents = Vec::new();
    let mut page = agents_repo::AgentPage::default();
    for _ in 0..MAX_PAGES {
        let result = agents_repo::list(tx, Some(AgentStatus::Active), &page).await?;
        let last = result.agents.last().map(|agent| agents_repo::AgentCursor {
            created_at: agent.created_at,
            id: agent.id,
        });
        agents.extend(result.agents);
        match (result.has_more, last) {
            (true, Some(cursor)) => page.after = Some(cursor),
            _ => break,
        }
    }
    Ok(agents)
}

fn catalogue(agents: &[Agent]) -> HookCatalogue {
    let mut grouped: BTreeMap<(String, String), CatalogueHook> = BTreeMap::new();
    for agent in agents {
        for (id, hook) in allow_list_of(agent) {
            let schema = serde_json::to_string(&hook).unwrap_or_default();
            grouped
                .entry((id.clone(), schema))
                .or_insert_with(|| CatalogueHook {
                    hook: HookView::new(&id, &hook),
                    agent_ids: Vec::new(),
                })
                .agent_ids
                .push(agent.id.to_string());
        }
    }
    HookCatalogue {
        agents: agents
            .iter()
            .map(|agent| CatalogueAgent {
                id: agent.id.to_string(),
                hostname: agent.hostname.clone(),
                hook_allow_list: agent.hook_allow_list,
            })
            .collect(),
        hooks: grouped.into_values().collect(),
    }
}
