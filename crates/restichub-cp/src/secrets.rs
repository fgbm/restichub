//! The write-only secret API (FR-RP-03, FR-RP-05).
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
use restichub_db::audit::{Actor, AuditAction, REDACTED, diff};
use restichub_db::secrets::{self as repo, NewSecret};
use restichub_db::{SecretId, SecretUsageKind, UserId, kek};
use restichub_secrets::{
    KekId, KekProvider, NONCE_LEN, SecretPurpose, SecretString, WrappedDek, encrypt_secret,
    unwrap_dek,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::context::{Operation, RequestContext};
use crate::error::{ApiError, map_write_error};
use crate::json::ApiJson;
use crate::server::AppState;
use crate::validation;
use crate::validation::Reason;
use crate::views::{SecretDetailView, SecretView};

/// The type does not derive `Debug`, so a log line cannot print the value by accident. The value is
/// a [`SecretString`], so the buffer is zeroized on drop.
#[derive(Deserialize, ToSchema)]
pub struct CreateSecretRequest {
    /// Holds 1 to 64 characters from `a-z`, `0-9`, `.`, `_` and `-`.
    pub name: String,
    pub purpose: SecretPurpose,
    /// Holds 1 to 64 KiB bytes.
    #[schema(value_type = String)]
    pub value: SecretString,
}

/// The type does not derive `Debug`, so a log line cannot print the value by accident. The value is
/// a [`SecretString`], so the buffer is zeroized on drop.
#[derive(Deserialize, ToSchema)]
pub struct ReplaceSecretRequest {
    /// Holds 1 to 64 KiB bytes.
    #[schema(value_type = String)]
    pub value: SecretString,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SecretListResponse {
    /// Never holds a value.
    pub items: Vec<SecretView>,
}

struct Encrypted {
    key_version: i32,
    nonce: [u8; NONCE_LEN],
    ciphertext: Vec<u8>,
}

/// Creates an encrypted secret row and returns its identifier.
///
/// The repository create uses it to bind a credential without the audit record of the standalone
/// secrets API: the repository audit record covers the whole change.
pub(crate) async fn create_bound_secret(
    tx: &mut restichub_db::TenantTx<'_>,
    provider: &dyn KekProvider,
    name: &str,
    purpose: SecretPurpose,
    plaintext: &SecretString,
    created_by: Option<UserId>,
) -> Result<SecretId, ApiError> {
    let secret_id = SecretId::new();
    let encrypted = encrypt_for_tenant(tx, provider, secret_id, purpose, plaintext).await?;
    repo::create(
        tx,
        &NewSecret {
            id: secret_id,
            name,
            purpose: purpose.as_str(),
            key_version: encrypted.key_version,
            nonce: encrypted.nonce.as_slice(),
            ciphertext: &encrypted.ciphertext,
            created_by,
        },
    )
    .await
    .map_err(map_write_error)?;
    Ok(secret_id)
}

/// Replaces the value of a bound secret row. The caller keeps the same identifier.
pub(crate) async fn replace_bound_secret(
    tx: &mut restichub_db::TenantTx<'_>,
    provider: &dyn KekProvider,
    secret_id: SecretId,
    purpose: SecretPurpose,
    plaintext: &SecretString,
) -> Result<(), ApiError> {
    let encrypted = encrypt_for_tenant(tx, provider, secret_id, purpose, plaintext).await?;
    repo::replace(
        tx,
        secret_id,
        encrypted.key_version,
        encrypted.nonce.as_slice(),
        &encrypted.ciphertext,
    )
    .await?;
    Ok(())
}

/// Lists the metadata of the secrets of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/secrets",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The metadata of the secrets of the tenant.", body = SecretListResponse),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_secrets(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<SecretListResponse>, ApiError> {
    context.authorize(Operation::SecretsRead)?;
    let (list, mut usage) = context
        .tenant_tx(&state, async |tx| {
            Ok((repo::list(tx).await?, repo::list_usage(tx, None).await?))
        })
        .await?;
    // A reader sees only the kinds of object that the role may read.
    usage.retain(|item| {
        context
            .authorize(match item.kind {
                SecretUsageKind::Repository => Operation::RepositoriesRead,
                SecretUsageKind::Channel => Operation::ChannelsRead,
                SecretUsageKind::Profile => Operation::ProfilesRead,
                SecretUsageKind::JobTemplate => Operation::ScriptsRead,
            })
            .is_ok()
    });
    let items = list
        .iter()
        .map(|secret| SecretView::new(secret, &usage))
        .collect();
    Ok(Json(SecretListResponse { items }))
}

#[utoipa::path(
    post,
    path = "/api/v1/secrets",
    summary = "Creates a secret in the tenant.",
    description = "The response holds the metadata only. The control plane never returns the value (AC-RP-02).",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateSecretRequest,
    responses(
        (status = 201, description = "The secret is created.", body = SecretView),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 409, description = "The name exists.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_secret(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CreateSecretRequest>,
) -> Result<(StatusCode, Json<SecretView>), ApiError> {
    context.authorize(Operation::SecretsWrite)?;
    let name = validation::validate_secret_name("name", &body.name)?;
    let purpose = body.purpose;
    // The control plane makes the secrets of a webhook channel itself (ADR-0036).
    if purpose.is_internal() {
        return Err(ApiError::invalid_field("purpose", Reason::Unsupported));
    }
    validation::validate_secret_value("value", body.value.expose_secret())?;
    let provider = state
        .kek()
        .ok_or_else(|| ApiError::internal("the server holds no key encryption key provider"))?;

    // Plaintext lives only in a `SecretString`.
    let plaintext = body.value;
    // The identifier comes first: it is part of the associated data of the encryption.
    let secret_id = SecretId::new();
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let encrypted = encrypt_for_tenant(&mut tx, provider, secret_id, purpose, &plaintext).await?;
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
    .map_err(map_write_error)?;
    let mut event = context.audit_event(AuditAction::SecretCreate, "secret", secret_id.as_uuid());
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
    restichub_db::audit::insert(&mut tx, &event).await?;
    let metadata = repo::get(&mut tx, secret_id).await?;
    tx.commit().await?;
    let Some(metadata) = metadata else {
        return Err(ApiError::internal(
            "the created secret is not found after the read",
        ));
    };
    Ok((StatusCode::CREATED, Json(SecretView::new(&metadata, &[]))))
}

/// Reads the metadata of one secret of the tenant.
///
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
        crate::error::ProtectedResponses,
        (status = 404, description = "The secret is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_secret(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<SecretDetailView>, ApiError> {
    context.authorize(Operation::SecretsRead)?;
    let secret_id = parse_secret_id(&id)?;
    let metadata = context
        .tenant_tx(&state, async |tx| Ok(repo::get(tx, secret_id).await?))
        .await?;
    match metadata {
        Some(metadata) => Ok(Json(SecretDetailView::from(&metadata))),
        None => Err(ApiError::not_found()),
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
        crate::error::ProtectedResponses,
        (status = 404, description = "The secret is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn replace_secret_value(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ReplaceSecretRequest>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::SecretsWrite)?;
    let secret_id = parse_secret_id(&id)?;
    validation::validate_secret_value("value", body.value.expose_secret())?;

    let plaintext = body.value;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    // The read comes first and locks the row: a secret of another tenant is not visible and gives
    // HTTP 404, also when the key encryption key provider is absent (AC-TN-01). A concurrent
    // delete cannot remove the row between the read and the write.
    let metadata = repo::get_for_update(&mut tx, secret_id).await?;
    let Some(metadata) = metadata else {
        return Err(ApiError::not_found());
    };
    // The password of a repository cannot change until the rotation of FR-RP-06 (1.0): a new value
    // would not open the existing repository.
    if restichub_db::repositories::is_password_secret(&mut tx, secret_id).await? {
        return Err(ApiError::conflict());
    }
    let provider = state
        .kek()
        .ok_or_else(|| ApiError::internal("the server holds no key encryption key provider"))?;
    let purpose = SecretPurpose::parse(&metadata.purpose).ok_or_else(|| {
        ApiError::internal(format!("the secret {} holds an unknown purpose", secret_id))
    })?;
    if purpose.is_internal() {
        return Err(ApiError::conflict());
    }
    let encrypted = encrypt_for_tenant(&mut tx, provider, secret_id, purpose, &plaintext).await?;
    repo::replace(
        &mut tx,
        secret_id,
        encrypted.key_version,
        encrypted.nonce.as_slice(),
        &encrypted.ciphertext,
    )
    .await?;
    let mut event = context.audit_event(AuditAction::SecretReplace, "secret", secret_id.as_uuid());
    // The diff holds the marker `<redacted>` for the value. It never holds the plaintext.
    event.diff = Some(diff(None, Some(&json!({"value": REDACTED})), &[]));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Removes a secret of the tenant.
#[utoipa::path(
    delete,
    path = "/api/v1/secrets/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the secret.")),
    responses(
        (status = 204, description = "The secret is removed."),
        crate::error::ProtectedResponses,
        (status = 404, description = "The secret is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "An object uses the secret.", body = crate::error::ProblemBody)
    )
)]
pub async fn delete_secret(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::SecretsWrite)?;
    let secret_id = parse_secret_id(&id)?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let metadata = repo::get_for_update(&mut tx, secret_id).await?;
    let Some(metadata) = metadata else {
        return Err(ApiError::not_found());
    };
    // A profile names a hook secret inside its JSON, so no foreign key refuses the delete.
    if !repo::list_usage(&mut tx, Some(secret_id)).await?.is_empty() {
        return Err(ApiError::conflict());
    }
    repo::delete(&mut tx, secret_id).await.map_err(|error| {
        // A repository binds the secret, so the row cannot go. The foreign key refuses the delete.
        if error.is_foreign_key_violation() {
            ApiError::conflict()
        } else {
            ApiError::from(error)
        }
    })?;
    let mut event = context.audit_event(AuditAction::SecretDelete, "secret", secret_id.as_uuid());
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
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
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
) -> Result<Encrypted, ApiError> {
    let tenant = tx.tenant();
    let key = kek::active_key(tx)
        .await?
        .ok_or_else(ApiError::tenant_key_missing)?;
    let wrapped = wrapped_dek(&key)?;
    let dek = unwrap_dek(provider, tenant.as_uuid(), key.key_version, &wrapped)?;
    let encrypted = encrypt_secret(
        &dek,
        tenant.as_uuid(),
        secret_id.as_uuid(),
        purpose,
        plaintext,
    )?;
    Ok(Encrypted {
        key_version: key.key_version,
        nonce: encrypted.nonce,
        ciphertext: encrypted.ciphertext,
    })
}

fn wrapped_dek(key: &kek::TenantKey) -> Result<WrappedDek, ApiError> {
    let kek_id = KekId::parse(&key.kek_id)?;
    let nonce =
        key.nonce
            .as_slice()
            .try_into()
            .map_err(|error: std::array::TryFromSliceError| {
                ApiError::internal(format!("the stored DEK nonce has a wrong length: {error}"))
            })?;
    Ok(WrappedDek {
        kek_id,
        nonce,
        ciphertext: key.wrapped_dek.clone(),
    })
}

/// `None` for an API token actor.
pub(crate) fn actor_user(actor: &Actor) -> Option<UserId> {
    match actor {
        Actor::User(id) => Some(*id),
        _ => None,
    }
}

fn parse_secret_id(value: &str) -> Result<SecretId, ApiError> {
    Uuid::parse_str(value)
        .map(SecretId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause("id", Reason::InvalidFormat, error.to_string())
        })
}
