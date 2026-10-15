//! The signing secret of a webhook channel (ADR-0035, ADR-0017).
//!
//! The control plane generates the secret and stores it encrypted, like a repository secret. The
//! create response and a rotation response return it once. Nothing else returns it.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use restichub_db::{ChannelId, SecretId, TenantTx, UserId};
use restichub_secrets::{KekProvider, SecretPurpose, SecretString};

use crate::delivery::{DeliveryError, SecretReader};
use crate::error::ApiError;

/// The prefix tells a person what the value is when it shows up in a ticket.
const PREFIX: &str = "whsec_";

const SECRET_BYTES: usize = 32;

pub fn generate() -> Result<SecretString, ApiError> {
    let mut bytes = [0_u8; SECRET_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|error| ApiError::internal(format!("cannot generate a secret: {error}")))?;
    Ok(SecretString::new(format!(
        "{PREFIX}{}",
        URL_SAFE_NO_PAD.encode(bytes)
    )))
}

pub fn secret_name(channel: ChannelId) -> String {
    format!("webhook-{channel}")
}

/// Stores a new secret for the channel and returns the id of its row.
pub async fn store(
    tx: &mut TenantTx<'_>,
    provider: &dyn KekProvider,
    channel: ChannelId,
    value: &SecretString,
    created_by: Option<UserId>,
) -> Result<SecretId, ApiError> {
    crate::secrets::create_bound_secret(
        tx,
        provider,
        &secret_name(channel),
        SecretPurpose::WebhookSecret,
        value,
        created_by,
    )
    .await
}

pub async fn replace(
    tx: &mut TenantTx<'_>,
    provider: &dyn KekProvider,
    secret_id: SecretId,
    value: &SecretString,
) -> Result<(), ApiError> {
    crate::secrets::replace_bound_secret(
        tx,
        provider,
        secret_id,
        SecretPurpose::WebhookSecret,
        value,
    )
    .await
}

/// Plaintext for one send. The caller drops it after the request.
pub async fn read(
    tx: &mut TenantTx<'_>,
    provider: &dyn KekProvider,
    secret_id: SecretId,
) -> Result<SecretString, DeliveryError> {
    SecretReader::new(provider)
        .read_for_control_plane(tx, secret_id)
        .await
}
