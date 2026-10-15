//! The script template and job template endpoints (FR-HK-12, ADR-0037).
//!
//! An Admin manages the templates of the tenant, and every change writes an audit record with the
//! SHA-256 hash of the script. Any role that can read the profiles reads the templates. The control
//! plane adds its read-only built-in templates to the lists; they have no row and the identifier
//! `builtin:<name>`. A template of another tenant gives HTTP 404 (AC-TN-01).
//!
//! A template is a starting point. A profile copies the script and keeps the identifier of the
//! template for display, so a later change of the template does not change a profile.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::audit::{AuditAction, diff};
use restichub_db::script_templates::{
    self as repo, JobTemplate, MAX_TEMPLATES, NewJobTemplate, NewScriptTemplate, ScriptTemplate,
    ScriptTemplateUpdate,
};
use restichub_db::{JobTemplateId, ScriptTemplateId};
use restichub_proto::validate::{MAX_SCRIPT_BODY_BYTES, has_forbidden, is_script_env_name};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use utoipa::ToSchema;

use super::{
    EnvKind, FailurePolicy, Interpreter, ScriptDef, audit_value, body_hash, builtin, parse_phase,
    phase_name, validate_scripts,
};
use crate::context::{Operation, RequestContext};
use crate::error::{ApiError, map_write_error};
use crate::json::ApiJson;
use crate::profiles::HookPhase;
use crate::server::AppState;
use crate::validation::Reason;

/// The most parameters that a script template declares.
const MAX_PARAMETERS: usize = 32;

/// The type of a template parameter. The value becomes an environment variable of the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParameterKind {
    String,
    /// The user picks a secret of the purpose `hook_secret`.
    SecretRef,
    Integer,
    Boolean,
}

impl ParameterKind {
    fn is_valid_default(self, value: &str) -> bool {
        match self {
            Self::String => !has_forbidden(value),
            Self::SecretRef => false,
            Self::Integer => value.parse::<i64>().is_ok(),
            Self::Boolean => value == "true" || value == "false",
        }
    }
}

/// One declared parameter of a script template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScriptTemplateParameter {
    pub name: String,
    /// Shown in the form. 1 to 64 characters.
    pub label: String,
    #[serde(rename = "type")]
    pub kind: ParameterKind,
    /// Fills the form. A secret reference has none.
    #[serde(default, rename = "default", skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
}

/// A script template, built-in or of the tenant.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScriptTemplateView {
    /// A UUID, or `builtin:<name>` for a template of the control plane.
    pub id: String,
    pub builtin: bool,
    pub name: String,
    pub description: String,
    pub interpreter: Interpreter,
    pub phase: HookPhase,
    pub body: String,
    pub timeout_seconds: u32,
    pub failure_policy: FailurePolicy,
    pub parameters: Vec<ScriptTemplateParameter>,
    /// In RFC 3339. Absent for a built-in template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// In RFC 3339. Absent for a built-in template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ScriptTemplateListResponse {
    pub items: Vec<ScriptTemplateView>,
}

/// A job template: an ordered set of scripts from which a profile is made.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct JobTemplateView {
    /// A UUID, or `builtin:<name>` for a template of the control plane.
    pub id: String,
    pub builtin: bool,
    pub name: String,
    pub description: String,
    pub scripts: Vec<ScriptDef>,
    /// In RFC 3339. Absent for a built-in template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// In RFC 3339. Absent for a built-in template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct JobTemplateListResponse {
    pub items: Vec<JobTemplateView>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateScriptTemplateRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub interpreter: Interpreter,
    pub phase: HookPhase,
    pub body: String,
    #[serde(default)]
    pub timeout_seconds: u32,
    pub failure_policy: FailurePolicy,
    #[serde(default)]
    pub parameters: Vec<ScriptTemplateParameter>,
}

/// The body of an update. An absent field keeps its value.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateScriptTemplateRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub interpreter: Option<Interpreter>,
    #[serde(default)]
    pub phase: Option<HookPhase>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub timeout_seconds: Option<u32>,
    #[serde(default)]
    pub failure_policy: Option<FailurePolicy>,
    #[serde(default)]
    pub parameters: Option<Vec<ScriptTemplateParameter>>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateJobTemplateRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub scripts: Vec<ScriptDef>,
}

