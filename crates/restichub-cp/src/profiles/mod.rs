//! The profile endpoints (FR-PF-01, FR-PF-03, FR-PF-04, FR-PF-05, ADR-0006).
//!
//! An Admin and an Operator create, update and delete the profiles of the tenant (requirements
//! section 2). A Viewer reads and previews. A profile of another tenant gives HTTP 404 (AC-TN-01).
//! The platform tenant refuses every profile operation with HTTP 403.
//!
//! The settings are a typed closed set: the paths, the excludes, the tags, the options, the scripts,
//! the retention and the check options. There is no free-form map (FR-PR-01).
//!
//! The control plane validates the settings before a write (FR-PF-05). The compiler checks that
//! need a concrete target (the VSS flag on Windows only, the trust mode for scripts and the restic option
//! and version matrix) run in the preview endpoint, because they depend on the facts of a
//! registered agent (FR-PF-04, AC-PF-02). The preview masks the repository password and the
//! backend credentials (AC-PF-01).
//!
//! The schedule is not a profile field: it belongs to the plan (FR-SC-01).

pub(crate) mod script_secrets;
mod settings;

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use restichub_compiler::{
    Architecture, CompileError, OperatingSystem, Plan, ResticVersion, TargetFacts, compile, preview,
};
use restichub_db::agents::{self as agents_repo, Agent};
use restichub_db::audit::{AuditAction, diff};
use restichub_db::plans as plans_repo;
use restichub_db::profiles::{self as repo, ProfileCursor, ProfilePage};
use restichub_db::repositories::{self as repositories_repo, Repository};
use restichub_db::{AgentId, BackendKind, ProfileId};
use restichub_proto::agent::v1::{
    RepositoryAccess, RestCredentials, S3Credentials, repository_access::BackendCredentials,
};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::context::{Operation, RequestContext};
use crate::delete_check;
use crate::error::{ApiError, map_write_error};
use crate::json::ApiJson;
use crate::secrets::actor_user;
use crate::server::AppState;
use crate::validation::Reason;

pub(crate) use settings::compiler_profile_from_row;
pub use settings::{
    BackupOptions, CheckOptions, CreateProfileRequest, GroupBy, HookPhase, JobLimitDefaults,
    MAX_EXCLUDE_FILES, MAX_EXCLUDES, MAX_PATHS, MAX_TAGS, PlanWarning, PreviewEnvView,
    PreviewRequest, PreviewResponse, PreviewStepView, ProfileListResponse, ProfilePaths,
    ProfileResponse, ProfileView, ProfileWarning, Retention, SnapshotFilter, UpdateProfileRequest,
};
use settings::{
    parse_profile_id, profile_value, save_warnings, stored_value, validate_create, validate_update,
};

/// Query parameters of the list endpoint (FR-PF-01).
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
pub struct ProfileListQuery {
    pub cursor: Option<String>,
    pub limit: Option<i64>,
}

