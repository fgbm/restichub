//! The typed repository settings, the secret binding and the audit diff.
//!
//! The module holds the API types of the repository endpoints and the helpers that validate the
//! backend settings and bind the secrets. The handlers live in the parent module.

use restichub_db::audit::REDACTED;
use restichub_db::repositories::{Repository, RepositorySecrets, RepositorySettings};
use restichub_db::repository_health::RepositoryHealth;
use restichub_db::{
    AgentId, BackendKind, JobStatus, RepositoryId, SecretId, agents as agents_repo,
};
use restichub_secrets::{SecretPurpose, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::dashboard::LockState;
use crate::error::ApiError;
use crate::secrets::{create_bound_secret, replace_bound_secret};
use crate::validation;
use crate::validation::Reason;

/// The backend settings of a repository (FR-RP-01).
///
/// The type is internally tagged on `kind`. Each variant carries the typed settings of its backend
/// only, so a client cannot send a setting of another backend.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackendSettings {
    /// An S3-compatible object store. The prefix is optional.
    S3 {
        endpoint: String,
        bucket: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prefix: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        region: Option<String>,
    },
    /// A rest-server. The user name and the password are optional; the user name is not a secret.
    Rest {
        endpoint: String,
        /// In an update, an absent field keeps the stored user name; `null` or an empty string
        /// clears it.
        #[serde(
            default,
            deserialize_with = "present",
            skip_serializing_if = "Option::is_none"
        )]
        #[schema(value_type = Option<String>, nullable)]
        username: Option<Option<String>>,
    },
    /// A path on the host of the agent that runs the job.
    Local { path: String },
}

/// Tells an explicit `null` (`Some(None)`) from an absent field (`None`, through `default`).
fn present<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// The write-only secret fields of a repository create.
///
/// The type does not derive `Debug`, so a log line cannot print a value by accident. Each value is
/// a [`SecretString`], so the buffer is zeroized on drop.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateRepositoryRequest {
    /// Holds 1 to 48 characters from `a-z`, `0-9`, `.`, `_` and `-`.
    pub name: String,
    pub backend: BackendSettings,
    /// Value that opens the repository. Holds 1 to 64 KiB bytes.
    #[schema(value_type = String)]
    pub password: SecretString,
    /// Credential that names the S3 account. Mandatory for the S3 backend.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub s3_access_key: Option<SecretString>,
    /// Credential that signs the S3 request. Mandatory for the S3 backend.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub s3_secret_key: Option<SecretString>,
    /// Credential of the rest-server account. Optional for the rest backend.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub rest_password: Option<SecretString>,
    /// The identifier of the maintenance agent (FR-RP-08). Required for a local repository: the
    /// path is on the disk of this host, and every job of the repository runs there.
    #[serde(default)]
    pub maintenance_agent_id: Option<String>,
}

/// The write-only secret fields of a repository update.
///
/// An absent field keeps the current value. A present value replaces it. The type does not derive
/// `Debug`, so a log line cannot print a value by accident.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateRepositoryRequest {
    #[serde(default)]
    pub name: Option<String>,
    /// The settings. The backend kind does not change; a different kind is refused.
    #[serde(default)]
    pub backend: Option<BackendSettings>,
    /// Replaces the stored value when the field is present.
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub password: Option<SecretString>,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub s3_access_key: Option<SecretString>,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub s3_secret_key: Option<SecretString>,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub rest_password: Option<SecretString>,
    /// The maintenance agent. An empty string clears the assignment. A local repository keeps a
    /// host, and the host of an initialized local repository cannot change.
    #[serde(default)]
    pub maintenance_agent_id: Option<String>,
}

