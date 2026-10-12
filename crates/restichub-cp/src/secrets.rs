//! The write-only secret API (issue #17, FR-RP-03, FR-RP-05).
//!
//! The control plane stores the repository passwords and the backend credentials of the tenants.
//! The API creates a secret, replaces its value, lists the metadata and removes the secret. No
//! endpoint returns the value: the read endpoints show the metadata and the marker `set` only
//! (AC-RP-02).
//!
//! The control plane encrypts a value with AES-256-GCM and the data encryption key (DEK) of the
//! tenant. The key encryption key (KEK) wraps the DEK. The plaintext lives in a [`SecretString`]
//! only and is zeroized on drop (ADR-0017, ADR-0023).
//!
//! The service unwraps the DEK for each request. It keeps no DEK cache: a cache would hold a key in
//! memory for the lifetime of the process, and the MVP has one active key version for each tenant,
//! so the unwrapping cost is low. A later rotation adds a small cache with an explicit invalidation.
//!
//! A delivery of a secret to an agent uses [`crate::delivery::SecretReader`], not this module
//! (AC-RP-03).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::audit::{Actor, AuditAction, AuditEvent, REDACTED, diff};
use restichub_db::secrets::{self as repo, NewSecret};
use restichub_db::{SecretId, UserId, kek};
use restichub_secrets::{
    KekId, KekProvider, NONCE_LEN, SecretPurpose, SecretString, WrappedDek, encrypt_secret,
    unwrap_dek,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::client_ip::ClientIp;
use crate::context::{Operation, RequestContext};
use crate::error::ApiError;
use crate::json::ApiJson;
use crate::server::{AppState, RequestIdHeader};
use crate::validation;
use crate::views::{SecretDetailView, SecretView};

/// The body of the create-secret request.
///
/// The type does not derive `Debug`, so a log line cannot print the value by accident. The value is
/// a [`SecretString`], so the buffer is zeroized on drop.
#[derive(Deserialize, ToSchema)]
pub struct CreateSecretRequest {
    /// The name of the secret. It holds 1 to 64 characters from `a-z`, `0-9`, `.`, `_` and `-`.
    pub name: String,
    /// The purpose of the value: `repository_password`, `s3_access_key`, `s3_secret_key` or
    /// `rest_server_password`.
    pub purpose: String,
    /// The value. It holds 1 to 64 KiB bytes.
    #[schema(value_type = String)]
    pub value: SecretString,
}

/// The body of the replace-secret request.
///
/// The type does not derive `Debug`, so a log line cannot print the value by accident. The value is
/// a [`SecretString`], so the buffer is zeroized on drop.
#[derive(Deserialize, ToSchema)]
pub struct ReplaceSecretRequest {
    /// The new value. It holds 1 to 64 KiB bytes.
    #[schema(value_type = String)]
    pub value: SecretString,
}

/// A list of secret metadata.
#[derive(Debug, Serialize, ToSchema)]
pub struct SecretListResponse {
    /// The metadata of the secrets. The list never holds a value.
    pub items: Vec<SecretView>,
}

/// The outcome of the encryption of one secret value.
struct Encrypted {
    /// The version of the tenant key that encrypted the value.
    key_version: i32,
    /// The 96-bit nonce of the encryption.
    nonce: [u8; NONCE_LEN],
    /// The authenticated ciphertext of the value.
    ciphertext: Vec<u8>,
}

/// Lists the metadata of the secrets of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/secrets",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The metadata of the secrets of the tenant.", body = SecretListResponse),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody)
    )
)]
pub async fn list_secrets(
    State(state): State<AppState>,
    context: RequestContext,
    request_id: RequestIdHeader,
) -> Result<Json<SecretListResponse>, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::SecretsRead, &request_id)?;
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let list = repo::list(&mut tx)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let items = list.iter().map(SecretView::from).collect();
    Ok(Json(SecretListResponse { items }))
}

