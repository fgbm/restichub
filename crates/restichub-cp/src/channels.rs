//! The notification channel endpoints (FR-MN-04, AC-MN-01, ADR-0036, ADR-0017).
//!
//! An Admin manages the channels of the tenant. A channel is an email recipient list or a webhook.
//! The control plane generates the signing secret of a webhook and returns it once, in the create
//! response and in a rotation response. No other response, audit record or log line holds it. The
//! list shows the origin of a webhook URL only, because the path of such a URL often is a secret.
//!
//! A channel of another tenant gives HTTP 404 (AC-TN-01). The platform tenant refuses every channel
//! operation with HTTP 403.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use restichub_db::audit::{AuditAction, REDACTED, diff};
use restichub_db::channels::{self as repo, Channel, ChannelKind, MAX_CHANNELS};
use restichub_db::{ChannelId, DeliveryId};
use restichub_secrets::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use utoipa::ToSchema;

use crate::alerts::deliver::send_message;
use crate::alerts::message::{Message, MessageEvent};
use crate::alerts::secret;
use crate::alerts::ssrf::{UrlError, parse_target};
use crate::context::{Operation, RequestContext};
use crate::error::{ApiError, map_write_error};
use crate::json::ApiJson;
use crate::server::AppState;
use crate::validation::{self, Reason};

/// The largest number of recipients of an email channel.
const MAX_RECIPIENTS: usize = 20;

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateChannelRequest {
    /// Free text of 1 to 100 characters. Unique in the tenant, ignoring the letter case. Holds no control or format character.
    #[schema(max_length = 100)]
    pub name: String,
    pub kind: ChannelKind,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Required for an email channel: 1 to 20 addresses.
    #[serde(default)]
    pub recipients: Vec<String>,
    /// Required for a webhook; `https` unless the installation allows `http`.
    #[serde(default)]
    pub url: Option<String>,
}

