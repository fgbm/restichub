//! The delivery of the queued alert messages (FR-MN-04, ADR-0036).
//!
//! A pass claims the due rows of the outbox in one transaction, then handles each row in turn: in
//! one transaction it counts the attempt, extends the lease and reads the channel again, then it
//! sends with no transaction open and records the result with the number of its attempt. The claim is a lease, so two instances never send one row, and a crash
//! repeats the send after the lease ends. A holder whose lease ended and whose row another sender
//! claimed since changes nothing.

use restichub_db::alerts::{self, ClaimedDelivery};
use restichub_db::audit::{AuditAction, AuditEvent, diff};
use restichub_db::channels::{self, Channel, ChannelKind};
use restichub_db::{ChannelId, Database, DbError, TenantId, TenantTx};
use restichub_secrets::KekProvider;
use serde_json::json;
use time::Duration;

use crate::clock::Clock;

use super::email;
use super::message::{Message, MessageEvent};
use super::secret::{self, WebhookCredentials};
use super::sender::Sender;
use super::transport::SendError;
use super::webhook;

/// Lease length: a crashed sender leaves the row due again after this time. The pass extends it
/// before each send, so it only has to outlast one send; the longest send takes 30 seconds.
const LEASE: Duration = Duration::minutes(2);

/// The most deliveries that one pass claims for one tenant. The sends of one tenant run in turn,
/// so the batch and the longest send bound the time of the tenant.
pub const BATCH: i64 = 10;

/// The number of attempts after which a delivery ends as failed.
pub const MAX_ATTEMPTS: i32 = 5;