/// The presence of the bound secrets. The view never holds a value (AC-RP-02).
#[derive(Debug, Serialize, ToSchema)]
pub struct CredentialsView {
    /// True when the value is set. A repository always holds one.
    pub password: bool,
    pub s3_access_key: bool,
    pub s3_secret_key: bool,
    pub rest_password: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RepositoryView {
    pub id: String,
    pub name: String,
    pub backend: BackendSettings,
    pub credentials: CredentialsView,
    /// False until an agent runs the init step. The MVP control plane never sets it true.
    pub initialized: bool,
    pub maintenance_agent_id: Option<String>,
    /// In RFC 3339.
    pub created_at: String,
    /// In RFC 3339.
    pub updated_at: String,
    /// Null for an API token actor.
    pub created_by: Option<String>,
    /// Uses the same lock marks as the dashboard (FR-SC-06).
    pub lock_state: LockState,
    /// The newest check that ran to its end. A cancelled check does not count.
    pub last_check: Option<LastCheckView>,
    /// From the newest snapshot listing of the Restore screen, kept for 7 days. Null without a
    /// listing or when the listing was cut.
    pub snapshot_count: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckResult {
    Passed,
    Failed,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LastCheckView {
    /// In RFC 3339.
    pub finished_at: String,
    pub result: CheckResult,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RepositoryListResponse {
    pub items: Vec<RepositoryView>,
}

impl RepositoryView {
    /// A repository without health facts is a new one: no lock, no check and no listing.
    pub fn new(repository: &Repository, health: Option<&RepositoryHealth>) -> Self {
        let backend = match repository.backend {
            BackendKind::S3 => BackendSettings::S3 {
                endpoint: repository.s3_endpoint.clone().unwrap_or_default(),
                bucket: repository.s3_bucket.clone().unwrap_or_default(),
                prefix: repository.s3_prefix.clone(),
                region: repository.s3_region.clone(),
            },
            BackendKind::Rest => BackendSettings::Rest {
                endpoint: repository.rest_endpoint.clone().unwrap_or_default(),
                username: repository.rest_username.clone().map(Some),
            },
            BackendKind::Local => BackendSettings::Local {
                path: repository.local_path.clone().unwrap_or_default(),
            },
        };
        Self {
            id: repository.id.to_string(),
            name: repository.name.clone(),
            backend,
            credentials: CredentialsView {
                password: true,
                s3_access_key: repository.s3_access_key_secret_id.is_some(),
                s3_secret_key: repository.s3_secret_key_secret_id.is_some(),
                rest_password: repository.rest_password_secret_id.is_some(),
            },
            initialized: repository.initialized,
            maintenance_agent_id: repository.maintenance_agent_id.map(|id| id.to_string()),
            created_at: format_time(repository.created_at),
            updated_at: format_time(repository.updated_at),
            created_by: repository.created_by.map(|id| id.to_string()),
            lock_state: match health {
                Some(health) if health.lock_stale => LockState::Stale,
                Some(health) if health.lock_held => LockState::Held,
                _ => LockState::Free,
            },
            last_check: health.and_then(|health| {
                let finished_at = health.check_finished_at?;
                Some(LastCheckView {
                    finished_at: format_time(finished_at),
                    result: if health.check_status? == JobStatus::Succeeded {
                        CheckResult::Passed
                    } else {
                        CheckResult::Failed
                    },
                })
            }),
            snapshot_count: health.and_then(|health| health.snapshot_count),
        }
    }
}

/// The typed backend settings of a repository, with owned values.
pub(super) struct ValidatedBackend {
    pub(super) kind: BackendKind,
    s3_endpoint: Option<String>,
    s3_bucket: Option<String>,
    s3_prefix: Option<String>,
    s3_region: Option<String>,
    rest_endpoint: Option<String>,
    pub(super) rest_username: Option<String>,
    local_path: Option<String>,
}

impl ValidatedBackend {
    pub(super) fn settings(&self) -> RepositorySettings<'_> {
        RepositorySettings {
            s3_endpoint: self.s3_endpoint.as_deref(),
            s3_bucket: self.s3_bucket.as_deref(),
            s3_prefix: self.s3_prefix.as_deref(),
            s3_region: self.s3_region.as_deref(),
            rest_endpoint: self.rest_endpoint.as_deref(),
            rest_username: self.rest_username.as_deref(),
            local_path: self.local_path.as_deref(),
        }
    }
}

/// Validates the settings of the backend kind. The other backend columns stay absent.
pub(super) fn validate_backend(backend: BackendSettings) -> Result<ValidatedBackend, ApiError> {
    let mut validated = ValidatedBackend {
        kind: BackendKind::S3,
        s3_endpoint: None,
        s3_bucket: None,
        s3_prefix: None,
        s3_region: None,
        rest_endpoint: None,
        rest_username: None,
        local_path: None,
    };
    match backend {
        BackendSettings::S3 {
            endpoint,
            bucket,
            prefix,
            region,
        } => {
            validated.kind = BackendKind::S3;
            validated.s3_endpoint = Some(validation::validate_endpoint(
                "backend.endpoint",
                &endpoint,
            )?);
            validated.s3_bucket = Some(validation::validate_bucket("backend.bucket", &bucket)?);
            validated.s3_prefix = validation::validate_s3_prefix(
                "backend.prefix",
                prefix.as_deref().unwrap_or_default(),
            )?;
            validated.s3_region = validate_region(region.as_deref())?;
        }
        BackendSettings::Rest { endpoint, username } => {
            validated.kind = BackendKind::Rest;
            validated.rest_endpoint = Some(validation::validate_endpoint(
                "backend.endpoint",
                &endpoint,
            )?);
            validated.rest_username = validation::validate_rest_username(
                "backend.username",
                username.flatten().as_deref().unwrap_or_default(),
            )?;
        }
        BackendSettings::Local { path } => {
            validated.kind = BackendKind::Local;
            validated.local_path = Some(validation::validate_local_path("backend.path", &path)?);
        }
    }
    Ok(validated)
}

/// Rebuilds the typed settings of a stored repository. The update reuses them when the client sends
/// no settings.
pub(super) fn validated_from_row(repository: &Repository, kind: BackendKind) -> ValidatedBackend {
    ValidatedBackend {
        kind,
        s3_endpoint: repository.s3_endpoint.clone(),
        s3_bucket: repository.s3_bucket.clone(),
        s3_prefix: repository.s3_prefix.clone(),
        s3_region: repository.s3_region.clone(),
        rest_endpoint: repository.rest_endpoint.clone(),
        rest_username: repository.rest_username.clone(),
        local_path: repository.local_path.clone(),
    }
}

fn validate_region(value: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.chars().count() > 64 || restichub_proto::validate::has_forbidden(value) {
        return Err(ApiError::invalid_field(
            "backend.region",
            Reason::InvalidFormat,
        ));
    }
    Ok(Some(value.to_owned()))
}

/// Builds the final restic location of the backend and checks it (ADR-0021).
///
/// The control plane stores the typed settings, so the location is built here to prove that the
/// settings always give a form that restic opens without an external program.
pub(super) fn repository_location(backend: &ValidatedBackend) -> Result<String, ApiError> {
    let location = match backend.kind {
        BackendKind::S3 => {
            let endpoint = backend.s3_endpoint.as_deref().unwrap_or_default();
            let bucket = backend.s3_bucket.as_deref().unwrap_or_default();
            match backend.s3_prefix.as_deref() {
                Some(prefix) => format!("s3:{endpoint}/{bucket}/{prefix}"),
                None => format!("s3:{endpoint}/{bucket}"),
            }
        }
        BackendKind::Rest => {
            format!(
                "rest:{}",
                backend.rest_endpoint.as_deref().unwrap_or_default()
            )
        }
        BackendKind::Local => backend.local_path.clone().unwrap_or_default(),
    };
    if !restichub_proto::validate::is_safe_repository_location(&location) {
        return Err(ApiError::invalid_field("backend", Reason::InvalidFormat));
    }
    Ok(location)
}

/// The restic address that the agent opens for one repository job (ADR-0021).
///
/// The job dispatcher uses it. The stored settings were validated at the create and the update, so
/// the location is the same form as [`repository_location`] gives.
pub(crate) fn stored_location(repository: &Repository) -> String {
    match repository.backend {
        BackendKind::S3 => {
            let endpoint = repository.s3_endpoint.as_deref().unwrap_or_default();
            let bucket = repository.s3_bucket.as_deref().unwrap_or_default();
            match repository.s3_prefix.as_deref() {
                Some(prefix) => format!("s3:{endpoint}/{bucket}/{prefix}"),
                None => format!("s3:{endpoint}/{bucket}"),
            }
        }
        BackendKind::Rest => format!(
            "rest:{}",
            repository.rest_endpoint.as_deref().unwrap_or_default()
        ),
        BackendKind::Local => repository.local_path.clone().unwrap_or_default(),
    }
}

/// The secret values of a create or an update, after validation.
pub(super) struct Credentials {
    s3_access_key: Option<SecretString>,
    s3_secret_key: Option<SecretString>,
    rest_password: Option<SecretString>,
}

/// Checks the secret fields against the backend kind (FR-RP-01).
pub(super) fn validate_credentials(
    kind: &BackendKind,
    s3_access_key: Option<SecretString>,
    s3_secret_key: Option<SecretString>,
    rest_password: Option<SecretString>,
) -> Result<Credentials, ApiError> {
    match kind {
        BackendKind::S3 => {
            if rest_password.is_some() {
                return Err(ApiError::invalid_field("rest_password", Reason::Unknown));
            }
            let access = s3_access_key
                .ok_or_else(|| ApiError::invalid_field("s3_access_key", Reason::Empty))?;
            let secret = s3_secret_key
                .ok_or_else(|| ApiError::invalid_field("s3_secret_key", Reason::Empty))?;
            validate_secret_value("s3_access_key", &access)?;
            validate_secret_value("s3_secret_key", &secret)?;
            Ok(Credentials {
                s3_access_key: Some(access),
                s3_secret_key: Some(secret),
                rest_password: None,
            })
        }
        BackendKind::Rest => {
            if s3_access_key.is_some() || s3_secret_key.is_some() {
                return Err(ApiError::invalid_field("s3_access_key", Reason::Unknown));
            }
            if let Some(value) = rest_password.as_ref() {
                validate_secret_value("rest_password", value)?;
            }
            Ok(Credentials {
                s3_access_key: None,
                s3_secret_key: None,
                rest_password,
            })
        }
        BackendKind::Local => {
            if s3_access_key.is_some() || s3_secret_key.is_some() || rest_password.is_some() {
                return Err(ApiError::invalid_field("backend", Reason::Unknown));
            }
            Ok(Credentials {
                s3_access_key: None,
                s3_secret_key: None,
                rest_password: None,
            })
        }
    }
}

/// Creates the secret rows of the backend credentials and returns the secret identifiers.
///
/// The secret names derive from the repository identifier, not from the name, so a rename never
/// collides with the secrets of another repository (FR-RP-01).
pub(super) async fn bind_new_credentials(
    tx: &mut restichub_db::TenantTx<'_>,
    provider: &dyn restichub_secrets::KekProvider,
    owner: &str,
    kind: &BackendKind,
    credentials: &Credentials,
    password: SecretId,
    created_by: Option<restichub_db::UserId>,
) -> Result<RepositorySecrets, ApiError> {
    let mut secrets = RepositorySecrets {
        password,
        s3_access_key: None,
        s3_secret_key: None,
        rest_password: None,
    };
    if *kind == BackendKind::S3 {
        let access = credentials
            .s3_access_key
            .as_ref()
            .ok_or_else(|| ApiError::invalid_field("s3_access_key", Reason::Empty))?;
        let secret = credentials
            .s3_secret_key
            .as_ref()
            .ok_or_else(|| ApiError::invalid_field("s3_secret_key", Reason::Empty))?;
        secrets.s3_access_key = Some(
            create_bound_secret(
                tx,
                provider,
                &format!("{owner}.s3-access-key"),
                SecretPurpose::S3AccessKey,
                access,
                created_by,
            )
            .await?,
        );
        secrets.s3_secret_key = Some(
            create_bound_secret(
                tx,
                provider,
                &format!("{owner}.s3-secret-key"),
                SecretPurpose::S3SecretKey,
                secret,
                created_by,
            )
            .await?,
        );
    }
    if *kind == BackendKind::Rest
        && let Some(value) = credentials.rest_password.as_ref()
    {
        secrets.rest_password = Some(
            create_bound_secret(
                tx,
                provider,
                &format!("{owner}.rest-password"),
                SecretPurpose::RestServerPassword,
                value,
                created_by,
            )
            .await?,
        );
    }
    Ok(secrets)
}

/// Replaces the values of the backend credentials that the update carries.
#[allow(clippy::too_many_arguments)]
pub(super) async fn replace_credentials(
    tx: &mut restichub_db::TenantTx<'_>,
    provider: &dyn restichub_secrets::KekProvider,
    owner: &str,
    kind: &BackendKind,
    secrets: &mut RepositorySecrets,
    s3_access_key: Option<SecretString>,
    s3_secret_key: Option<SecretString>,
    rest_password: Option<SecretString>,
    created_by: Option<restichub_db::UserId>,
) -> Result<(), ApiError> {
    if *kind != BackendKind::S3 && (s3_access_key.is_some() || s3_secret_key.is_some()) {
        return Err(ApiError::invalid_field("s3_access_key", Reason::Unknown));
    }
    if *kind != BackendKind::Rest && rest_password.is_some() {
        return Err(ApiError::invalid_field("rest_password", Reason::Unknown));
    }
    if let Some(value) = s3_access_key {
        validate_secret_value("s3_access_key", &value)?;
        match secrets.s3_access_key {
            Some(id) => {
                replace_bound_secret(tx, provider, id, SecretPurpose::S3AccessKey, &value).await?
            }
            None => {
                secrets.s3_access_key = Some(
                    create_bound_secret(
                        tx,
                        provider,
                        &format!("{owner}.s3-access-key"),
                        SecretPurpose::S3AccessKey,
                        &value,
                        created_by,
                    )
                    .await?,
                )
            }
        }
    }
    if let Some(value) = s3_secret_key {
        validate_secret_value("s3_secret_key", &value)?;
        match secrets.s3_secret_key {
            Some(id) => {
                replace_bound_secret(tx, provider, id, SecretPurpose::S3SecretKey, &value).await?
            }
            None => {
                secrets.s3_secret_key = Some(
                    create_bound_secret(
                        tx,
                        provider,
                        &format!("{owner}.s3-secret-key"),
                        SecretPurpose::S3SecretKey,
                        &value,
                        created_by,
                    )
                    .await?,
                )
            }
        }
    }
    if let Some(value) = rest_password {
        validate_secret_value("rest_password", &value)?;
        match secrets.rest_password {
            Some(id) => {
                replace_bound_secret(tx, provider, id, SecretPurpose::RestServerPassword, &value)
                    .await?
            }
            None => {
                secrets.rest_password = Some(
                    create_bound_secret(
                        tx,
                        provider,
                        &format!("{owner}.rest-password"),
                        SecretPurpose::RestServerPassword,
                        &value,
                        created_by,
                    )
                    .await?,
                )
            }
        }
    }
    Ok(())
}

pub(super) fn validate_secret_value(
    field: &'static str,
    value: &SecretString,
) -> Result<(), ApiError> {
    validation::validate_secret_value(field, value.expose_secret()).map_err(ApiError::from)
}

/// Anonymous secret values are not useful.
pub(super) fn parse_optional_agent_id(value: Option<&str>) -> Result<Option<AgentId>, ApiError> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) => Uuid::parse_str(value)
            .map(AgentId::from_uuid)
            .map(Some)
            .map_err(|error| {
                ApiError::invalid_field_cause(
                    "maintenance_agent_id",
                    Reason::InvalidFormat,
                    error.to_string(),
                )
            }),
    }
}