fn default_enabled() -> bool {
    true
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateChannelRequest {
    /// Free text of 1 to 100 characters. Unique in the tenant, ignoring the letter case. Holds no control or format character.
    #[schema(max_length = 100)]
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    /// The full new address list of an email channel.
    #[serde(default)]
    pub recipients: Option<Vec<String>>,
    /// The full new target of a webhook. The path and the query stay write-only: the control plane
    /// stores them encrypted.
    #[serde(default)]
    pub url: Option<String>,
    /// Generates a new signing secret for a webhook. The response returns it once.
    #[serde(default)]
    pub rotate_secret: Option<bool>,
}

#[derive(Serialize, ToSchema)]
pub struct ChannelView {
    pub id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub enabled: bool,
    /// The addresses of an email channel. Empty for a webhook.
    pub recipients: Vec<String>,
    /// The scheme, the host and the port of a webhook URL. The path and the query are write-only.
    pub url_origin: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// The end of the last delivery attempt of an alert, in RFC 3339. A test send does not count.
    pub last_delivery_at: Option<String>,
    /// Null when no alert delivery was attempted. False also for an attempt that waits for a retry.
    pub last_delivery_ok: Option<bool>,
}

impl From<&Channel> for ChannelView {
    fn from(channel: &Channel) -> Self {
        Self {
            id: channel.id.to_string(),
            name: channel.name.clone(),
            kind: channel.kind,
            enabled: channel.enabled,
            recipients: channel.recipients.clone(),
            url_origin: channel.url_origin.clone(),
            created_at: format_time(channel.created_at),
            updated_at: format_time(channel.updated_at),
            last_delivery_at: channel.last_delivery_at.map(format_time),
            last_delivery_ok: channel.last_delivery_ok,
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct ChannelListResponse {
    pub items: Vec<ChannelView>,
    /// False when the installation has no SMTP server, so an email channel cannot send.
    pub email_configured: bool,
    /// True when the installation allows a webhook URL with the scheme `http`.
    pub webhook_allow_http: bool,
}

#[derive(Serialize, ToSchema)]
pub struct ChannelResponse {
    pub channel: ChannelView,
    /// Set in the create response of a webhook and after a rotation. It is shown once.
    pub signing_secret: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct TestResponse {
    pub ok: bool,
    /// A short code such as `blocked_address` or `http_500`. Absent when the send worked.
    pub error: Option<String>,
}

/// Lists the channels of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/notification-channels",
    security(("cookie" = []), ("bearer" = [])),
    responses(
        (status = 200, description = "The channels of the tenant.", body = ChannelListResponse),
        crate::error::ProtectedResponses,
    )
)]
pub async fn list_channels(
    State(state): State<AppState>,
    context: RequestContext,
) -> Result<Json<ChannelListResponse>, ApiError> {
    context.authorize(Operation::ChannelsRead)?;
    let channels = context
        .tenant_tx(&state, async |tx| Ok(repo::list(tx).await?))
        .await?;
    let settings = state.sender().map(|sender| sender.settings());
    Ok(Json(ChannelListResponse {
        items: channels.iter().map(ChannelView::from).collect(),
        email_configured: settings.is_some_and(|settings| settings.smtp.is_some()),
        webhook_allow_http: settings.is_some_and(|settings| settings.webhook.allow_http),
    }))
}

/// Reads one channel of the tenant.
#[utoipa::path(
    get,
    path = "/api/v1/notification-channels/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the channel.")),
    responses(
        (status = 200, description = "The channel.", body = ChannelView),
        crate::error::ProtectedResponses,
        (status = 404, description = "The channel is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn get_channel(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<ChannelView>, ApiError> {
    context.authorize(Operation::ChannelsRead)?;
    let channel_id = parse_channel_id(&id)?;
    let channel = context
        .tenant_tx(&state, async |tx| Ok(repo::get(tx, channel_id).await?))
        .await?;
    channel
        .as_ref()
        .map(|channel| Json(ChannelView::from(channel)))
        .ok_or_else(ApiError::not_found)
}

/// Creates a channel. A webhook gets a generated signing secret that the response shows once.
#[utoipa::path(
    post,
    path = "/api/v1/notification-channels",
    security(("cookie" = []), ("bearer" = [])),
    request_body = CreateChannelRequest,
    responses(
        (status = 201, description = "The channel is created.", body = ChannelResponse),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 409, description = "The name exists, or the tenant holds the most channels.", body = crate::error::ProblemBody)
    )
)]
pub async fn create_channel(
    State(state): State<AppState>,
    context: RequestContext,
    ApiJson(body): ApiJson<CreateChannelRequest>,
) -> Result<(StatusCode, Json<ChannelResponse>), ApiError> {
    context.authorize(Operation::ChannelsWrite)?;
    let name = crate::validation::validate_record_name("name", &body.name)?;
    let (recipients, url) = match body.kind {
        ChannelKind::Email => {
            if body.url.is_some() {
                return Err(ApiError::invalid_field("url", Reason::Unsupported));
            }
            (validate_recipients(&body.recipients)?, None)
        }
        ChannelKind::Webhook => {
            if !body.recipients.is_empty() {
                return Err(ApiError::invalid_field("recipients", Reason::Unsupported));
            }
            let url = body
                .url
                .as_deref()
                .ok_or_else(|| ApiError::invalid_field("url", Reason::Empty))?;
            (Vec::new(), Some(split_url(&validate_url(&state, url)?)))
        }
    };

    let channel_id = ChannelId::new();
    let created_by = crate::secrets::actor_user(&context.actor);
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    if repo::count(&mut tx).await? >= MAX_CHANNELS {
        return Err(ApiError::conflict());
    }
    let mut signing_secret = None;
    let mut secret_id = None;
    let mut url_secret_id = None;
    if let Some((_, tail)) = &url {
        let provider = state
            .kek()
            .ok_or_else(|| ApiError::internal("the server holds no key encryption key provider"))?;
        let value = secret::generate()?;
        secret_id = Some(secret::store(&mut tx, provider, channel_id, &value, created_by).await?);
        url_secret_id =
            Some(secret::store_url_tail(&mut tx, provider, channel_id, tail, created_by).await?);
        signing_secret = Some(value);
    }
    let url_origin = url.as_ref().map(|(origin, _)| origin.as_str());
    repo::create(
        &mut tx,
        &repo::NewChannel {
            id: channel_id,
            name: &name,
            kind: body.kind,
            enabled: body.enabled,
            recipients: &recipients,
            url_origin,
            secret_id,
            url_secret_id,
            created_by,
        },
    )
    .await
    .map_err(map_write_error)?;

    let mut event =
        context.audit_event(AuditAction::ChannelCreate, "channel", channel_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&audit_value(
            &name,
            body.kind,
            body.enabled,
            &recipients,
            url_origin,
        )),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    let created = repo::get(&mut tx, channel_id).await?;
    tx.commit().await?;
    let Some(created) = created else {
        return Err(ApiError::internal(
            "the created channel is not found after the read",
        ));
    };
    Ok((
        StatusCode::CREATED,
        Json(ChannelResponse {
            channel: ChannelView::from(&created),
            signing_secret: signing_secret.map(|value| value.expose_secret().to_owned()),
        }),
    ))
}

