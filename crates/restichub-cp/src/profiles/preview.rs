//! The draft preview of a profile: the steps and the command lines for one agent
//! (FR-PF-04, AC-PF-01, AC-PF-02).
//!
//! A caller with the profile write right previews its draft. A caller without that right changes no
//! setting, so the server compiles the stored profile that the request names and ignores every
//! draft field.
//!
//! Only an Admin changes the code that runs on a host, so a caller without the script write right
//! keeps the stored scripts of its draft (FR-HK-12).

use axum::Json;
use axum::extract::State;
use restichub_compiler::compile;
use restichub_db::agents as agents_repo;
use restichub_db::plans as plans_repo;
use restichub_db::profiles::{self as repo, Profile};
use restichub_db::repositories::{self as repositories_repo, Repository};
use restichub_db::{AgentId, ProfileId};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::parameters::{ParameterDef, ParameterDefRequest, ParameterDefView};
use crate::scripts::ScriptDef;
use crate::server::AppState;
use crate::validation::Reason;

use super::settings::{
    BackupOptions, CheckOptions, CreateProfileRequest, ProfilePaths, Retention, profile_tag,
    validate_draft,
};
use super::{
    compile_error_to_api, compiler_profile_from_row, parse_agent_id, preview_error_to_api,
    repository_access, script_secrets, target_facts_with,
};

/// The profile fields describe a draft: the server validates them as a create request, stores
/// nothing and checks no name uniqueness. A caller without the profile write right sends the
/// identifiers only, and the server compiles the stored profile.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    /// The identifier of the registered agent that supplies the target facts.
    pub agent_id: String,
    /// The repository of the commands. Absent uses the repository of the first plan of `profile_id`
    /// on this agent, then of the first plan of `profile_id`, and no repository when the draft
    /// holds no stored profile or plan.
    #[serde(default)]
    pub repository_id: Option<String>,
    /// The stored profile that the draft edits. Required for a caller without the profile write
    /// right: the preview then compiles the stored profile.
    #[serde(default)]
    pub profile_id: Option<String>,
    /// Free text of 0 to 100 characters. An unsaved profile has no name, so the preview accepts an
    /// empty value and checks no uniqueness. Holds no control or format character.
    #[serde(default)]
    #[schema(max_length = 100)]
    pub name: String,
    #[serde(default)]
    pub paths: ProfilePaths,
    #[serde(default)]
    pub excludes: Vec<String>,
    #[serde(default)]
    pub exclude_files: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub options: BackupOptions,
    /// Scripts from the control plane, in the order they run in their phase. The preview accepts
    /// them from a caller with the script write right only; another caller gets the stored scripts.
    #[serde(default)]
    pub scripts: Vec<ScriptDef>,
    #[serde(default)]
    pub retention: Option<Retention>,
    #[serde(default)]
    pub check: Option<CheckOptions>,
    /// A caller without the profile write right sends no fields, and the stored profile gives them.
    #[serde(default)]
    pub parameters: Vec<ParameterDefRequest>,
    /// Accepted only as an empty list, so an old client keeps working; use `scripts` instead
    /// (ADR-0040).
    #[serde(default)]
    #[schema(ignore)]
    pub hooks: Option<Vec<serde_json::Value>>,
}

impl From<PreviewRequest> for CreateProfileRequest {
    fn from(body: PreviewRequest) -> Self {
        Self {
            name: body.name,
            paths: body.paths,
            excludes: body.excludes,
            exclude_files: body.exclude_files,
            tags: body.tags,
            options: body.options,
            scripts: body.scripts,
            retention: body.retention,
            check: body.check,
            parameters: body.parameters,
            hooks: body.hooks,
        }
    }
}

/// One environment variable of a previewed step. A secret value is masked.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewEnvView {
    pub name: String,
    pub value: String,
}

/// One previewed step: the kind, the maintenance flag, the command line and the environment.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewStepView {
    /// The step kind, for example `backup`, `forget`, `prune`, `check` or `script`.
    pub kind: String,
    /// True for `forget`, `prune` and `check`.
    pub maintenance: bool,
    /// Rendered argv with masked secrets.
    pub command: String,
    /// Process variables with masked secrets.
    pub environment: Vec<PreviewEnvView>,
}

/// The preview of a profile draft for one target agent.
#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewResponse {
    /// Null when the draft names no repository and its stored profile has no plan, so the steps
    /// carry no repository.
    pub repository_id: Option<String>,
    pub steps: Vec<PreviewStepView>,
}