/// Refuses a maintenance agent that is not in the tenant (FR-RP-08).
pub(super) async fn ensure_agent(
    tx: &mut restichub_db::TenantTx<'_>,
    agent: Option<AgentId>,
) -> Result<(), ApiError> {
    let Some(agent) = agent else {
        return Ok(());
    };
    if agents_repo::get(tx, agent).await?.is_none() {
        return Err(ApiError::invalid_field(
            "maintenance_agent_id",
            Reason::Unknown,
        ));
    }
    Ok(())
}

/// The diff value of a repository change. It never holds a secret value.
pub(super) fn created_diff(name: &str, backend: &ValidatedBackend) -> serde_json::Value {
    json!({
        "name": name,
        "backend": backend_value(backend),
        "password": REDACTED,
        "credentials": REDACTED,
    })
}

pub(super) fn current_diff(repository: &Repository) -> serde_json::Value {
    json!({
        "name": repository.name,
        "backend": match repository.backend {
            BackendKind::S3 => json!({"kind": "s3", "endpoint": repository.s3_endpoint, "bucket": repository.s3_bucket, "prefix": repository.s3_prefix, "region": repository.s3_region}),
            BackendKind::Rest => json!({"kind": "rest", "endpoint": repository.rest_endpoint, "username": repository.rest_username}),
            BackendKind::Local => json!({"kind": "local", "path": repository.local_path}),
        },
        "maintenance_agent_id": repository.maintenance_agent_id.map(|id| id.to_string()),
        "initialized": repository.initialized,
    })
}