/// Updates a channel. An absent field keeps its value.
#[utoipa::path(
    patch,
    path = "/api/v1/notification-channels/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the channel.")),
    request_body = UpdateChannelRequest,
    responses(
        (status = 200, description = "The updated channel.", body = ChannelResponse),
        (status = 400, description = "A field is not valid.", body = crate::error::ProblemBody),
        crate::error::ProtectedResponses,
        (status = 404, description = "The channel is not found.", body = crate::error::ProblemBody),
        (status = 409, description = "The name exists.", body = crate::error::ProblemBody)
    )
)]
pub async fn update_channel(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateChannelRequest>,
) -> Result<Json<ChannelResponse>, ApiError> {
    context.authorize(Operation::ChannelsWrite)?;
    let channel_id = parse_channel_id(&id)?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(current) = repo::get_for_update(&mut tx, channel_id).await? else {
        return Err(ApiError::not_found());
    };

    let name = match &body.name {
        Some(name) => crate::validation::validate_record_name("name", name)?,
        None => current.name.clone(),
    };
    let enabled = body.enabled.unwrap_or(current.enabled);
    let (recipients, url) = match current.kind {
        ChannelKind::Email => {
            if body.url.is_some() {
                return Err(ApiError::invalid_field("url", Reason::Unsupported));
            }
            if body.rotate_secret == Some(true) {
                return Err(ApiError::invalid_field(
                    "rotate_secret",
                    Reason::Unsupported,
                ));
            }
            let recipients = match &body.recipients {
                Some(list) => validate_recipients(list)?,
                None => current.recipients.clone(),
            };
            (recipients, None)
        }
        ChannelKind::Webhook => {
            if body.recipients.is_some() {
                return Err(ApiError::invalid_field("recipients", Reason::Unsupported));
            }
            let new_url = match &body.url {
                Some(url) => Some(split_url(&validate_url(&state, url)?)),
                None => None,
            };
            (Vec::new(), new_url)
        }
    };
    let url_origin = match &url {
        Some((origin, _)) => Some(origin.clone()),
        None => current.url_origin.clone(),
    };

    repo::update(
        &mut tx,
        channel_id,
        &repo::ChannelUpdate {
            name: &name,
            enabled,
            recipients: &recipients,
            url_origin: url_origin.as_deref(),
        },
    )
    .await
    .map_err(map_write_error)?;

    if let (Some((_, tail)), Some(url_secret_id)) = (&url, current.url_secret_id) {
        let provider = state
            .kek()
            .ok_or_else(|| ApiError::internal("the server holds no key encryption key provider"))?;
        secret::replace_url_tail(&mut tx, provider, url_secret_id, tail).await?;
    }

    let mut signing_secret = None;
    if body.rotate_secret == Some(true)
        && let Some(secret_id) = current.secret_id
    {
        let provider = state
            .kek()
            .ok_or_else(|| ApiError::internal("the server holds no key encryption key provider"))?;
        let value = secret::generate()?;
        secret::replace(&mut tx, provider, secret_id, &value).await?;
        signing_secret = Some(value);
    }

    let mut after = audit_value(
        &name,
        current.kind,
        enabled,
        &recipients,
        url_origin.as_deref(),
    );
    if signing_secret.is_some() {
        after["signing_secret"] = json!(REDACTED);
    }
    let mut event =
        context.audit_event(AuditAction::ChannelUpdate, "channel", channel_id.as_uuid());
    event.diff = Some(diff(
        Some(&audit_value(
            &current.name,
            current.kind,
            current.enabled,
            &current.recipients,
            current.url_origin.as_deref(),
        )),
        Some(&after),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    let updated = repo::get(&mut tx, channel_id).await?;
    tx.commit().await?;
    let Some(updated) = updated else {
        return Err(ApiError::internal(
            "the updated channel is not found after the read",
        ));
    };
    Ok(Json(ChannelResponse {
        channel: ChannelView::from(&updated),
        signing_secret: signing_secret.map(|value| value.expose_secret().to_owned()),
    }))
}

/// Removes a channel. Its pending messages go with it.
#[utoipa::path(
    delete,
    path = "/api/v1/notification-channels/{id}",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the channel.")),
    responses(
        (status = 204, description = "The channel is removed."),
        crate::error::ProtectedResponses,
        (status = 404, description = "The channel is not found.", body = crate::error::ProblemBody)
    )
)]
pub async fn delete_channel(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    context.authorize(Operation::ChannelsWrite)?;
    let channel_id = parse_channel_id(&id)?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let Some(current) = repo::get_for_update(&mut tx, channel_id).await? else {
        return Err(ApiError::not_found());
    };
    repo::delete(&mut tx, channel_id).await?;
    // The secret row goes with the channel, so no key outlives its channel.
    for secret_id in [current.secret_id, current.url_secret_id]
        .into_iter()
        .flatten()
    {
        restichub_db::secrets::delete(&mut tx, secret_id).await?;
    }
    let mut event =
        context.audit_event(AuditAction::ChannelDelete, "channel", channel_id.as_uuid());
    event.diff = Some(diff(
        Some(&audit_value(
            &current.name,
            current.kind,
            current.enabled,
            &current.recipients,
            current.url_origin.as_deref(),
        )),
        None,
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sends a test message through the channel and reports the result.
#[utoipa::path(
    post,
    path = "/api/v1/notification-channels/{id}/test",
    security(("cookie" = []), ("bearer" = [])),
    params(("id" = String, Path, description = "The identifier of the channel.")),
    responses(
        (status = 200, description = "The result of the send.", body = TestResponse),
        crate::error::ProtectedResponses,
        (status = 404, description = "The channel is not found.", body = crate::error::ProblemBody),
        (status = 429, description = "The actor or the tenant sent too many tests.", body = crate::error::ProblemBody),
        (status = 503, description = "The server runs the most tests that it allows.", body = crate::error::ProblemBody)
    )
)]
pub async fn test_channel(
    State(state): State<AppState>,
    context: RequestContext,
    Path(id): Path<String>,
) -> Result<Json<TestResponse>, ApiError> {
    context.authorize(Operation::ChannelsWrite)?;
    let channel_id = parse_channel_id(&id)?;
    check_test_limit(&state, &context)?;
    let _slot = state
        .auth()
        .test_send_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|error| {
            tracing::debug!(%error, "no free slot for a test send");
            ApiError::busy(5)
        })?;
    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    // The read comes first: a channel of another tenant gives HTTP 404 (AC-TN-01).
    let Some(channel) = repo::get(&mut tx, channel_id).await? else {
        return Err(ApiError::not_found());
    };
    let sender = state
        .sender()
        .ok_or_else(|| ApiError::internal("the server holds no alert sender"))?;
    let provider = state
        .kek()
        .ok_or_else(|| ApiError::internal("the server holds no key encryption key provider"))?;
    let tenant_name = restichub_db::tenants::get(&mut tx, context.tenant_id)
        .await?
        .map(|tenant| tenant.name)
        .unwrap_or_default();
    let credentials = match channel.kind {
        ChannelKind::Webhook => secret::read_webhook(&mut tx, provider, &channel).await.ok(),
        ChannelKind::Email => None,
    };
    tx.commit().await?;

    let message = Message {
        delivery_id: DeliveryId::new(),
        event: MessageEvent::Test,
        sent_at: state.auth().clock.now(),
        tenant_id: context.tenant_id,
        tenant_name,
        alert: None,
        public_origin: sender.settings().public_origin.clone(),
        language: sender.settings().language,
    };
    let result = send_message(sender, &channel, credentials.as_ref(), &message).await;

    let mut tx = state.db().begin_tenant(context.tenant_id).await?;
    let mut event = context.audit_event(AuditAction::AlertSent, "channel", channel_id.as_uuid());
    event.diff = Some(diff(
        None,
        Some(&json!({
            "channel_id": channel_id.to_string(),
            "channel_kind": channel.kind.as_str(),
            "event": MessageEvent::Test.as_str(),
            "ok": result.is_ok(),
        })),
        &[],
    ));
    restichub_db::audit::insert(&mut tx, &event).await?;
    tx.commit().await?;

    Ok(Json(TestResponse {
        ok: result.is_ok(),
        error: result.err().map(|error| error.code()),
    }))
}