/// Creates a secret in the tenant.
///
/// The response holds the metadata only. The control plane never returns the value (AC-RP-02).
#[utoipa::path(
    post,
    path = "/api/v1/secrets",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateSecretRequest,
    responses(
        (status = 201, description = "The secret is created.", body = SecretView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody),
        (status = 409, description = "The name exists.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_secret(
    State(state): State<AppState>,
    context: RequestContext,
    client: ClientIp,
    request_id: RequestIdHeader,
    ApiJson(body): ApiJson<CreateSecretRequest>,
) -> Result<(StatusCode, Json<SecretView>), ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::SecretsWrite, &request_id)?;
    let name = validation::validate_secret_name(&body.name)
        .map_err(|_| ApiError::invalid_argument(&request_id))?;
    let purpose = SecretPurpose::parse(&body.purpose)
        .ok_or_else(|| ApiError::invalid_argument(&request_id))?;
    validation::validate_secret_value(body.value.expose_secret())
        .map_err(|_| ApiError::invalid_argument(&request_id))?;
    let provider = state
        .kek()
        .ok_or_else(|| ApiError::internal_error(&request_id))?;

    // The value lives in a `SecretString` only.
    let plaintext = body.value;
    // The identifier comes first: it is part of the associated data of the encryption.
    let secret_id = SecretId::new();
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let encrypted = encrypt_for_tenant(
        &mut tx,
        provider,
        secret_id,
        purpose,
        &plaintext,
        &request_id,
    )
    .await?;
    repo::create(
        &mut tx,
        &NewSecret {
            id: secret_id,
            name: &name,
            purpose: purpose.as_str(),
            key_version: encrypted.key_version,
            nonce: encrypted.nonce.as_slice(),
            ciphertext: &encrypted.ciphertext,
            created_by: actor_user(&context.actor),
        },
    )
    .await
    .map_err(|error| map_write_error(&request_id, &error))?;
    let mut event = AuditEvent::new(AuditAction::SecretCreate, "secret");
    event.actor = context.actor.clone();
    event.source_ip = Some(client.0);
    event.object_id = Some(secret_id.as_uuid());
    // The diff holds the marker `<redacted>` for the value. It never holds the plaintext.
    event.diff = Some(diff(
        None,
        Some(&json!({
            "name": name,
            "purpose": purpose.as_str(),
            "value": REDACTED,
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let metadata = repo::get(&mut tx, secret_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let Some(metadata) = metadata else {
        return Err(ApiError::internal_error(&request_id));
    };
    Ok((StatusCode::CREATED, Json(SecretView::from(&metadata))))
}

/// Reads the metadata of one secret of the tenant.
///
/// The response holds the metadata and the marker `set`. It never holds the value (AC-RP-02).
#[utoipa::path(
    get,
    path = "/api/v1/secrets/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the secret.")),
    responses(
        (status = 200, description = "The metadata of the secret and the marker `set`.", body = SecretDetailView),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody),
        (status = 404, description = "The secret is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_secret(
    State(state): State<AppState>,
    context: RequestContext,
    request_id: RequestIdHeader,
    Path(id): Path<String>,
) -> Result<Json<SecretDetailView>, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::SecretsRead, &request_id)?;
    let secret_id = parse_secret_id(&id, &request_id)?;
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let metadata = repo::get(&mut tx, secret_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    match metadata {
        Some(metadata) => Ok(Json(SecretDetailView::from(&metadata))),
        None => Err(ApiError::not_found(&request_id)),
    }
}

/// Replaces the value of a secret of the tenant.
#[utoipa::path(
    put,
    path = "/api/v1/secrets/{id}/value",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the secret.")),
    request_body = ReplaceSecretRequest,
    responses(
        (status = 204, description = "The value is replaced."),
        (status = 400, description = "The value is not valid.", body = crate::error::ProblemBody),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody),
        (status = 404, description = "The secret is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn replace_secret_value(
    State(state): State<AppState>,
    context: RequestContext,
    client: ClientIp,
    request_id: RequestIdHeader,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ReplaceSecretRequest>,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::SecretsWrite, &request_id)?;
    let secret_id = parse_secret_id(&id, &request_id)?;
    validation::validate_secret_value(body.value.expose_secret())
        .map_err(|_| ApiError::invalid_argument(&request_id))?;

    let plaintext = body.value;
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    // The read comes first and locks the row: a secret of another tenant is not visible and gives
    // HTTP 404, also when the key encryption key provider is absent (AC-TN-01). A concurrent
    // delete cannot remove the row between the read and the write.
    let metadata = repo::get_for_update(&mut tx, secret_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let Some(metadata) = metadata else {
        return Err(ApiError::not_found(&request_id));
    };
    let provider = state
        .kek()
        .ok_or_else(|| ApiError::internal_error(&request_id))?;
    let purpose = SecretPurpose::parse(&metadata.purpose)
        .ok_or_else(|| ApiError::internal_error(&request_id))?;
    let encrypted = encrypt_for_tenant(
        &mut tx,
        provider,
        secret_id,
        purpose,
        &plaintext,
        &request_id,
    )
    .await?;
    repo::replace(
        &mut tx,
        secret_id,
        encrypted.key_version,
        encrypted.nonce.as_slice(),
        &encrypted.ciphertext,
    )
    .await
    .map_err(|error| ApiError::internal(&request_id, &error))?;
    let mut event = AuditEvent::new(AuditAction::SecretReplace, "secret");
    event.actor = context.actor.clone();
    event.source_ip = Some(client.0);
    event.object_id = Some(secret_id.as_uuid());
    // The diff holds the marker `<redacted>` for the value. It never holds the plaintext.
    event.diff = Some(diff(None, Some(&json!({"value": REDACTED})), &[]));
    restichub_db::audit::insert(&mut tx, &event)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a secret of the tenant.
///
/// Epic #5 adds the reference check: the endpoint will return HTTP 409 when a repository references
/// the secret.
#[utoipa::path(
    delete,
    path = "/api/v1/secrets/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the secret.")),
    responses(
        (status = 204, description = "The secret is removed."),
        (status = 401, description = "The request is not authenticated.", body = crate::error::ProblemBody),
        (status = 403, description = "The role is not sufficient.", body = crate::error::ProblemBody),
        (status = 404, description = "The secret is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn delete_secret(
    State(state): State<AppState>,
    context: RequestContext,
    client: ClientIp,
    request_id: RequestIdHeader,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let request_id = request_id.0;
    context.authorize(Operation::SecretsWrite, &request_id)?;
    let secret_id = parse_secret_id(&id, &request_id)?;
    let mut tx = state
        .db()
        .begin_tenant(context.tenant_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let metadata = repo::get_for_update(&mut tx, secret_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let Some(metadata) = metadata else {
        return Err(ApiError::not_found(&request_id));
    };
    repo::delete(&mut tx, secret_id)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    let mut event = AuditEvent::new(AuditAction::SecretDelete, "secret");
    event.actor = context.actor.clone();
    event.source_ip = Some(client.0);
    event.object_id = Some(secret_id.as_uuid());
    // The diff holds the marker `<redacted>` for the value. It never holds the plaintext.
    event.diff = Some(diff(
        Some(&json!({
            "name": metadata.name,
            "purpose": metadata.purpose,
            "value": REDACTED,
        })),
        None,
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event)
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    tx.commit()
        .await
        .map_err(|error| ApiError::internal(&request_id, &error))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Encrypts a secret value with the active data encryption key (DEK) of the tenant.
///
/// The function unwraps the DEK with the key encryption key (KEK) provider for each request. The
/// service keeps no DEK cache (see the module documentation).
async fn encrypt_for_tenant(
    tx: &mut restichub_db::TenantTx<'_>,
    provider: &dyn KekProvider,
    secret_id: SecretId,
    purpose: SecretPurpose,
    plaintext: &SecretString,
    request_id: &str,
) -> Result<Encrypted, ApiError> {
    let tenant = tx.tenant();
    let key = kek::active_key(tx)
        .await
        .map_err(|error| ApiError::internal(request_id, &error))?
        .ok_or_else(|| ApiError::tenant_key_missing(request_id))?;
    let wrapped = wrapped_dek(&key, request_id)?;
    let dek = unwrap_dek(provider, tenant.as_uuid(), key.key_version, &wrapped)
        .map_err(|error| ApiError::internal(request_id, &error))?;
    let encrypted = encrypt_secret(
        &dek,
        tenant.as_uuid(),
        secret_id.as_uuid(),
        purpose,
        plaintext,
    )
    .map_err(|error| ApiError::internal(request_id, &error))?;
    Ok(Encrypted {
        key_version: key.key_version,
        nonce: encrypted.nonce,
        ciphertext: encrypted.ciphertext,
    })
}

/// Builds the wrapped data encryption key (DEK) from the stored key row.
fn wrapped_dek(key: &kek::TenantKey, request_id: &str) -> Result<WrappedDek, ApiError> {
    let kek_id =
        KekId::parse(&key.kek_id).map_err(|error| ApiError::internal(request_id, &error))?;
    let nonce = key
        .nonce
        .as_slice()
        .try_into()
        .map_err(|_| ApiError::internal_error(request_id))?;
    Ok(WrappedDek {
        kek_id,
        nonce,
        ciphertext: key.wrapped_dek.clone(),
    })
}

/// Returns the user of an actor, or `None` for an API token actor.
fn actor_user(actor: &Actor) -> Option<UserId> {
    match actor {
        Actor::User(id) => Some(*id),
        _ => None,
    }
}

/// Parses a secret identifier from a path parameter.
fn parse_secret_id(value: &str, request_id: &str) -> Result<SecretId, ApiError> {
    Uuid::parse_str(value)
        .map(SecretId::from_uuid)
        .map_err(|_| ApiError::invalid_argument(request_id))
}

/// Maps a write error to the API error. A duplicate name is a conflict.
fn map_write_error(request_id: &str, error: &restichub_db::DbError) -> ApiError {
    if error.is_unique_violation() {
        ApiError::conflict(request_id)
    } else {
        ApiError::internal(request_id, error)
    }
}
