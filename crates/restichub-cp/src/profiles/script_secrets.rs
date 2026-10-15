//! The check of the secrets that the scripts of a profile name (FR-HK-09, ADR-0017).

use restichub_db::{SecretId, TenantTx};
use restichub_secrets::SecretPurpose;
use uuid::Uuid;

use crate::error::ApiError;
use crate::validation::Reason;

/// Refuses a script variable that names a secret which is not a hook secret of the tenant. A
/// script receives the secret only through its environment, so it may not hand the script a
/// repository password (ADR-0017, FR-HK-09).
pub(super) async fn check_script_secrets(
    tx: &mut TenantTx<'_>,
    scripts: &[crate::scripts::ScriptDef],
) -> Result<(), ApiError> {
    for (script, definition) in scripts.iter().enumerate() {
        for (variable, entry) in definition.env.iter().enumerate() {
            if entry.kind == crate::scripts::EnvKind::SecretRef {
                let field = format!("scripts[{script}].env[{variable}].value");
                ensure_hook_secret(tx, &entry.value, field).await?;
            }
        }
    }
    Ok(())
}

/// Refuses a value that is not the identifier of a hook secret of the tenant.
async fn ensure_hook_secret(
    tx: &mut TenantTx<'_>,
    value: &str,
    field: String,
) -> Result<(), ApiError> {
    let Ok(id) = Uuid::parse_str(value) else {
        return Err(ApiError::invalid_field(field, Reason::InvalidFormat));
    };
    let metadata = restichub_db::secrets::get(tx, SecretId::from_uuid(id)).await?;
    match metadata {
        Some(metadata) if metadata.purpose == SecretPurpose::HookSecret.as_str() => Ok(()),
        _ => Err(ApiError::invalid_field(field, Reason::Unknown)),
    }
}