/// A test connects to a target that the tenant chose, so the actor and the tenant have a bucket.
fn check_test_limit(state: &AppState, context: &RequestContext) -> Result<(), ApiError> {
    let actor = crate::rate_limit::actor_label(&context.actor);
    let now = std::time::Instant::now();
    let key = crate::rate_limit::test_send_key(context.tenant_id, &actor);
    if let Err(wait) = state.auth().test_send_limiter.check(&key, now) {
        return Err(ApiError::rate_limited(wait.as_secs().max(1)));
    }
    let tenant_key = crate::rate_limit::test_send_tenant_key(context.tenant_id);
    if let Err(wait) = state
        .auth()
        .test_send_tenant_limiter
        .check(&tenant_key, now)
    {
        return Err(ApiError::rate_limited(wait.as_secs().max(1)));
    }
    Ok(())
}

/// Trims, lowercases and checks the addresses, and removes duplicates.
fn validate_recipients(list: &[String]) -> Result<Vec<String>, ApiError> {
    if list.is_empty() {
        return Err(ApiError::invalid_field("recipients", Reason::Empty));
    }
    if list.len() > MAX_RECIPIENTS {
        return Err(ApiError::invalid_field("recipients", Reason::TooLong));
    }
    let mut out: Vec<String> = Vec::with_capacity(list.len());
    for address in list {
        let address = validation::validate_email("recipients", address)?;
        // The characters below could change the envelope of the SMTP command.
        if address.contains(['<', '>', ',', ';', '"']) {
            return Err(ApiError::invalid_field(
                "recipients",
                Reason::InvalidCharacter,
            ));
        }
        if !out.contains(&address) {
            out.push(address);
        }
    }
    Ok(out)
}

