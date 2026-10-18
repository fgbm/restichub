//! The parameter values of a host (FR-PF-10, FR-PF-11, FR-PF-12).
//!
//! The table of a host shows two sources: the values that the host holds and the parameters that
//! the profiles of the plans of this host declare. A value that no such profile declares stays in
//! the table with the flag `unused`, so the operator can remove it.
//!
//! A Viewer reads the names, the kinds, the required flags, the names of the secrets and the
//! profiles that need each parameter, and the server hides every text value (FR-PF-12). An Admin
//! and an Operator change the text values; only an Admin changes a value of the kind `secret`
//! (ADR-0047).

use std::collections::{BTreeMap, BTreeSet};

use axum::Json;
use axum::extract::{Path, State};
use restichub_db::agents as agents_repo;
use restichub_db::audit::{AuditAction, diff};
use restichub_db::parameters as repo;
use restichub_db::parameters::ParameterValue;
use restichub_db::{AgentId, ParameterKind};
use restichub_db::{plans as plans_repo, profiles as profiles_repo};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::parameters::{
    self, ParameterDef, ParameterValueDef, audit_values, may_read_text, validate_values,
};
use crate::secrets::actor_user;
use crate::server::AppState;
use crate::validation::Reason;

/// The value of one parameter on a host (FR-PF-12).
#[derive(Debug, Serialize, ToSchema)]
pub struct ParameterValueView {
    pub kind: ParameterKind,
    /// The text as the reader sees it; null for a secret and for a hidden text.
    pub value: Option<String>,
    /// The identifier of the secret of a secret parameter.
    pub secret_id: Option<String>,
    /// What the referenced secret is called; a name is not a secret.
    pub secret_name: Option<String>,
}

/// One row of the parameter table of a host (FR-PF-10, FR-PF-12).
#[derive(Debug, Serialize, ToSchema)]
pub struct HostParameterView {
    pub name: String,
    pub kind: ParameterKind,
    /// True when a profile of the plans of this host requires the parameter.
    pub required: bool,
    /// What a text parameter falls back to. Null for a secret, when no profile declares a value
    /// and when the reader may not see it.
    pub default_value: Option<String>,
    pub description: Option<String>,
    /// The profiles of the plans of this host that declare the parameter.
    pub needed_by: Vec<String>,
    /// What the host holds for this parameter. Null when it holds none.
    pub value: Option<ParameterValueView>,
    /// True for a value that no profile of the plans of this host declares.
    pub unused: bool,
    /// True when the server hid a text value from this reader (FR-PF-12).
    pub hidden: bool,
}

/// What the page of a host shows: the values that it holds and the parameters of the profiles of
/// its plans (FR-PF-10).
#[derive(Debug, Serialize, ToSchema)]
pub struct HostParametersResponse {
    pub agent_id: String,
    pub parameters: Vec<HostParameterView>,
}

/// The request body that replaces the values of a host (FR-PF-10).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplaceParametersRequest {
    /// The whole set. A name that the body leaves out loses its value.
    pub parameters: Vec<ParameterValueDef>,
}