/// Previews a profile draft for one target agent (FR-PF-04, AC-PF-01).
///
/// The endpoint validates the draft and compiles it with the facts of the registered agent, then
/// renders the steps and the command lines with masked secrets. A compile error points to the
/// draft field (AC-PF-02). A new profile has no name yet, so the preview accepts an empty name and
/// keeps every other check.
#[utoipa::path(
    post,
    path = "/api/v1/profiles/preview",
    tags = ["profiles"],
    summary = "Previews a profile draft for one target agent.",
    description = "The steps and the command lines agree with the command that the agent runs. The draft is not stored. Secrets are masked. A caller without the profile write right receives the preview of the stored profile that `profile_id` names, and its draft fields are ignored. A caller without the script write right receives the stored scripts and its draft scripts are ignored. A caller without the script read right receives the parameter values of the requested host only when a plan of the stored profile runs there.",
    security(("cookie" = []), ("bearer" = [])),
    request_body = PreviewRequest,
    responses(
        (status = 200, description = "The previewed steps.", body = PreviewResponse),
        (status = 400, description = "The draft or the agent is not valid.", body = crate::error::ProblemBody, content_type = "application/problem+json"),
        crate::error::ProtectedResponses,
        (status = 404, description = "The stored profile of the draft is not found.", body = crate::error::ProblemBody, content_type = "application/problem+json")
    )
)]
pub async fn preview_profile(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<PreviewRequest>,
) -> Result<Json<PreviewResponse>, ApiError> {
    context.authorize(Operation::ProfilesRead)?;
    let agent_id = parse_agent_id(&body.agent_id)?;
    let profile_id = body
        .profile_id
        .as_deref()
        .map(parse_preview_profile_id)
        .transpose()?;
    let repository_id = body.repository_id.clone();
    // A draft names settings, so it needs the write right. Without it the stored profile gives
    // the settings: a template that a Viewer put into the draft would otherwise show a host value
    // in the rendered command (FR-HK-12, FR-PF-12).
    let writes_profile = context.authorize(Operation::ProfilesWrite).is_ok();
    if !writes_profile && profile_id.is_none() {
        return Err(ApiError::invalid_field("profile_id", Reason::Empty));
    }

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(agent) = agents_repo::get(&mut tx, agent_id).await? else {
        return Err(ApiError::invalid_field("agent_id", Reason::Unknown));
    };
    let stored = match profile_id {
        Some(profile_id) => match repo::get(&mut tx, profile_id).await? {
            Some(profile) => Some(profile),
            None => return Err(ApiError::not_found()),
        },
        None => None,
    };
    let draft = if writes_profile {
        let mut request: CreateProfileRequest = body.into();
        // The scripts of the draft of a caller without the script write right change nothing: the
        // stored scripts keep the code that runs on a host (FR-HK-12).
        if context.authorize(Operation::ScriptsWrite).is_err() {
            request.scripts = stored_scripts(stored.as_ref())?;
        }
        Some(validate_draft(request)?)
    } else {
        None
    };
    if let Some(draft) = draft.as_ref() {
        script_secrets::check_script_secrets(&mut tx, &draft.scripts).await?;
    }
    let repository = match repository_id.as_deref() {
        Some(value) => Some(named_repository(&mut tx, value).await?),
        None => planned_repository(&mut tx, profile_id, agent_id).await?,
    };
    // The preview shows the value that the job of the chosen plan runs with: the plan, then the
    // host, then the default (FR-PF-10, AC-PF-08). A new draft has no plan, so the host values and
    // the defaults apply.
    let definitions = match (draft.as_ref(), stored.as_ref()) {
        (Some(draft), _) => draft.parameters.clone(),
        (None, Some(stored)) => crate::parameters::from_row(&stored.parameters)?,
        (None, None) => Vec::new(),
    };
    let may_read_text = crate::parameters::may_read_text(&context);
    let chosen_plan = match profile_id {
        Some(profile_id) => {
            planned_plan(
                &mut tx,
                profile_id,
                agent_id,
                repository.as_ref(),
                may_read_text,
            )
            .await?
        }
        None => None,
    };
    let plan_values = match &chosen_plan {
        Some(plan) => restichub_db::parameters::plan_values(&mut tx, plan.id).await?,
        None => Vec::new(),
    };
    // A host has a value for each of its profiles, so a reader without the script read right gets
    // the values of the requested host only through a plan of the profile that runs there. Any
    // other host would show the values of parameters that this profile never uses (FR-PF-12).
    let host_values = if may_read_text || chosen_plan.is_some() {
        restichub_db::parameters::agent_values(&mut tx, agent_id).await?
    } else {
        Vec::new()
    };
    // This reader gets no text default either: a default becomes the empty placeholder that the
    // profile view shows (FR-PF-12).
    let definitions = if may_read_text {
        definitions
    } else {
        placeholder_defaults(definitions)
    };
    let parameters = crate::parameters::resolve(&definitions, &plan_values, &host_values);
    let compiler_profile = match (draft.as_ref(), stored.as_ref()) {
        (Some(draft), _) => {
            let mut compiler_profile = draft.to_compiler_profile();
            // The retention of a stored profile filters on its own tag, so the commands show it.
            if let Some(profile_id) = profile_id {
                compiler_profile.profile_tag = Some(profile_tag(profile_id));
            }
            compiler_profile
        }
        (None, Some(stored)) => compiler_profile_from_row(stored)?,
        (None, None) => return Err(ApiError::invalid_field("profile_id", Reason::Empty)),
    };
    tx.commit().await?;

    let target = target_facts_with(&agent, state.auth().clock.now(), parameters)?;
    let repository_id = repository
        .as_ref()
        .map(|repository| repository.id.to_string());
    let plan = compile(&compiler_profile, repository_id.as_deref(), &target)
        .map_err(compile_error_to_api)?;
    let access = repository.as_ref().map(repository_access);
    let steps = restichub_compiler::preview(&plan, access.as_ref(), target.os)
        .map_err(preview_error_to_api)?;

    Ok(Json(PreviewResponse {
        repository_id,
        steps: steps
            .iter()
            .map(|step| PreviewStepView {
                kind: step.step.step_id.clone(),
                maintenance: step.maintenance,
                command: step.command.clone(),
                environment: step
                    .environment
                    .iter()
                    .map(|(name, value)| PreviewEnvView {
                        name: name.clone(),
                        value: value.clone(),
                    })
                    .collect(),
            })
            .collect(),
    }))
}