fn validate_url(state: &AppState, url: &str) -> Result<String, ApiError> {
    let policy = state
        .sender()
        .map(|sender| sender.settings().webhook.clone())
        .unwrap_or_default();
    let url = url.trim();
    parse_target(url, &policy).map_err(|error| {
        ApiError::invalid_field(
            "url",
            match error {
                UrlError::Scheme => Reason::Unsupported,
                UrlError::Forbidden => Reason::Reserved,
                UrlError::Invalid | UrlError::Parts => Reason::InvalidFormat,
            },
        )
    })?;
    Ok(url.to_owned())
}

/// Splits a checked URL into the origin and the path with the query. The origin stays in the
/// channel row; the control plane stores the rest encrypted.
fn split_url(url: &str) -> (String, SecretString) {
    let uri = url.parse::<axum::http::Uri>().ok();
    let origin = uri
        .as_ref()
        .and_then(|uri| {
            Some(format!(
                "{}://{}",
                uri.scheme_str()?,
                uri.authority()?.as_str()
            ))
        })
        .unwrap_or_default();
    let tail = uri
        .as_ref()
        .and_then(|uri| uri.path_and_query().map(|value| value.as_str().to_owned()))
        .filter(|value| value.starts_with('/'))
        .unwrap_or_else(|| "/".to_owned());
    (origin, SecretString::new(tail))
}

/// A webhook shows its origin, never the path or the query.
fn audit_value(
    name: &str,
    kind: ChannelKind,
    enabled: bool,
    recipients: &[String],
    url_origin: Option<&str>,
) -> serde_json::Value {
    json!({
        "name": name,
        "kind": kind.as_str(),
        "enabled": enabled,
        "recipients": recipients,
        "url_origin": url_origin,
    })
}

fn parse_channel_id(value: &str) -> Result<ChannelId, ApiError> {
    uuid::Uuid::parse_str(value.trim())
        .map(ChannelId::from_uuid)
        .map_err(|error| {
            ApiError::invalid_field_cause("id", Reason::InvalidFormat, error.to_string())
        })
}

fn format_time(value: OffsetDateTime) -> String {
    use time::format_description::well_known::Rfc3339;
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}