/// Reads the parameter table of a host (FR-PF-10, FR-PF-12).
#[utoipa::path(
    get,
    path = "/api/v1/agents/{id}/parameters",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the agent.")),
    responses(
        (status = 200, description = "The values on the host and the parameters of the profiles of its plans. For a Viewer, a text value is empty with the flag `hidden`.", body = HostParametersResponse),
        crate::error::ProtectedResponses,
        (status = 404, description = "The agent is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_parameters(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<HostParametersResponse>, ApiError> {
    context.authorize(Operation::AgentsRead)?;
    let agent_id = parse_agent_id(&id)?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    if agents_repo::get(&mut tx, agent_id).await?.is_none() {
        return Err(ApiError::not_found());
    }
    let declared = declared_of_host(&mut tx, agent_id, false).await?;
    let values = repo::agent_values(&mut tx, agent_id).await?;
    let names = secret_names(&mut tx, &values).await?;
    tx.commit().await?;

    Ok(Json(HostParametersResponse {
        agent_id: agent_id.to_string(),
        parameters: table(&declared, &values, &names, may_read_text(&context)),
    }))
}

/// Replaces the values of a host (FR-PF-10, FR-PF-11).
///
/// The change is refused when it leaves a plan of this host without a required value. The set is
/// replaced in one transaction, and the audit record names the parameters and hides the values
/// (FR-PF-12).
#[utoipa::path(
    put,
    path = "/api/v1/agents/{id}/parameters",
    summary = "Replaces the parameter values of a host.",
    description = "The control plane refuses a change that leaves a plan without a required parameter (`parameter_missing_in_plans`).",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the agent.")),
    request_body = ReplaceParametersRequest,
    responses(
        (status = 200, description = "The stored values.", body = HostParametersResponse),
        (status = 400, description = "A field is not valid, or a plan loses a required value.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The agent is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn replace_parameters(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ReplaceParametersRequest>,
) -> Result<Json<HostParametersResponse>, ApiError> {
    context.authorize(Operation::AgentParametersWrite)?;
    let agent_id = parse_agent_id(&id)?;
    let values = validate_values("parameters", body.parameters)?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    // The lock on the host row serializes two changes of its values and a plan save, which takes a
    // key share lock on the same row (FR-PF-11).
    let Some(agent) = agents_repo::get_for_update(&mut tx, agent_id).await? else {
        return Err(ApiError::not_found());
    };
    let before = repo::agent_values(&mut tx, agent_id).await?;
    // Only an Admin changes a secret value, because it chooses the credential that a script
    // receives (ADR-0047). An entry that the body leaves as it was is no change.
    if super::changes_secret(&before, &values) {
        context.authorize(Operation::ParameterSecretWrite)?;
    }
    let declared = declared_of_host(&mut tx, agent_id, true).await?;
    check_kinds(&declared, &values)?;
    super::checks::check_secret_values(&mut tx, &values, "parameters").await?;
    repo::replace_agent_values(&mut tx, agent_id, &values, actor_user(&context.actor)).await?;
    if let Some(missing) = super::checks::missing_in_plans_of_host(&mut tx, agent.id).await? {
        return Err(super::checks::refuse_missing(missing));
    }
    check_plans_compile(&mut tx, &agent, &values, state.auth().clock.now()).await?;
    let names = secret_names(&mut tx, &values).await?;
    let mut event = context.audit_event(
        AuditAction::AgentParametersUpdate,
        "agent",
        agent_id.as_uuid(),
    );
    event.diff = Some(diff(
        Some(&json!({"parameters": audit_values(&before, None)})),
        Some(&json!({"parameters": audit_values(&values, Some(&before))})),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(Json(HostParametersResponse {
        agent_id: agent_id.to_string(),
        parameters: table(&declared, &values, &names, may_read_text(&context)),
    }))
}

/// Compiles every plan of the host with the new values (FR-PF-10).
///
/// A value can break a plan without breaking the profile, for example a tag that expands to a value
/// with a comma. The refusal names the value that the compile rejects.
async fn check_plans_compile(
    tx: &mut restichub_db::TenantTx<'_>,
    agent: &restichub_db::agents::Agent,
    values: &[ParameterValue],
    now: time::OffsetDateTime,
) -> Result<(), ApiError> {
    for plan in plans_repo::list_by_agent(tx, agent.id).await? {
        let Some(profile) = profiles_repo::get_for_share(tx, plan.profile_id).await? else {
            continue;
        };
        let definitions = parameters::from_row(&profile.parameters)?;
        let plan_values = repo::plan_values(tx, plan.id).await?;
        let compiler_profile = crate::profiles::compiler_profile_from_row(&profile)?;
        let repository = plan.repository_id.to_string();
        let compile_with = |values: &[ParameterValue]| {
            let resolved = parameters::resolve(&definitions, &plan_values, values);
            let target = crate::profiles::target_facts_with(agent, now, resolved)?;
            Ok::<_, ApiError>(restichub_compiler::compile(
                &compiler_profile,
                Some(&repository),
                &target,
            ))
        };
        let Err(error) = compile_with(values)? else {
            continue;
        };
        // A compile error does not always name the parameter, so the check tries each value in turn
        // with a value that fills any template.
        let mut field = "parameters".to_owned();
        for (index, value) in values.iter().enumerate() {
            let mut swapped = values.to_vec();
            swapped[index] = benign(value);
            if compile_with(&swapped)?.is_ok() {
                field = format!("parameters[{index}].value");
                break;
            }
        }
        let (_, reason) = crate::profiles::compile_error_parts(&error);
        return Err(ApiError::invalid_field_cause(
            field,
            reason,
            error.to_string(),
        ));
    }
    Ok(())
}

/// The same value of another kind of parameter that fills a template: a text parameter takes a word
/// and a secret keeps its reference, which no template reads (ADR-0047).
fn benign(value: &ParameterValue) -> ParameterValue {
    match value.kind {
        ParameterKind::Text => ParameterValue {
            value: Some("x".to_owned()),
            ..value.clone()
        },
        ParameterKind::Secret => value.clone(),
    }
}

/// One declaration of a parameter of a profile, with the profiles of the host that declare it.
struct Declared {
    definition: ParameterDef,
    needed_by: BTreeSet<String>,
}

/// The parameters that the profiles of the plans of a host declare (FR-PF-10).
///
/// `for_share` takes a share lock on each profile, so a concurrent save of a profile waits for this
/// transaction and cannot make a parameter required between the check and the commit (FR-PF-11).
async fn declared_of_host(
    tx: &mut restichub_db::TenantTx<'_>,
    agent_id: AgentId,
    for_share: bool,
) -> Result<BTreeMap<String, Declared>, ApiError> {
    let mut declared: BTreeMap<String, Declared> = BTreeMap::new();
    for plan in plans_repo::list_by_agent(tx, agent_id).await? {
        let profile = if for_share {
            profiles_repo::get_for_share(tx, plan.profile_id).await?
        } else {
            profiles_repo::get(tx, plan.profile_id).await?
        };
        let Some(profile) = profile else {
            continue;
        };
        for definition in parameters::from_row(&profile.parameters)? {
            declared
                .entry(definition.name.clone())
                .and_modify(|entry| {
                    entry.definition.required |= definition.required;
                    entry.needed_by.insert(profile.name.clone());
                })
                .or_insert_with(|| Declared {
                    definition,
                    needed_by: BTreeSet::from([profile.name.clone()]),
                });
        }
    }
    Ok(declared)
}

/// The table of a host: the declared parameters, then the values that no profile declares.
fn table(
    declared: &BTreeMap<String, Declared>,
    values: &[ParameterValue],
    names: &BTreeMap<String, String>,
    may_read_text: bool,
) -> Vec<HostParameterView> {
    let mut rows = Vec::new();
    for (name, entry) in declared {
        let stored = values.iter().find(|value| &value.name == name);
        rows.push(HostParameterView {
            name: name.clone(),
            kind: entry.definition.kind,
            required: entry.definition.required,
            default_value: visible_text(entry.definition.default_value.as_deref(), may_read_text),
            description: entry.definition.description.clone(),
            needed_by: entry.needed_by.iter().cloned().collect(),
            value: stored.map(|value| value_view(value, names, may_read_text)),
            unused: false,
            hidden: hidden(stored, may_read_text),
        });
    }
    for value in values
        .iter()
        .filter(|value| !declared.contains_key(&value.name))
    {
        rows.push(HostParameterView {
            name: value.name.clone(),
            kind: value.kind,
            required: false,
            default_value: None,
            description: None,
            needed_by: Vec::new(),
            value: Some(value_view(value, names, may_read_text)),
            unused: true,
            hidden: hidden(Some(value), may_read_text),
        });
    }
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    rows
}

fn value_view(
    value: &ParameterValue,
    names: &BTreeMap<String, String>,
    may_read_text: bool,
) -> ParameterValueView {
    ParameterValueView {
        kind: value.kind,
        value: visible_text(value.value.as_deref(), may_read_text),
        secret_id: value.secret_id.map(|id| id.to_string()),
        secret_name: value
            .secret_id
            .and_then(|id| names.get(&id.to_string()).cloned()),
    }
}

/// A text value goes with the body for an Admin and an Operator only (FR-PF-12).
fn visible_text(value: Option<&str>, may_read_text: bool) -> Option<String> {
    match value {
        Some(value) if may_read_text => Some(value.to_owned()),
        Some(_) => Some(String::new()),
        None => None,
    }
}

fn hidden(value: Option<&ParameterValue>, may_read_text: bool) -> bool {
    !may_read_text && value.is_some_and(|value| value.kind == ParameterKind::Text)
}

async fn secret_names(
    tx: &mut restichub_db::TenantTx<'_>,
    values: &[ParameterValue],
) -> Result<BTreeMap<String, String>, ApiError> {
    let mut names = BTreeMap::new();
    for value in values {
        let Some(id) = value.secret_id else {
            continue;
        };
        if names.contains_key(&id.to_string()) {
            continue;
        }
        let Some(metadata) = restichub_db::secrets::get(tx, id).await? else {
            continue;
        };
        names.insert(id.to_string(), metadata.name);
    }
    Ok(names)
}

/// A submitted value must keep the kind of the declaration (FR-PF-11).
///
/// A name that no profile of this host declares stays allowed: the host may keep a value of a
/// parameter that a profile stopped declaring, and the table shows it as `unused` (ADR-0047).
fn check_kinds(
    declared: &BTreeMap<String, Declared>,
    values: &[ParameterValue],
) -> Result<(), ApiError> {
    for value in values {
        let Some(entry) = declared.get(&value.name) else {
            continue;
        };
        if entry.definition.kind != value.kind {
            return Err(ApiError::parameters(
                "parameters",
                Reason::ParameterKindMismatch,
                vec![value.name.clone()],
            ));
        }
    }
    Ok(())
}

fn parse_agent_id(value: &str) -> Result<AgentId, ApiError> {
    uuid::Uuid::parse_str(value)
        .map(AgentId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause("id", Reason::InvalidFormat, error.to_string())
        })
}
