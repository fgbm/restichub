//! The sender: it delivers one message through a channel (ADR-0036).

use std::sync::Arc;

use tokio_rustls::rustls::{self, ClientConfig};

use super::message::Message;
use super::settings::AlertsSettings;
use super::ssrf::parse_target;
use super::transport::{Resolver, SendError, SystemResolver, default_tls_config};
use super::{email, webhook};

#[derive(Clone)]
pub struct Sender {
    settings: Arc<AlertsSettings>,
    tls: Arc<ClientConfig>,
    resolver: Arc<dyn Resolver>,
}

impl std::fmt::Debug for Sender {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Sender").finish_non_exhaustive()
    }
}

impl Sender {
    pub fn new(settings: Arc<AlertsSettings>) -> Result<Self, rustls::Error> {
        Ok(Self {
            settings,
            tls: default_tls_config()?,
            resolver: Arc::new(SystemResolver),
        })
    }

    /// Replaces the TLS client configuration. A test trusts a self-signed root with it.
    pub fn with_tls(mut self, tls: Arc<ClientConfig>) -> Self {
        self.tls = tls;
        self
    }

    /// Replaces the host name resolver. A test answers with a chosen address.
    pub fn with_resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.resolver = resolver;
        self
    }

    pub fn settings(&self) -> &AlertsSettings {
        &self.settings
    }

    pub fn email_configured(&self) -> bool {
        self.settings.smtp.is_some()
    }

    pub async fn send_email(
        &self,
        recipients: &[String],
        message: &Message,
    ) -> Result<Option<String>, SendError> {
        let smtp = self
            .settings
            .smtp
            .as_ref()
            .ok_or(SendError::SmtpNotConfigured)?;
        let domain = smtp
            .from
            .rsplit('@')
            .next()
            .unwrap_or("restichub")
            .to_owned();
        let mail = email::Mail {
            subject: message.subject(),
            text: message.text(),
            message_id_domain: domain,
            date: message.sent_at,
            id: message.delivery_id.to_string(),
        };
        email::send(smtp, &self.tls, recipients, &mail).await
    }

    pub async fn send_webhook(
        &self,
        url: &str,
        secret: &[u8],
        message: &Message,
    ) -> Result<(), SendError> {
        let target = parse_target(url, &self.settings.webhook)?;
        let body = message.json();
        let delivery_id = message.delivery_id.to_string();
        webhook::send(
            &self.settings.webhook,
            self.resolver.as_ref(),
            &self.tls,
            &webhook::Request {
                target: &target,
                body: &body,
                secret,
                timestamp: message.sent_at.unix_timestamp(),
                delivery_id: &delivery_id,
            },
        )
        .await
    }
}
