//! The delivery of the queued alert messages (FR-MN-04, ADR-0036).
//!
//! A pass claims the due rows of the outbox in one transaction, then handles each row in turn: it
//! extends the lease of the row, sends with no transaction open, and records the result with the
//! number of its attempt. The claim is a lease, so two instances never send one row, and a crash
//! repeats the send after the lease ends. A holder whose lease ended and whose row another sender
//! claimed since changes nothing.

use restichub_db::alerts::{self, ClaimedDelivery};
use restichub_db::audit::{AuditAction, AuditEvent, diff};
use restichub_db::channels::ChannelKind;
use restichub_db::{Database, DbError, TenantId};
use restichub_secrets::KekProvider;
use serde_json::json;
use time::Duration;

use crate::clock::Clock;

use super::message::{Message, MessageEvent};
use super::secret::{self, WebhookCredentials};
use super::sender::Sender;
use super::transport::SendError;

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
}

struct Prepared {
    claimed: ClaimedDelivery,
    /// `None` for an email channel, and for a webhook whose secrets cannot be read.
    credentials: Option<WebhookCredentials>,
}

/// Sends the due deliveries of one tenant. Every time comes from `clock` at the moment of use, so
/// a slow earlier send does not age the lease, the backoff or the signature of a later one.
pub async fn deliver_tenant(
    db: &Database,
    sender: &Sender,
    provider: &dyn KekProvider,
    clock: &dyn Clock,
    tenant: TenantId,
    tenant_name: &str,
) -> Result<Report, DbError> {
    let prepared = {
        let mut tx = db.begin_tenant(tenant).await?;
        let claim_time = clock.now();
        let claimed = alerts::claim_due(&mut tx, claim_time, claim_time + LEASE, BATCH).await?;
        let mut prepared = Vec::with_capacity(claimed.len());
        for claimed in claimed {
            let credentials = if claimed.channel.kind == ChannelKind::Webhook {
                match secret::read_webhook(&mut tx, provider, &claimed.channel).await {
                    Ok(value) => Some(value),
                    Err(error) => {
                        tracing::warn!(
                            %error,
                            channel = %claimed.channel.id,
                            "cannot read the secrets of the webhook"
                        );
                        None
                    }
                }
            } else {
                None
            };
            prepared.push(Prepared {
                claimed,
                credentials,
            });
        }
        tx.commit().await?;
        prepared
    };

    let mut report = Report::default();
    for item in &prepared {
        let delivery = &item.claimed;
        let mut tx = db.begin_tenant(tenant).await?;
        let held =
            alerts::extend_lease(&mut tx, delivery.id, delivery.attempt, clock.now() + LEASE)
                .await?;
        tx.commit().await?;
        if !held {
            report.lost += 1;
            continue;
        }

        let result = send_one(sender, item, tenant, tenant_name, clock).await;

        let finished = clock.now();
        let mut tx = db.begin_tenant(tenant).await?;
        match result {
            Ok(note) => {
                if alerts::mark_sent(
                    &mut tx,
                    delivery.id,
                    delivery.attempt,
                    finished,
                    note.as_deref(),
                )
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
                let last = error.is_permanent() || delivery.attempt >= MAX_ATTEMPTS;
                let retry_at = (!last).then(|| finished + backoff(delivery.attempt));
                let recorded = alerts::mark_attempt_failed(
                    &mut tx,
                    delivery.id,
                    delivery.attempt,
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
    item: &Prepared,
    tenant: TenantId,
    tenant_name: &str,
    clock: &dyn Clock,
) -> Result<Option<String>, SendError> {
    let delivery = &item.claimed;
    if !delivery.channel.enabled {
        return Err(SendError::ChannelDisabled);
    }
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
    send_message(
        sender,
        &delivery.channel,
        item.credentials.as_ref(),
        &message,
    )
    .await
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
