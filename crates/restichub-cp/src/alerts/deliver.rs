//! The delivery of the queued alert messages (FR-MN-04, ADR-0035).
//!
//! A pass claims the due rows of the outbox in one transaction, sends them with no transaction
//! open, and records the results in a second transaction. The claim is a lease, so two instances
//! never send one row, and a crash repeats the send after the lease ends.

use restichub_db::alerts::{self, ClaimedDelivery};
use restichub_db::audit::{AuditAction, AuditEvent, diff};
use restichub_db::channels::ChannelKind;
use restichub_db::{Database, DbError, TenantId};
use restichub_secrets::{KekProvider, SecretString};
use serde_json::json;
use time::{Duration, OffsetDateTime};

use super::message::{Message, MessageEvent};
use super::secret;
use super::sender::Sender;
use super::transport::SendError;

/// Lease length: a crashed sender leaves the row due again after this time.
const LEASE: Duration = Duration::minutes(5);

/// The most deliveries that one pass claims for one tenant.
const BATCH: i64 = 20;

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
}

struct Prepared {
    claimed: ClaimedDelivery,
    /// `None` for an email channel, and for a webhook whose secret cannot be read.
    secret: Option<SecretString>,
}

/// Sends the due deliveries of one tenant.
pub async fn deliver_tenant(
    db: &Database,
    sender: &Sender,
    provider: &dyn KekProvider,
    tenant: TenantId,
    tenant_name: &str,
    now: OffsetDateTime,
) -> Result<Report, DbError> {
    let prepared = {
        let mut tx = db.begin_tenant(tenant).await?;
        let claimed = alerts::claim_due(&mut tx, now, now + LEASE, BATCH).await?;
        let mut prepared = Vec::with_capacity(claimed.len());
        for claimed in claimed {
            let secret = match (claimed.channel.kind, claimed.channel.secret_id) {
                (ChannelKind::Webhook, Some(secret_id)) => {
                    match secret::read(&mut tx, provider, secret_id).await {
                        Ok(value) => Some(value),
                        Err(error) => {
                            tracing::warn!(
                                %error,
                                channel = %claimed.channel.id,
                                "cannot read the webhook secret"
                            );
                            None
                        }
                    }
                }
                _ => None,
            };
            prepared.push(Prepared { claimed, secret });
        }
        tx.commit().await?;
        prepared
    };
    if prepared.is_empty() {
        return Ok(Report::default());
    }

    let mut results = Vec::with_capacity(prepared.len());
    for item in &prepared {
        let result = send_one(sender, item, tenant, tenant_name, now).await;
        results.push(result);
    }

    let mut report = Report::default();
    let mut tx = db.begin_tenant(tenant).await?;
    for (item, result) in prepared.iter().zip(results) {
        let delivery = &item.claimed;
        match result {
            Ok(()) => {
                alerts::mark_sent(&mut tx, delivery.id, now).await?;
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
            }
            Err(error) => {
                let last = error.is_permanent() || delivery.attempt >= MAX_ATTEMPTS;
                let retry_at = (!last).then(|| now + backoff(delivery.attempt));
                alerts::mark_attempt_failed(&mut tx, delivery.id, &error.code(), retry_at).await?;
                if last {
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
    }
    tx.commit().await?;
    Ok(report)
}

async fn send_one(
    sender: &Sender,
    item: &Prepared,
    tenant: TenantId,
    tenant_name: &str,
    now: OffsetDateTime,
) -> Result<(), SendError> {
    let delivery = &item.claimed;
    if !delivery.channel.enabled {
        return Err(SendError::ChannelDisabled);
    }
    let message = Message {
        delivery_id: delivery.id,
        event: MessageEvent::from(delivery.event),
        sent_at: now,
        tenant_id: tenant,
        tenant_name: tenant_name.to_owned(),
        alert: Some(delivery.alert.clone()),
        public_origin: sender.settings().public_origin.clone(),
    };
    send_message(sender, &delivery.channel, item.secret.as_ref(), &message).await
}

/// Sends a message through a channel with the secret that the caller read.
pub async fn send_message(
    sender: &Sender,
    channel: &restichub_db::channels::Channel,
    secret: Option<&SecretString>,
    message: &Message,
) -> Result<(), SendError> {
    match channel.kind {
        ChannelKind::Email => sender.send_email(&channel.recipients, message).await,
        ChannelKind::Webhook => {
            let url = channel.url.as_deref().ok_or(SendError::InvalidUrl)?;
            let secret = secret.ok_or(SendError::SecretUnavailable)?;
            sender
                .send_webhook(url, secret.expose_secret().as_bytes(), message)
                .await
        }
    }
}