/// Lists the profiles of the tenant with keyset pagination.
#[utoipa::path(
    get,
    path = "/api/v1/profiles",
    security(("cookie" = []), ("bearer" = [])),
    params(ProfileListQuery),
    responses(
        (status = 200, description = "The profiles of the tenant. For a Viewer, each script has an empty body and empty values of string variables.", body = ProfileListResponse),
        (status = 400, description = "A query parameter is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_profiles(
    State(state): State<AppState>,
    context: RequestContext,
    Query(query): Query<ProfileListQuery>,
) -> Result<Json<ProfileListResponse>, ApiError> {
    context.authorize(Operation::ProfilesRead)?;
    let page = ProfilePage {
        limit: query.limit.unwrap_or(repo::DEFAULT_PAGE_LIMIT),
        after: query
            .cursor
            .as_deref()
            .map(decode_cursor)
            .transpose()
            .map_err(|cause| {
                ApiError::invalid_field_cause("cursor", Reason::InvalidFormat, cause)
            })?,
    };
    let result = context
        .tenant_tx(&state, async |tx| Ok(repo::list(tx, &page).await?))
        .await?;
    let next_cursor = if result.has_more {
        result.profiles.last().map(|profile| {
            encode_cursor(ProfileCursor {
                created_at: profile.created_at,
                id: profile.id,
            })
        })
    } else {
        None
    };
    let mut items = result
        .profiles
        .iter()
        .map(ProfileView::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    if context.authorize(Operation::ScriptsRead).is_err() {
        items.iter_mut().for_each(ProfileView::hide_script_bodies);
    }
    Ok(Json(ProfileListResponse { items, next_cursor }))
}

/// Reads the job limits that a new profile gets when its request names none (ADR-0039).
#[utoipa::path(
    get,
    path = "/api/v1/profiles/defaults",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The default job limits.", body = JobLimitDefaults),
        crate::error::ProtectedResponses,
    )
)]
pub async fn job_limit_defaults(
    context: RequestContext,
) -> Result<Json<JobLimitDefaults>, ApiError> {
    context.authorize(Operation::ProfilesRead)?;
    Ok(Json(JobLimitDefaults::current()))
}