/// The body of an update. An absent field keeps its value.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateJobTemplateRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub scripts: Option<Vec<ScriptDef>>,
}

/// A template identifier from a path: a UUID, or a built-in name.
enum TemplateRef {
    Builtin(String),
    Stored(uuid::Uuid),
}

fn parse_ref(value: &str) -> Result<TemplateRef, ApiError> {
    if let Some(name) = value.strip_prefix("builtin:") {
        return Ok(TemplateRef::Builtin(name.to_owned()));
    }
    uuid::Uuid::parse_str(value)
        .map(TemplateRef::Stored)
        .map_err(|error| {
            ApiError::invalid_field_cause("id", Reason::InvalidFormat, error.to_string())
        })
}

fn format_time(value: OffsetDateTime) -> String {
    use time::format_description::well_known::Rfc3339;
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

fn script_view(row: &ScriptTemplate) -> Result<ScriptTemplateView, ApiError> {
    let broken = |what: &str| ApiError::internal(format!("the stored script template has {what}"));
    Ok(ScriptTemplateView {
        id: row.id.to_string(),
        builtin: false,
        name: row.name.clone(),
        description: row.description.clone(),
        interpreter: Interpreter::parse(&row.interpreter)
            .ok_or_else(|| broken("an interpreter"))?,
        phase: parse_phase(&row.phase).ok_or_else(|| broken("a phase"))?,
        body: row.body.clone(),
        timeout_seconds: u32::try_from(row.timeout_seconds).unwrap_or(0),
        failure_policy: FailurePolicy::parse(&row.failure_policy)
            .ok_or_else(|| broken("a failure policy"))?,
        parameters: serde_json::from_value(row.parameters.clone())
            .map_err(|_| broken("parameters"))?,
        created_at: Some(format_time(row.created_at)),
        updated_at: Some(format_time(row.updated_at)),
    })
}

fn job_view(row: &JobTemplate) -> Result<JobTemplateView, ApiError> {
    Ok(JobTemplateView {
        id: row.id.to_string(),
        builtin: false,
        name: row.name.clone(),
        description: row.description.clone(),
        scripts: super::from_row(&row.scripts)?,
        created_at: Some(format_time(row.created_at)),
        updated_at: Some(format_time(row.updated_at)),
    })
}

fn validate_text(
    field: &'static str,
    value: &str,
    maximum: usize,
    required: bool,
) -> Result<String, ApiError> {
    let value = value.trim();
    if required && value.is_empty() {
        return Err(ApiError::invalid_field(field, Reason::Empty));
    }
    if value.chars().count() > maximum {
        return Err(ApiError::invalid_field(field, Reason::TooLong));
    }
    if has_forbidden(value) {
        return Err(ApiError::invalid_field(field, Reason::InvalidCharacter));
    }
    Ok(value.to_owned())
}

fn validate_parameters(
    parameters: Vec<ScriptTemplateParameter>,
) -> Result<Vec<ScriptTemplateParameter>, ApiError> {
    if parameters.len() > MAX_PARAMETERS {
        return Err(ApiError::invalid_field("parameters", Reason::TooLong));
    }
    let mut names = std::collections::BTreeSet::new();
    let mut result = Vec::with_capacity(parameters.len());
    for (index, mut parameter) in parameters.into_iter().enumerate() {
        let field = format!("parameters[{index}]");
        if !is_script_env_name(&parameter.name) {
            return Err(ApiError::invalid_field(
                format!("{field}.name"),
                Reason::InvalidCharacter,
            ));
        }
        if !names.insert(parameter.name.to_ascii_uppercase()) {
            return Err(ApiError::invalid_field(
                format!("{field}.name"),
                Reason::Reserved,
            ));
        }
        parameter.label = parameter.label.trim().to_owned();
        if parameter.label.is_empty() || parameter.label.chars().count() > 64 {
            return Err(ApiError::invalid_field(
                format!("{field}.label"),
                Reason::InvalidFormat,
            ));
        }
        if let Some(default) = &parameter.default_value
            && (default.len() > 1024 || !parameter.kind.is_valid_default(default))
        {
            return Err(ApiError::invalid_field(
                format!("{field}.default"),
                Reason::InvalidFormat,
            ));
        }
        result.push(parameter);
    }
    Ok(result)
}

/// Checks the body and the limits of a template. A template holds no variable.
fn validate_template_body(body: &str, timeout_seconds: u32) -> Result<(), ApiError> {
    if body.trim().is_empty() {
        return Err(ApiError::invalid_field("body", Reason::Empty));
    }
    if body.len() > MAX_SCRIPT_BODY_BYTES {
        return Err(ApiError::invalid_field("body", Reason::TooLong));
    }
    if body.contains('\0') {
        return Err(ApiError::invalid_field("body", Reason::InvalidCharacter));
    }
    if timeout_seconds > super::MAX_TIMEOUT_SECONDS {
        return Err(ApiError::invalid_field("timeout_seconds", Reason::TooLong));
    }
    Ok(())
}

fn script_template_value(view: &ScriptTemplateView) -> Value {
    json!({
        "name": view.name,
        "description": view.description,
        "interpreter": view.interpreter.as_str(),
        "phase": phase_name(view.phase),
        "timeout_seconds": view.timeout_seconds,
        "failure_policy": view.failure_policy.as_str(),
        "sha256": body_hash(&view.body),
        "parameters": view.parameters,
    })
}

fn job_template_value(view: &JobTemplateView) -> Value {
    json!({
        "name": view.name,
        "description": view.description,
        "scripts": audit_value(&view.scripts),
    })
}

/// Lists the script templates: the built-in ones, then those of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/script-templates",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The script templates.", body = ScriptTemplateListResponse),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_script_templates(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<ScriptTemplateListResponse>, ApiError> {
    context.authorize(Operation::ScriptsRead)?;
    let rows = context
        .tenant_tx(&state, async |tx| Ok(repo::list(tx).await?))
        .await?;
    let mut items = builtin::script_templates();
    for row in &rows {
        items.push(script_view(row)?);
    }
    Ok(Json(ScriptTemplateListResponse { items }))
}

