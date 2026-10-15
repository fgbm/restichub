//! The secrets of a webhook channel (ADR-0036, ADR-0017).
//!
//! The control plane generates the signing secret and stores it encrypted, like a repository
//! secret. The create response and a rotation response return it once. Nothing else returns it.
//! The path and the query of the URL often hold a token, so they are a second encrypted secret;
//! the channel row keeps the origin only.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use restichub_db::channels::Channel;
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

fn url_secret_name(channel: ChannelId) -> String {
    format!("webhook-url-{channel}")
}

/// What a send to a webhook needs. The type has no `Debug`, so a log line cannot show a value.
pub struct WebhookCredentials {
    pub signing: SecretString,
    /// The whole URL: the origin of the channel row and the decrypted path and query.
    pub url: SecretString,
}

/// Stores the path and the query of a webhook URL as an encrypted secret.
pub async fn store_url_tail(
    tx: &mut TenantTx<'_>,
    provider: &dyn KekProvider,
    channel: ChannelId,
    path_and_query: &SecretString,
    created_by: Option<UserId>,
) -> Result<SecretId, ApiError> {
    crate::secrets::create_bound_secret(
        tx,
        provider,
        &url_secret_name(channel),
        SecretPurpose::WebhookUrl,
        path_and_query,
        created_by,
    )
    .await
}

pub async fn replace_url_tail(
    tx: &mut TenantTx<'_>,
    provider: &dyn KekProvider,
    secret_id: SecretId,
    path_and_query: &SecretString,
) -> Result<(), ApiError> {
    crate::secrets::replace_bound_secret(
        tx,
        provider,
        secret_id,
        SecretPurpose::WebhookUrl,
        path_and_query,
    )
    .await
}

/// Reads and decrypts the signing secret and the URL of a webhook channel for one send.
pub async fn read_webhook(
    tx: &mut TenantTx<'_>,
    provider: &dyn KekProvider,
    channel: &Channel,
) -> Result<WebhookCredentials, DeliveryError> {
    let (Some(signing_id), Some(tail_id), Some(origin)) = (
        channel.secret_id,
        channel.url_secret_id,
        channel.url_origin.as_deref(),
    ) else {
        return Err(DeliveryError::NotFound);
    };
    let signing = read(tx, provider, signing_id).await?;
    let tail = read(tx, provider, tail_id).await?;
    Ok(WebhookCredentials {
        signing,
        url: SecretString::new(format!("{origin}{}", tail.expose_secret())),
    })
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