/// Reads one profile of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/profiles/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the profile.")),
    responses(
        (status = 200, description = "The profile. For a Viewer, each script has an empty body and empty values of string variables.", body = ProfileView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The profile is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_profile(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<ProfileView>, ApiError> {
    context.authorize(Operation::ProfilesRead)?;
    let profile_id = parse_profile_id(&id)?;
    let profile = context
        .tenant_tx(&state, async |tx| Ok(repo::get(tx, profile_id).await?))
        .await?;
    match profile {
        Some(profile) => {
            let mut view = ProfileView::try_from(&profile)?;
            if context.authorize(Operation::ScriptsRead).is_err() {
                view.hide_script_bodies();
            }
            Ok(Json(view))
        }
        None => Err(ApiError::not_found()),
    }
}

/// Creates a profile and validates its settings (FR-PF-01).
#[utoipa::path(
    post,
    path = "/api/v1/profiles",
    summary = "Creates a profile in the tenant.",
    description = "The control plane validates the settings before the write (FR-PF-05).",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateProfileRequest,
    responses(
        (status = 201, description = "The profile is created.", body = ProfileResponse),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 409, description = "The name exists.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_profile(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CreateProfileRequest>,
) -> Result<(StatusCode, Json<ProfileResponse>), ApiError> {
    context.authorize(Operation::ProfilesWrite)?;
    let validated = validate_create(body)?;
    // Code that runs on a host needs the Admin role (FR-HK-12, threat T33).
    if !validated.scripts.is_empty() {
        context.authorize(Operation::ScriptsWrite)?;
    }
    let warnings = save_warnings(&validated)?;
    let profile_id = ProfileId::new();
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    ensure_repository(&mut tx, validated.repository_id).await?;
    script_secrets::check_script_secrets(&mut tx, &validated.scripts).await?;
    repo::create(
        &mut tx,
        &validated.new_profile(profile_id, actor_user(&context.actor)),
    )
    .await
    .map_err(map_write_error)?;

    let mut event =
        context.audit_event(AuditAction::ProfileCreate, "profile", profile_id.as_uuid());
    event.diff = Some(diff(None, Some(&profile_value(&validated)), &[]));
    restichub_db::audit::insert(&mut tx, &event).await?;

    let created = repo::get(&mut tx, profile_id).await?;
    tx.commit().await?;
    let Some(created) = created else {
        return Err(ApiError::internal(
            "the created profile is not found after the read",
        ));
    };
    Ok((
        StatusCode::CREATED,
        Json(ProfileResponse {
            profile: ProfileView::try_from(&created)?,
            warnings,
            plan_warnings: Vec::new(),
        }),
    ))
}

/// Updates a profile. The body replaces the settings; an absent name keeps the name.
#[utoipa::path(
    patch,
    path = "/api/v1/profiles/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the profile.")),
    request_body = UpdateProfileRequest,
    responses(
        (status = 200, description = "The updated profile.", body = ProfileResponse),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The profile is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn update_profile(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateProfileRequest>,
) -> Result<Json<ProfileResponse>, ApiError> {
    context.authorize(Operation::ProfilesWrite)?;
    let profile_id = parse_profile_id(&id)?;
    let now = state.auth().clock.now();
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    // The read locks the row: a concurrent delete cannot remove it between the read and the write.
    let Some(current) = repo::get_for_update(&mut tx, profile_id).await? else {
        return Err(ApiError::not_found());
    };
    let validated = validate_update(body, &current.name)?;
    // A change of the scripts needs the Admin role. An Operator can still save the other settings
    // of a profile that holds scripts, when the scripts stay as they are (FR-HK-12).
    if serde_json::to_value(&validated.scripts).ok().as_ref() != Some(&current.scripts) {
        context.authorize(Operation::ScriptsWrite)?;
    }
    let warnings = save_warnings(&validated)?;
    ensure_repository(&mut tx, validated.repository_id).await?;
    if validated.repository_id != current.repository_id {
        ensure_plans_on_repository_host(&mut tx, profile_id, validated.repository_id).await?;
    }
    script_secrets::check_script_secrets(&mut tx, &validated.scripts).await?;
    repo::update(&mut tx, profile_id, &validated.update())
        .await
        .map_err(map_write_error)?;

    let mut event =
        context.audit_event(AuditAction::ProfileUpdate, "profile", profile_id.as_uuid());
    event.diff = Some(diff(
        Some(&stored_value(&current)),
        Some(&profile_value(&validated)),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;

    let updated = repo::get(&mut tx, profile_id).await?;
    let Some(updated) = updated else {
        return Err(ApiError::internal(
            "the updated profile is not found after the read",
        ));
    };
    // The plans of the profile are revalidated against the facts of their agent. A plan that no
    // longer compiles is a warning, not a refusal; it fails at dispatch time in a later part of
    // epic #7.
    let plan_warnings = plan_compile_warnings(&mut tx, &updated, now).await?;
    tx.commit().await?;
    Ok(Json(ProfileResponse {
        profile: ProfileView::try_from(&updated)?,
        warnings,
        plan_warnings,
    }))
}

/// Names the plans that block the delete of a profile.
#[utoipa::path(
    get,
    path = "/api/v1/profiles/{id}/usage",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the profile.")),
    responses(
        (status = 200, description = "What blocks the delete now.", body = delete_check::UsageView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The profile is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn profile_usage(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<delete_check::UsageView>, ApiError> {
    context.authorize(Operation::ProfilesRead)?;
    let profile_id = parse_profile_id(&id)?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    if repo::get(&mut tx, profile_id).await?.is_none() {
        return Err(ApiError::not_found());
    }
    let block = delete_check::profile_block(&mut tx, profile_id).await?;
    tx.commit().await?;
    Ok(Json(block.into()))
}

/// Removes a profile.
#[utoipa::path(
    delete,
    path = "/api/v1/profiles/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the profile.")),
    responses(
        (status = 204, description = "The profile is removed."),
        crate::error::ProtectedResponses,
        (status = 404, description = "The profile is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "A plan holds the profile (`profile_in_use`). The body lists the plans.", body = crate::error::ProblemBody)
    )
)]
pub async fn delete_profile(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::ProfilesWrite)?;
    let profile_id = parse_profile_id(&id)?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(current) = repo::get_for_update(&mut tx, profile_id).await? else {
        return Err(ApiError::not_found());
    };
    // A plan binds the profile. Remove or repoint the plans first, so the foreign key never blocks
    // the delete with an internal error (FR-SC-11).
    if let Some(block) = delete_check::profile_block(&mut tx, profile_id).await? {
        return Err(block.into_error());
    }
    repo::delete(&mut tx, profile_id)
        .await
        .map_err(map_write_error)?;
    let mut event =
        context.audit_event(AuditAction::ProfileDelete, "profile", profile_id.as_uuid());
    event.diff = Some(diff(Some(&stored_value(&current)), None, &[]));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Previews a profile for one target agent (FR-PF-04, AC-PF-01).
///
/// The endpoint compiles the profile with the facts of the registered agent and renders the steps
/// and the command lines with masked secrets. A compile error points to the profile field
/// (AC-PF-02).
#[utoipa::path(
    post,
    path = "/api/v1/profiles/{id}/preview",
    summary = "Previews a profile for one target agent.",
    description = "The steps and the command lines agree with the command that the agent runs. Secrets are masked (AC-PF-01).",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the profile.")),
    request_body = PreviewRequest,
    responses(
        (status = 200, description = "The previewed steps.", body = PreviewResponse),
        (status = 400, description = "The profile or the agent is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The profile is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn preview_profile(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<PreviewRequest>,
) -> Result<Json<PreviewResponse>, ApiError> {
    context.authorize(Operation::ProfilesRead)?;
    let profile_id = parse_profile_id(&id)?;
    let agent_id = parse_agent_id(&body.agent_id)?;

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(profile) = repo::get(&mut tx, profile_id).await? else {
        return Err(ApiError::not_found());
    };
    let Some(agent) = agents_repo::get(&mut tx, agent_id).await? else {
        return Err(ApiError::invalid_field("agent_id", Reason::Unknown));
    };
    let Some(repository) = repositories_repo::get(&mut tx, profile.repository_id).await? else {
        return Err(ApiError::internal(
            "the repository of the profile is not found in the tenant",
        ));
    };
    tx.commit().await?;

    let target = target_facts(&agent, state.auth().clock.now())?;
    let compiler_profile = compiler_profile_from_row(&profile)?;
    let plan = compile(&compiler_profile, &target).map_err(compile_error_to_api)?;
    let access = repository_access(&repository);
    let steps = preview(&plan, &access, target.os).map_err(preview_error_to_api)?;

    Ok(Json(PreviewResponse {
        profile_id: profile_id.to_string(),
        repository_id: profile.repository_id.to_string(),
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

/// Refuses a repository that is not in the tenant (FR-TN-01).
async fn ensure_repository(
    tx: &mut restichub_db::TenantTx<'_>,
    repository: restichub_db::RepositoryId,
) -> Result<(), ApiError> {
    if repositories_repo::get(tx, repository).await?.is_none() {
        return Err(ApiError::invalid_field("repository_id", Reason::Unknown));
    }
    Ok(())
}

async fn ensure_plans_on_repository_host(
    tx: &mut restichub_db::TenantTx<'_>,
    profile_id: ProfileId,
    repository_id: restichub_db::RepositoryId,
) -> Result<(), ApiError> {
    // The lock orders this save after a concurrent change of the repository host.
    let Some(repository) = repositories_repo::get_for_update(tx, repository_id).await? else {
        return Err(ApiError::invalid_field("repository_id", Reason::Unknown));
    };
    if repository.backend != BackendKind::Local {
        return Ok(());
    }
    let plans = plans_repo::list_by_profile(tx, profile_id).await?;
    if plans
        .iter()
        .any(|plan| repository.maintenance_agent_id != Some(plan.agent_id))
    {
        return Err(ApiError::invalid_field(
            "repository_id",
            Reason::PlanOnOtherHost,
        ));
    }
    Ok(())
}

fn parse_agent_id(value: &str) -> Result<AgentId, ApiError> {
    uuid::Uuid::parse_str(value)
        .map(AgentId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause("agent_id", Reason::InvalidFormat, error.to_string())
        })
}

fn encode_cursor(cursor: ProfileCursor) -> String {
    format!("{};{}", cursor.created_at.unix_timestamp_nanos(), cursor.id)
}

fn decode_cursor(value: &str) -> Result<ProfileCursor, String> {
    let (time, id) = value
        .split_once(';')
        .ok_or_else(|| "the cursor holds no `;`".to_owned())?;
    let nanos: i128 = time
        .parse()
        .map_err(|error: std::num::ParseIntError| error.to_string())?;
    let created_at = time::OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .map_err(|error| error.to_string())?;
    if !(1970..=9999).contains(&created_at.year()) {
        return Err("the time of the cursor is outside the supported range".to_owned());
    }
    let id = id.parse().map_err(|error: uuid::Error| error.to_string())?;
    Ok(ProfileCursor {
        created_at,
        id: ProfileId::from_uuid(id),
    })
}

/// The compiler input facts of a registered agent (FR-PF-03).
pub(crate) fn target_facts(
    agent: &Agent,
    now: time::OffsetDateTime,
) -> Result<TargetFacts, ApiError> {
    let os =
        parse_os(&agent.os).ok_or_else(|| ApiError::invalid_field("agent_id", Reason::Unknown))?;
    let arch = parse_arch(&agent.arch);
    // An agent that reports no parsable version cannot be checked against the option matrix.
    let restic_version = ResticVersion::parse(&agent.restic_version)
        .ok_or_else(|| ApiError::invalid_field("agent_id", Reason::Incompatible))?;
    Ok(TargetFacts {
        agent_id: agent.id.to_string(),
        agent_name: agent.hostname.clone(),
        hostname: agent.hostname.clone(),
        os,
        arch,
        labels: parse_labels(&agent.labels),
        restic_version,
        now,
        server_scripts: crate::scripts::server_scripts_of(agent),
    })
}

fn parse_os(value: &str) -> Option<OperatingSystem> {
    match value.to_ascii_lowercase().as_str() {
        "windows" => Some(OperatingSystem::Windows),
        "linux" => Some(OperatingSystem::Linux),
        "macos" | "darwin" | "mac" => Some(OperatingSystem::Macos),
        _ => None,
    }
}

fn parse_arch(value: &str) -> Architecture {
    match value.to_ascii_lowercase().as_str() {
        "arm64" | "aarch64" => Architecture::Aarch64,
        _ => Architecture::X86_64,
    }
}

/// The labels of the agent as the `{{agent.label.<key>}}` variables. A label without `=` is skipped.
fn parse_labels(labels: &[String]) -> BTreeMap<String, String> {
    labels
        .iter()
        .filter_map(|label| label.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

/// The connection details that the preview renders. The secret values are placeholders: the
/// preview masks them (AC-PF-01). The rest-server user name is not a secret, so it is real.
fn repository_access(repository: &Repository) -> RepositoryAccess {
    let location = repository_location(repository);
    let backend_credentials = match repository.backend {
        BackendKind::S3 => Some(BackendCredentials::S3(S3Credentials {
            access_key_id: "secret".to_owned(),
            secret_access_key: "secret".to_owned(),
            session_token: String::new(),
            region: repository.s3_region.clone().unwrap_or_default(),
        })),
        BackendKind::Rest => Some(BackendCredentials::Rest(RestCredentials {
            username: repository.rest_username.clone().unwrap_or_default(),
            password: "secret".to_owned(),
        })),
        BackendKind::Local => None,
    };
    RepositoryAccess {
        repository_id: repository.id.to_string(),
        location,
        password: "secret".to_owned(),
        backend_credentials,
    }
}

/// The restic address of a stored backend (ADR-0021).
fn repository_location(repository: &Repository) -> String {
    match repository.backend {
        BackendKind::S3 => {
            let endpoint = repository.s3_endpoint.clone().unwrap_or_default();
            let bucket = repository.s3_bucket.clone().unwrap_or_default();
            match repository.s3_prefix.as_deref() {
                Some(prefix) => format!("s3:{endpoint}/{bucket}/{prefix}"),
                None => format!("s3:{endpoint}/{bucket}"),
            }
        }
        BackendKind::Rest => format!(
            "rest:{}",
            repository.rest_endpoint.clone().unwrap_or_default()
        ),
        BackendKind::Local => repository.local_path.clone().unwrap_or_default(),
    }
}

/// Compiles a stored profile for one agent and returns a rejection as a field error (FR-PF-05).
///
/// A plan binds a profile to an agent. The control plane compiles the stored profile with the facts
/// of that agent before a write, so a plan never points at a profile that the compiler rejects for
/// the target operating system (AC-SC-01). The compile refuses a missing path for the operating
/// system of the agent.
pub(crate) fn compile_for_agent(
    profile: &repo::Profile,
    agent: &restichub_db::agents::Agent,
    now: time::OffsetDateTime,
) -> Result<Plan, ApiError> {
    let target = target_facts(agent, now)?;
    let compiler_profile = compiler_profile_from_row(profile)?;
    compile(&compiler_profile, &target).map_err(compile_error_to_api)
}

/// The plans of the profile that no longer compile for the facts of their agent (FR-SC-11).
///
/// The profile update reports them and does not refuse the change. Such a plan keeps its schedule
/// and fails at dispatch time, when the queue recompiles the profile for the agent.
async fn plan_compile_warnings(
    tx: &mut restichub_db::TenantTx<'_>,
    profile: &repo::Profile,
    now: time::OffsetDateTime,
) -> Result<Vec<PlanWarning>, ApiError> {
    let compiler_profile = compiler_profile_from_row(profile)?;
    let mut warnings = Vec::new();
    for plan in plans_repo::list_by_profile(tx, profile.id).await? {
        let Some(agent) = agents_repo::get(tx, plan.agent_id).await? else {
            continue;
        };
        let Ok(target) = target_facts(&agent, now) else {
            continue;
        };
        if let Err(error) = compile(&compiler_profile, &target) {
            let (field, reason) = compile_error_parts(&error);
            warnings.push(PlanWarning {
                plan_id: plan.id.to_string(),
                name: plan.name,
                field,
                reason: reason.as_str().to_owned(),
            });
        }
    }
    Ok(warnings)
}

/// Turns a compiler rejection into a field error (AC-PF-02).
fn compile_error_to_api(error: CompileError) -> ApiError {
    let (field, reason) = compile_error_parts(&error);
    ApiError::invalid_field(field, reason)
}

/// The field and the stable reason of a compiler rejection (AC-PF-02).
fn compile_error_parts(error: &CompileError) -> (String, Reason) {
    match error {
        CompileError::UnknownVariable { field, .. }
        | CompileError::MissingLabel { field, .. }
        | CompileError::InvalidDate { field, .. }
        | CompileError::UnterminatedTemplate { field }
        | CompileError::InvalidValue { field } => (field.to_string(), Reason::InvalidFormat),
        CompileError::InvalidCron { .. } => ("cron".to_owned(), Reason::InvalidFormat),
        CompileError::NoPaths { .. } => ("paths".to_owned(), Reason::Empty),
        CompileError::ServerScriptsDisabled { field } => {
            (field.to_string(), Reason::ServerScriptsDisabled)
        }
        CompileError::InterpreterUnavailable { field, .. } => {
            (field.to_string(), Reason::Unavailable)
        }
        CompileError::UnsupportedOs { field, .. } => (field.to_string(), Reason::Unsupported),
        CompileError::IncompatibleOption { field, .. } => (field.to_string(), Reason::Incompatible),
        CompileError::UnsupportedRestic { .. } => ("agent_id".to_owned(), Reason::Incompatible),
        CompileError::StepsRejected { .. } => ("steps".to_owned(), Reason::InvalidFormat),
    }
}

/// Turns a rendering failure into a field error.
fn preview_error_to_api(error: restichub_compiler::PreviewError) -> ApiError {
    match error {
        restichub_compiler::PreviewError::Rejected(rejection) => {
            ApiError::invalid_field_cause("steps", Reason::InvalidFormat, format!("{rejection:?}"))
        }
        restichub_compiler::PreviewError::Arg(error) => {
            ApiError::invalid_field_cause("steps", Reason::InvalidFormat, error.to_string())
        }
    }
}
