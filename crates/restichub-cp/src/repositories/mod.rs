//! The repository endpoints (FR-RP-01, FR-RP-05, FR-RP-08, ADR-0003).
//!
//! An Admin creates, updates and deletes the repositories of its tenant. A Viewer reads. A
//! repository of another tenant gives HTTP 404 (AC-TN-01). The platform tenant gets HTTP 403.
//!
//! The backend is a typed closed set. Each kind carries its own typed settings; there is no
//! free-form map (FR-PR-01).
//!
//! The repository holds the identifiers of the bound secrets only. The create and the update take
//! the secret values as write-only fields and encrypt them with the data encryption key of the
//! tenant (ADR-0017). No endpoint returns a value (AC-RP-02).
//!
//! The control plane runs no init, test-access, forget, prune or check step here: they need the
//! agent (see the epic issue). The repository therefore starts in the `not initialized` state.

mod settings;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::RepositoryId;
use restichub_db::audit::{AuditAction, diff};
use restichub_db::repositories::{
    self as repo, NewRepository, RepositorySecrets, RepositoryUpdate,
};
use restichub_secrets::SecretPurpose;

use crate::context::{Operation, RequestContext};
use crate::error::{ApiError, map_write_error};
use crate::json::ApiJson;
use crate::secrets::{actor_user, create_bound_secret};
use crate::server::AppState;
use crate::validation;
use crate::validation::Reason;

pub use settings::{
    BackendSettings, CreateRepositoryRequest, CredentialsView, RepositoryListResponse,
    RepositoryView, UpdateRepositoryRequest,
};
use settings::{
    bind_new_credentials, created_diff, current_diff, ensure_agent, parse_optional_agent_id,
    parse_repository_id, replace_credentials, repository_location, updated_diff, validate_backend,
    validate_credentials, validate_secret_value, validated_from_row,
};

/// Lists the repositories of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/repositories",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The repositories of the tenant.", body = RepositoryListResponse),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_repositories(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<RepositoryListResponse>, ApiError> {
    context.authorize(Operation::RepositoriesRead)?;
    let list = context
        .tenant_tx(&state, async |tx| Ok(repo::list(tx).await?))
        .await?;
    Ok(Json(RepositoryListResponse {
        items: list.iter().map(RepositoryView::from).collect(),
    }))
}