/// Reads one script template.
#[utoipa::path(
    get,
    path = "/api/v1/script-templates/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the template.")),
    responses(
        (status = 200, description = "The template.", body = ScriptTemplateView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The template is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_script_template(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<ScriptTemplateView>, ApiError> {
    context.authorize(Operation::ScriptsRead)?;
    match parse_ref(&id)? {
        TemplateRef::Builtin(name) => builtin::script_template(&name)
            .map(Json)
            .ok_or_else(ApiError::not_found),
        TemplateRef::Stored(uuid) => {
            let row = context
                .tenant_tx(&state, async |tx| {
                    Ok(repo::get(tx, ScriptTemplateId::from_uuid(uuid)).await?)
                })
                .await?;
            match row {
                Some(row) => Ok(Json(script_view(&row)?)),
                None => Err(ApiError::not_found()),
            }
        }
    }
}

/// Creates a script template. Only an Admin can.
#[utoipa::path(
    post,
    path = "/api/v1/script-templates",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateScriptTemplateRequest,
    responses(
        (status = 201, description = "The template is created.", body = ScriptTemplateView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 409, description = "The name exists, or the tenant holds the most templates.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_script_template(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CreateScriptTemplateRequest>,
) -> Result<(StatusCode, Json<ScriptTemplateView>), ApiError> {
    context.authorize(Operation::ScriptsWrite)?;
    let name = validate_text("name", &body.name, 64, true)?;
    let description = validate_text("description", &body.description, 512, false)?;
    let parameters = validate_parameters(body.parameters)?;
    validate_template_body(&body.body, body.timeout_seconds)?;
    let parameters_value = serde_json::to_value(&parameters)
        .map_err(|error| ApiError::internal(format!("cannot encode the parameters: {error}")))?;

    let id = ScriptTemplateId::new();
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    if repo::count(&mut tx).await? >= MAX_TEMPLATES {
        return Err(ApiError::conflict());
    }
    repo::create(
        &mut tx,
        &NewScriptTemplate {
            id,
            name: &name,
            description: &description,
            interpreter: body.interpreter.as_str(),
            phase: phase_name(body.phase),
            body: &body.body,
            timeout_seconds: i32::try_from(body.timeout_seconds).unwrap_or(0),
            failure_policy: body.failure_policy.as_str(),
            parameters: &parameters_value,
            created_by: crate::secrets::actor_user(&context.actor),
        },
    )
    .await
    .map_err(map_write_error)?;
    let created = repo::get(&mut tx, id).await?;
    let Some(created) = created else {
        return Err(ApiError::internal(
            "the created template is not found after the read",
        ));
    };
    let view = script_view(&created)?;
    let mut event = context.audit_event(
        AuditAction::ScriptTemplateCreate,
        "script_template",
        id.as_uuid(),
    );
    event.diff = Some(diff(None, Some(&script_template_value(&view)), &[]));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(view)))
}

/// Updates a script template. Only an Admin can. An absent field keeps its value.
#[utoipa::path(
    patch,
    path = "/api/v1/script-templates/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the template.")),
    request_body = UpdateScriptTemplateRequest,
    responses(
        (status = 200, description = "The updated template.", body = ScriptTemplateView),
        (status = 400, description = "A field is not valid, or the template is built-in.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The template is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The name exists.", body = crate::error::ProblemBody)
    )
)]
pub async fn update_script_template(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateScriptTemplateRequest>,
) -> Result<Json<ScriptTemplateView>, ApiError> {
    context.authorize(Operation::ScriptsWrite)?;
    let TemplateRef::Stored(uuid) = parse_ref(&id)? else {
        return Err(ApiError::invalid_field("id", Reason::Reserved));
    };
    let id = ScriptTemplateId::from_uuid(uuid);
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(row) = repo::get_for_update(&mut tx, id).await? else {
        return Err(ApiError::not_found());
    };
    let current = script_view(&row)?;

    let name = match &body.name {
        Some(name) => validate_text("name", name, 64, true)?,
        None => current.name.clone(),
    };
    let description = match &body.description {
        Some(text) => validate_text("description", text, 512, false)?,
        None => current.description.clone(),
    };
    let interpreter = body.interpreter.unwrap_or(current.interpreter);
    let phase = body.phase.unwrap_or(current.phase);
    let script_body = body.body.unwrap_or_else(|| current.body.clone());
    let timeout_seconds = body.timeout_seconds.unwrap_or(current.timeout_seconds);
    let failure_policy = body.failure_policy.unwrap_or(current.failure_policy);
    let parameters = match body.parameters {
        Some(parameters) => validate_parameters(parameters)?,
        None => current.parameters.clone(),
    };
    validate_template_body(&script_body, timeout_seconds)?;
    let parameters_value = serde_json::to_value(&parameters)
        .map_err(|error| ApiError::internal(format!("cannot encode the parameters: {error}")))?;
    repo::update(
        &mut tx,
        id,
        &ScriptTemplateUpdate {
            name: &name,
            description: &description,
            interpreter: interpreter.as_str(),
            phase: phase_name(phase),
            body: &script_body,
            timeout_seconds: i32::try_from(timeout_seconds).unwrap_or(0),
            failure_policy: failure_policy.as_str(),
            parameters: &parameters_value,
        },
    )
    .await
    .map_err(map_write_error)?;
    let Some(updated) = repo::get(&mut tx, id).await? else {
        return Err(ApiError::internal(
            "the updated template is not found after the read",
        ));
    };
    let view = script_view(&updated)?;
    let mut event = context.audit_event(AuditAction::ScriptTemplateUpdate, "script_template", uuid);
    event.diff = Some(diff(
        Some(&script_template_value(&current)),
        Some(&script_template_value(&view)),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(Json(view))
}

/// Removes a script template. Only an Admin can. A profile keeps the script that it copied.
#[utoipa::path(
    delete,
    path = "/api/v1/script-templates/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the template.")),
    responses(
        (status = 204, description = "The template is removed."),
        (status = 400, description = "The template is built-in.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The template is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn delete_script_template(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::ScriptsWrite)?;
    let TemplateRef::Stored(uuid) = parse_ref(&id)? else {
        return Err(ApiError::invalid_field("id", Reason::Reserved));
    };
    let id = ScriptTemplateId::from_uuid(uuid);
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(row) = repo::get_for_update(&mut tx, id).await? else {
        return Err(ApiError::not_found());
    };
    let current = script_view(&row)?;
    repo::delete(&mut tx, id).await.map_err(map_write_error)?;
    let mut event = context.audit_event(AuditAction::ScriptTemplateDelete, "script_template", uuid);
    event.diff = Some(diff(Some(&script_template_value(&current)), None, &[]));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Validates the scripts of a job template. A secret reference may stay empty: the user picks the
/// secret when a profile is made from the template.
fn validate_template_scripts(scripts: Vec<ScriptDef>) -> Result<Vec<ScriptDef>, ApiError> {
    let blanks: Vec<Vec<usize>> = scripts
        .iter()
        .map(|script| {
            script
                .env
                .iter()
                .enumerate()
                .filter(|(_, variable)| {
                    variable.kind == EnvKind::SecretRef && variable.value.is_empty()
                })
                .map(|(index, _)| index)
                .collect()
        })
        .collect();
    // An empty reference stands in for a nil identifier while the rules run, and comes back after.
    let mut stand_in = scripts;
    for (script, indexes) in stand_in.iter_mut().zip(&blanks) {
        for index in indexes {
            script.env[*index].value = uuid::Uuid::nil().to_string();
        }
    }
    let mut checked = validate_scripts(stand_in, "scripts")?;
    for (script, indexes) in checked.iter_mut().zip(&blanks) {
        for index in indexes {
            script.env[*index].value = String::new();
        }
    }
    Ok(checked)
}

/// Lists the job templates: the built-in ones, then those of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/job-templates",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The job templates.", body = JobTemplateListResponse),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_job_templates(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<JobTemplateListResponse>, ApiError> {
    context.authorize(Operation::ScriptsRead)?;
    let rows = context
        .tenant_tx(&state, async |tx| Ok(repo::list_job_templates(tx).await?))
        .await?;
    let mut items = builtin::job_templates();
    for row in &rows {
        items.push(job_view(row)?);
    }
    Ok(Json(JobTemplateListResponse { items }))
}

/// Reads one job template.
#[utoipa::path(
    get,
    path = "/api/v1/job-templates/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the template.")),
    responses(
        (status = 200, description = "The template.", body = JobTemplateView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The template is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_job_template(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<JobTemplateView>, ApiError> {
    context.authorize(Operation::ScriptsRead)?;
    match parse_ref(&id)? {
        TemplateRef::Builtin(name) => builtin::job_template(&name)
            .map(Json)
            .ok_or_else(ApiError::not_found),
        TemplateRef::Stored(uuid) => {
            let row = context
                .tenant_tx(&state, async |tx| {
                    Ok(repo::get_job_template(tx, JobTemplateId::from_uuid(uuid)).await?)
                })
                .await?;
            match row {
                Some(row) => Ok(Json(job_view(&row)?)),
                None => Err(ApiError::not_found()),
            }
        }
    }
}

/// Creates a job template. Only an Admin can.
#[utoipa::path(
    post,
    path = "/api/v1/job-templates",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateJobTemplateRequest,
    responses(
        (status = 201, description = "The template is created.", body = JobTemplateView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 409, description = "The name exists, or the tenant holds the most templates.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_job_template(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CreateJobTemplateRequest>,
) -> Result<(StatusCode, Json<JobTemplateView>), ApiError> {
    context.authorize(Operation::ScriptsWrite)?;
    let name = validate_text("name", &body.name, 64, true)?;
    let description = validate_text("description", &body.description, 512, false)?;
    let scripts = validate_template_scripts(body.scripts)?;
    let scripts_value = serde_json::to_value(&scripts)
        .map_err(|error| ApiError::internal(format!("cannot encode the scripts: {error}")))?;

    let id = JobTemplateId::new();
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    if repo::count_job_templates(&mut tx).await? >= MAX_TEMPLATES {
        return Err(ApiError::conflict());
    }
    crate::profiles::hooks::check_script_secrets(&mut tx, &scripts).await?;
    repo::create_job_template(
        &mut tx,
        &NewJobTemplate {
            id,
            name: &name,
            description: &description,
            scripts: &scripts_value,
            created_by: crate::secrets::actor_user(&context.actor),
        },
    )
    .await
    .map_err(map_write_error)?;
    let Some(created) = repo::get_job_template(&mut tx, id).await? else {
        return Err(ApiError::internal(
            "the created template is not found after the read",
        ));
    };
    let view = job_view(&created)?;
    let mut event =
        context.audit_event(AuditAction::JobTemplateCreate, "job_template", id.as_uuid());
    event.diff = Some(diff(None, Some(&job_template_value(&view)), &[]));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(view)))
}

/// Updates a job template. Only an Admin can. An absent field keeps its value.
#[utoipa::path(
    patch,
    path = "/api/v1/job-templates/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the template.")),
    request_body = UpdateJobTemplateRequest,
    responses(
        (status = 200, description = "The updated template.", body = JobTemplateView),
        (status = 400, description = "A field is not valid, or the template is built-in.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The template is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The name exists.", body = crate::error::ProblemBody)
    )
)]
pub async fn update_job_template(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateJobTemplateRequest>,
) -> Result<Json<JobTemplateView>, ApiError> {
    context.authorize(Operation::ScriptsWrite)?;
    let TemplateRef::Stored(uuid) = parse_ref(&id)? else {
        return Err(ApiError::invalid_field("id", Reason::Reserved));
    };
    let id = JobTemplateId::from_uuid(uuid);
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(row) = repo::get_job_template_for_update(&mut tx, id).await? else {
        return Err(ApiError::not_found());
    };
    let current = job_view(&row)?;
    let name = match &body.name {
        Some(name) => validate_text("name", name, 64, true)?,
        None => current.name.clone(),
    };
    let description = match &body.description {
        Some(text) => validate_text("description", text, 512, false)?,
        None => current.description.clone(),
    };
    let scripts = match body.scripts {
        Some(scripts) => {
            let checked = validate_template_scripts(scripts)?;
            crate::profiles::hooks::check_script_secrets(&mut tx, &checked).await?;
            checked
        }
        None => current.scripts.clone(),
    };
    let scripts_value = serde_json::to_value(&scripts)
        .map_err(|error| ApiError::internal(format!("cannot encode the scripts: {error}")))?;
    repo::update_job_template(&mut tx, id, &name, &description, &scripts_value)
        .await
        .map_err(map_write_error)?;
    let Some(updated) = repo::get_job_template(&mut tx, id).await? else {
        return Err(ApiError::internal(
            "the updated template is not found after the read",
        ));
    };
    let view = job_view(&updated)?;
    let mut event = context.audit_event(AuditAction::JobTemplateUpdate, "job_template", uuid);
    event.diff = Some(diff(
        Some(&job_template_value(&current)),
        Some(&job_template_value(&view)),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(Json(view))
}

/// Removes a job template. Only an Admin can.
#[utoipa::path(
    delete,
    path = "/api/v1/job-templates/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the template.")),
    responses(
        (status = 204, description = "The template is removed."),
        (status = 400, description = "The template is built-in.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The template is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn delete_job_template(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::ScriptsWrite)?;
    let TemplateRef::Stored(uuid) = parse_ref(&id)? else {
        return Err(ApiError::invalid_field("id", Reason::Reserved));
    };
    let id = JobTemplateId::from_uuid(uuid);
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(row) = repo::get_job_template_for_update(&mut tx, id).await? else {
        return Err(ApiError::not_found());
    };
    let current = job_view(&row)?;
    repo::delete_job_template(&mut tx, id)
        .await
        .map_err(map_write_error)?;
    let mut event = context.audit_event(AuditAction::JobTemplateDelete, "job_template", uuid);
    event.diff = Some(diff(Some(&job_template_value(&current)), None, &[]));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