/// The wait after the failed attempt with the given number.
pub fn backoff(attempt: i32) -> Duration {
    match attempt {
        ..=1 => Duration::seconds(30),
        2 => Duration::minutes(2),
        3 => Duration::minutes(10),
        _ => Duration::hours(1),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub sent: usize,
    pub retried: usize,
    pub failed: usize,
    /// Rows whose claim another sender took over while this pass held it.
    pub lost: usize,
    /// Rows that went back unsent because the budget of the tenant left no room for the send.
    pub released: usize,
}

/// The longest time that one send of the kind takes. A send starts only when this much of the
/// budget of the tenant remains.
fn send_limit(kind: ChannelKind) -> std::time::Duration {
    match kind {
        ChannelKind::Email => email::SEND_TIMEOUT,
        ChannelKind::Webhook => webhook::SEND_TIMEOUT,
    }
}

// Read in the transaction that starts the send, so a channel that the operator disabled, changed
// or deleted since the claim never gets a send from the claim.
struct Target {
    /// `None` after the operator deleted it, between the claim and the send.
    channel: Option<Channel>,
    credentials: Option<WebhookCredentials>,
}

async fn read_target(
    tx: &mut TenantTx<'_>,
    provider: &dyn KekProvider,
    id: ChannelId,
) -> Result<Target, DbError> {
    let Some(channel) = channels::get(tx, id).await? else {
        return Ok(Target {
            channel: None,
            credentials: None,
        });
    };
    let credentials = if channel.enabled && channel.kind == ChannelKind::Webhook {
        match secret::read_webhook(tx, provider, &channel).await {
            Ok(value) => Some(value),
            Err(error) => {
                tracing::warn!(%error, channel = %channel.id, "cannot read the secrets of the webhook");
                None
            }
        }
    } else {
        None
    };
    Ok(Target {
        channel: Some(channel),
        credentials,
    })
}

/// Every time comes from `clock` at the moment of use, so a slow earlier send does not age the
/// lease, the backoff or the signature of a later one. A send starts only while `deadline` leaves
/// room for it; the claims after that go back unsent.
pub async fn deliver_tenant(
    db: &Database,
    sender: &Sender,
    provider: &dyn KekProvider,
    clock: &dyn Clock,
    tenant: TenantId,
    tenant_name: &str,
    deadline: tokio::time::Instant,
) -> Result<Report, DbError> {
    let claimed = {
        let mut tx = db.begin_tenant(tenant).await?;
        let claim_time = clock.now();
        let claimed =
            alerts::claim_due(&mut tx, claim_time, claim_time + LEASE, BATCH, MAX_ATTEMPTS).await?;
        tx.commit().await?;
        claimed
    };

    let mut report = Report::default();
    for (index, delivery) in claimed.iter().enumerate() {
        let room = deadline.saturating_duration_since(tokio::time::Instant::now());
        if room < send_limit(delivery.channel.kind) {
            let mut tx = db.begin_tenant(tenant).await?;
            for rest in &claimed[index..] {
                if alerts::release(&mut tx, rest.id, rest.attempt, clock.now()).await? {
                    report.released += 1;
                }
            }
            tx.commit().await?;
            break;
        }

        let mut tx = db.begin_tenant(tenant).await?;
        let Some(attempt) =
            alerts::extend_lease(&mut tx, delivery.id, delivery.attempt, clock.now() + LEASE)
                .await?
        else {
            tx.commit().await?;
            report.lost += 1;
            continue;
        };
        let target = read_target(&mut tx, provider, delivery.channel.id).await?;
        tx.commit().await?;

        let result = send_one(sender, delivery, &target, tenant, tenant_name, clock).await;

        let finished = clock.now();
        let mut tx = db.begin_tenant(tenant).await?;
        match result {
            Ok(note) => {
                if alerts::mark_sent(&mut tx, delivery.id, attempt, finished, note.as_deref())
                    .await?
                {
                    let mut event = AuditEvent::new(AuditAction::AlertSent, "alert");
                    event.object_id = Some(delivery.alert.id.as_uuid());
                    event.diff = Some(diff(
                        None,
                        Some(&json!({
                            "channel_id": delivery.channel.id.to_string(),
                            "channel_kind": delivery.channel.kind.as_str(),
                            "event": delivery.event.as_str(),
                            "delivery_id": delivery.id.to_string(),
                            "ok": true,
                        })),
                        &[],
                    ));
                    restichub_db::audit::insert(&mut tx, &event).await?;
                    report.sent += 1;
                } else {
                    report.lost += 1;
                }
            }
            Err(error) => {
                let last = error.is_permanent() || attempt >= MAX_ATTEMPTS;
                let retry_at = (!last).then(|| finished + backoff(attempt));
                let recorded = alerts::mark_attempt_failed(
                    &mut tx,
                    delivery.id,
                    attempt,
                    &error.code(),
                    retry_at,
                )
                .await?;
                if !recorded {
                    report.lost += 1;
                } else if last {
                    report.failed += 1;
                    tracing::warn!(
                        delivery = %delivery.id,
                        channel = %delivery.channel.id,
                        error = %error.code(),
                        "an alert delivery failed"
                    );
                } else {
                    report.retried += 1;
                }
            }
        }
        tx.commit().await?;
    }
    Ok(report)
}

async fn send_one(
    sender: &Sender,
    delivery: &ClaimedDelivery,
    target: &Target,
    tenant: TenantId,
    tenant_name: &str,
    clock: &dyn Clock,
) -> Result<Option<String>, SendError> {
    let channel = match &target.channel {
        Some(channel) if channel.enabled => channel,
        _ => return Err(SendError::ChannelDisabled),
    };
    let message = Message {
        delivery_id: delivery.id,
        event: MessageEvent::from(delivery.event),
        // The signature timestamp of a webhook is this time; a receiver rejects a stale one.
        sent_at: clock.now(),
        tenant_id: tenant,
        tenant_name: tenant_name.to_owned(),
        alert: Some(delivery.alert.clone()),
        public_origin: sender.settings().public_origin.clone(),
        language: sender.settings().language,
    };
    send_message(sender, channel, target.credentials.as_ref(), &message).await
}

/// Sends a message through a channel with the credentials that the caller read. `Some` is a note
/// about a part of the recipients that the server refused.
pub async fn send_message(
    sender: &Sender,
    channel: &restichub_db::channels::Channel,
    credentials: Option<&WebhookCredentials>,
    message: &Message,
) -> Result<Option<String>, SendError> {
    match channel.kind {
        ChannelKind::Email => sender.send_email(&channel.recipients, message).await,
        ChannelKind::Webhook => {
            let credentials = credentials.ok_or(SendError::SecretUnavailable)?;
            sender
                .send_webhook(
                    credentials.url.expose_secret(),
                    credentials.signing.expose_secret().as_bytes(),
                    message,
                )
                .await?;
            Ok(None)
        }
    }
}