/// Reads one repository of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/repositories/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository.")),
    responses(
        (status = 200, description = "The repository.", body = RepositoryView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_repository(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<RepositoryView>, ApiError> {
    context.authorize(Operation::RepositoriesRead)?;
    let repository_id = parse_repository_id(&id)?;
    let repository = context
        .tenant_tx(&state, async |tx| Ok(repo::get(tx, repository_id).await?))
        .await?;
    match repository {
        Some(repository) => Ok(Json(RepositoryView::from(&repository))),
        None => Err(ApiError::not_found()),
    }
}

/// Creates a repository and binds its secrets (FR-RP-01, FR-RP-05).
#[utoipa::path(
    post,
    path = "/api/v1/repositories",
    summary = "Creates a repository in the tenant.",
    description = "The control plane encrypts the secret values and never returns them (AC-RP-02).",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateRepositoryRequest,
    responses(
        (status = 201, description = "The repository is created.", body = RepositoryView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 409, description = "The name exists.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_repository(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CreateRepositoryRequest>,
) -> Result<(StatusCode, Json<RepositoryView>), ApiError> {
    context.authorize(Operation::RepositoriesWrite)?;
    let CreateRepositoryRequest {
        name,
        backend,
        password,
        s3_access_key,
        s3_secret_key,
        rest_password,
        maintenance_agent_id,
    } = body;
    let name = validation::validate_repository_name("name", &name)?;
    let backend = validate_backend(backend)?;
    repository_location(&backend)?;
    validate_secret_value("password", &password)?;
    let credentials =
        validate_credentials(&backend.kind, s3_access_key, s3_secret_key, rest_password)?;
    let maintenance_agent_id = parse_optional_agent_id(maintenance_agent_id.as_deref())?;
    let created_by = actor_user(&context.actor);

    let provider = state
        .kek()
        .ok_or_else(|| ApiError::internal("the server holds no key encryption key provider"))?;
    let repository_id = RepositoryId::new();
    let owner = repository_id.to_string();
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    ensure_agent(&mut tx, maintenance_agent_id).await?;

    let password_secret = create_bound_secret(
        &mut tx,
        provider,
        &format!("{owner}.password"),
        SecretPurpose::RepositoryPassword,
        &password,
        created_by,
    )
    .await?;
    let secrets = bind_new_credentials(
        &mut tx,
        provider,
        &owner,
        &backend.kind,
        &credentials,
        password_secret,
        created_by,
    )
    .await?;

    repo::create(
        &mut tx,
        &NewRepository {
            id: repository_id,
            name: &name,
            backend: backend.kind,
            settings: backend.settings(),
            secrets,
            maintenance_agent_id,
            created_by,
        },
    )
    .await
    .map_err(map_write_error)?;

    let mut event = context.audit_event(
        AuditAction::RepositoryCreate,
        "repository",
        repository_id.as_uuid(),
    );
    // The audit record diff never holds a secret value.
    event.diff = Some(diff(None, Some(&created_diff(&name, &backend)), &[]));
    restichub_db::audit::insert(&mut tx, &event).await?;

    let created = repo::get(&mut tx, repository_id).await?;
    tx.commit().await?;
    let Some(created) = created else {
        return Err(ApiError::internal(
            "the created repository is not found after the read",
        ));
    };
    Ok((StatusCode::CREATED, Json(RepositoryView::from(&created))))
}

/// Updates a repository and replaces the secret values that it carries.
#[utoipa::path(
    patch,
    path = "/api/v1/repositories/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository.")),
    request_body = UpdateRepositoryRequest,
    responses(
        (status = 200, description = "The updated repository.", body = RepositoryView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn update_repository(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateRepositoryRequest>,
) -> Result<Json<RepositoryView>, ApiError> {
    context.authorize(Operation::RepositoriesWrite)?;
    let repository_id = parse_repository_id(&id)?;
    let UpdateRepositoryRequest {
        name,
        backend,
        password,
        s3_access_key,
        s3_secret_key,
        rest_password,
        maintenance_agent_id,
    } = body;
    // The repository password cannot be replaced until the rotation of FR-RP-06 (1.0): a new
    // password would not open the existing repository.
    if password.is_some() {
        return Err(ApiError::conflict());
    }

    let provider = state
        .kek()
        .ok_or_else(|| ApiError::internal("the server holds no key encryption key provider"))?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    // The read locks the row: a concurrent delete cannot remove it between the read and the write.
    let Some(current) = repo::get_for_update(&mut tx, repository_id).await? else {
        return Err(ApiError::not_found());
    };

    let name = match name {
        Some(value) => validation::validate_repository_name("name", &value)?,
        None => current.name.clone(),
    };
    let backend = match backend {
        Some(settings) => {
            let validated = validate_backend(settings)?;
            if validated.kind != current.backend {
                return Err(ApiError::invalid_field("backend", Reason::Unknown));
            }
            validated
        }
        None => {
            let kind = current.backend;
            validated_from_row(&current, kind)
        }
    };
    repository_location(&backend)?;
    let agent_set = match maintenance_agent_id {
        Some(value) => Some(parse_optional_agent_id(Some(&value))?),
        None => None,
    };
    let agent = match agent_set {
        Some(agent) => agent,
        None => current.maintenance_agent_id,
    };
    ensure_agent(&mut tx, agent).await?;

    let mut secrets = RepositorySecrets {
        password: current.password_secret_id,
        s3_access_key: current.s3_access_key_secret_id,
        s3_secret_key: current.s3_secret_key_secret_id,
        rest_password: current.rest_password_secret_id,
    };
    let mut replaced: Vec<&str> = Vec::new();
    if s3_access_key.is_some() {
        replaced.push("s3_access_key");
    }
    if s3_secret_key.is_some() {
        replaced.push("s3_secret_key");
    }
    if rest_password.is_some() {
        replaced.push("rest_password");
    }
    let owner = repository_id.to_string();
    replace_credentials(
        &mut tx,
        provider,
        &owner,
        &backend.kind,
        &mut secrets,
        s3_access_key,
        s3_secret_key,
        rest_password,
        actor_user(&context.actor),
    )
    .await?;

    repo::update(
        &mut tx,
        repository_id,
        &RepositoryUpdate {
            name: &name,
            settings: backend.settings(),
            secrets,
            maintenance_agent_id: agent,
        },
    )
    .await
    .map_err(map_write_error)?;

    let mut event = context.audit_event(
        AuditAction::RepositoryUpdate,
        "repository",
        repository_id.as_uuid(),
    );
    // The diff names the replaced secret fields only. It never holds a secret value.
    event.diff = Some(diff(
        Some(&current_diff(&current)),
        Some(&updated_diff(
            &name,
            &backend,
            agent,
            current.initialized,
            &replaced,
        )),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;

    let updated = repo::get(&mut tx, repository_id).await?;
    tx.commit().await?;
    let Some(updated) = updated else {
        return Err(ApiError::internal(
            "the updated repository is not found after the read",
        ));
    };
    Ok(Json(RepositoryView::from(&updated)))
}

/// Removes a repository and its bound secrets.
#[utoipa::path(
    delete,
    path = "/api/v1/repositories/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the repository.")),
    responses(
        (status = 204, description = "The repository is removed."),
        crate::error::ProtectedResponses,
        (status = 404, description = "The repository is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn delete_repository(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::RepositoriesWrite)?;
    let repository_id = parse_repository_id(&id)?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(current) = repo::get_for_update(&mut tx, repository_id).await? else {
        return Err(ApiError::not_found());
    };
    // A profile binds the repository. Remove or repoint the profiles first, so the foreign key
    // never blocks the delete with an internal error.
    if restichub_db::profiles::is_repository_used(&mut tx, repository_id).await? {
        return Err(ApiError::conflict());
    }
    repo::delete(&mut tx, repository_id).await?;
    // The bound secrets belong to the repository. Remove them in the same transaction, so the
    // secrets screen does not keep an orphan and the foreign key never blocks the delete.
    restichub_db::secrets::delete(&mut tx, current.password_secret_id).await?;
    for secret in [
        current.s3_access_key_secret_id,
        current.s3_secret_key_secret_id,
        current.rest_password_secret_id,
    ]
    .into_iter()
    .flatten()
    {
        restichub_db::secrets::delete(&mut tx, secret).await?;
    }
    let mut event = context.audit_event(
        AuditAction::RepositoryDelete,
        "repository",
        repository_id.as_uuid(),
    );
    event.diff = Some(diff(Some(&current_diff(&current)), None, &[]));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