/// The repository that a preview names, which must be in the tenant (FR-PF-04, FR-TN-01).
async fn named_repository(
    tx: &mut restichub_db::TenantTx<'_>,
    value: &str,
) -> Result<Repository, ApiError> {
    let id = uuid::Uuid::parse_str(value.trim()).map_err(|error| {
        ApiError::invalid_field_cause("repository_id", Reason::InvalidFormat, error.to_string())
    })?;
    let repository_id = restichub_db::RepositoryId::from_uuid(id);
    let Some(repository) = repositories_repo::get(tx, repository_id).await? else {
        return Err(ApiError::invalid_field("repository_id", Reason::Unknown));
    };
    Ok(repository)
}

/// The repository of a preview that names none (ADR-0047).
///
/// A profile has no repository of its own. The first plan of the stored profile on the previewed
/// agent most likely names the repository that the user wants; a profile without such a plan falls
/// back to its first plan. A new draft has no plan, and the preview then shows the steps without
/// the repository environment.
async fn planned_repository(
    tx: &mut restichub_db::TenantTx<'_>,
    profile_id: Option<ProfileId>,
    agent_id: AgentId,
) -> Result<Option<Repository>, ApiError> {
    let Some(profile_id) = profile_id else {
        return Ok(None);
    };
    let plans = plans_repo::list_by_profile(tx, profile_id).await?;
    let chosen = plans
        .iter()
        .find(|plan| plan.agent_id == agent_id)
        .or_else(|| plans.first());
    let Some(chosen) = chosen else {
        return Ok(None);
    };
    let Some(repository) = repositories_repo::get(tx, chosen.repository_id).await? else {
        return Err(ApiError::internal(
            "the repository of the plan is not found in the tenant",
        ));
    };
    Ok(Some(repository))
}

/// The plan whose values and repository the preview shows (FR-PF-10).
///
/// The preview names an agent and, when the caller gives one, a repository. The plan of this
/// profile that runs on the agent with that repository supplies the overrides; without such a plan
/// the host values and the defaults apply. `any_host` lets a writer fall back to the plan of another
/// host, so its overrides still show in the commands of the draft.
async fn planned_plan(
    tx: &mut restichub_db::TenantTx<'_>,
    profile_id: ProfileId,
    agent_id: AgentId,
    repository: Option<&Repository>,
    any_host: bool,
) -> Result<Option<plans_repo::Plan>, ApiError> {
    let plans = plans_repo::list_by_profile(tx, profile_id).await?;
    let chosen = plans
        .iter()
        .find(|plan| {
            plan.agent_id == agent_id
                && repository.is_none_or(|repository| plan.repository_id == repository.id)
        })
        .or_else(|| plans.iter().find(|plan| plan.agent_id == agent_id))
        .or_else(|| {
            repository.filter(|_| any_host).and_then(|repository| {
                plans
                    .iter()
                    .find(|plan| plan.repository_id == repository.id)
            })
        });
    Ok(chosen.cloned())
}

/// The declarations as a reader without the script read right sees a text default: as an empty
/// placeholder (FR-PF-12).
fn placeholder_defaults(definitions: Vec<ParameterDef>) -> Vec<ParameterDef> {
    definitions
        .into_iter()
        .map(|definition| ParameterDef {
            default_value: ParameterDefView::of(&definition, false).default_value,
            ..definition
        })
        .collect()
}

/// The scripts of the stored profile of the draft, or none when the draft has no stored profile
/// (FR-HK-12).
fn stored_scripts(stored: Option<&Profile>) -> Result<Vec<ScriptDef>, ApiError> {
    match stored {
        Some(profile) => crate::scripts::from_row(&profile.scripts),
        None => Ok(Vec::new()),
    }
}

/// The stored profile of the draft. It comes from the body, so a bad value names that field.
fn parse_preview_profile_id(value: &str) -> Result<ProfileId, ApiError> {
    uuid::Uuid::parse_str(value.trim())
        .map(ProfileId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause("profile_id", Reason::InvalidFormat, error.to_string())
        })
}