/// The diff of an update names the replaced secret fields.
///
/// `replaced` names the secret fields whose value changed. The list holds the field names only: a
/// secret value is never written (FR-RP-05).
pub(super) fn updated_diff(
    name: &str,
    backend: &ValidatedBackend,
    agent: Option<AgentId>,
    initialized: bool,
    replaced: &[&str],
) -> serde_json::Value {
    let mut value = json!({
        "name": name,
        "backend": backend_value(backend),
        "maintenance_agent_id": agent.map(|id| id.to_string()),
        "initialized": initialized,
    });
    if !replaced.is_empty() {
        value["replaced"] = json!(replaced);
    }
    value
}

fn backend_value(backend: &ValidatedBackend) -> serde_json::Value {
    match backend.kind {
        BackendKind::S3 => {
            json!({"kind": "s3", "endpoint": backend.s3_endpoint, "bucket": backend.s3_bucket, "prefix": backend.s3_prefix, "region": backend.s3_region})
        }
        BackendKind::Rest => {
            json!({"kind": "rest", "endpoint": backend.rest_endpoint, "username": backend.rest_username})
        }
        BackendKind::Local => json!({"kind": "local", "path": backend.local_path}),
    }
}

pub(crate) fn parse_repository_id(value: &str) -> Result<RepositoryId, ApiError> {
    Uuid::parse_str(value)
        .map(RepositoryId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause("id", Reason::InvalidFormat, error.to_string())
        })
}

fn format_time(value: time::OffsetDateTime) -> String {
    use time::format_description::well_known::Rfc3339;
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}
