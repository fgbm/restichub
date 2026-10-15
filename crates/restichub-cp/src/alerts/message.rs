//! The message that a channel sends: the typed webhook payload and the text of an email
//! (FR-MN-03, AC-MN-02, ADR-0035).

use restichub_db::alerts::{Alert, AlertKind, DeliveryEvent};
use restichub_db::{DeliveryId, TenantId};
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The version of the webhook payload. A change that breaks a receiver raises it.
pub const SCHEMA_VERSION: u32 = 1;

/// The longest scrubbed text in a message, in characters.
pub const MAX_TEXT_CHARS: usize = 500;

const REDACTED: &str = "<redacted>";

/// What the message tells the receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MessageEvent {
    #[serde(rename = "alert.opened")]
    Opened,
    #[serde(rename = "alert.resolved")]
    Resolved,
    /// A message that an Admin starts with the test button.
    #[serde(rename = "test")]
    Test,
}

impl From<DeliveryEvent> for MessageEvent {
    fn from(event: DeliveryEvent) -> Self {
        match event {
            DeliveryEvent::Opened => Self::Opened,
            DeliveryEvent::Resolved => Self::Resolved,
        }
    }
}

impl MessageEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Opened => "opened",
            Self::Resolved => "resolved",
            Self::Test => "test",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Reference {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobReference {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlertPayload {
    pub id: String,
    pub kind: AlertKind,
    pub status: &'static str,
    pub opened_at: String,
    pub resolved_at: Option<String>,
    pub agent: Option<Reference>,
    pub plan: Option<Reference>,
    pub repository: Option<Reference>,
    pub job: Option<JobReference>,
    /// Scrubbed and bounded.
    pub message: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Payload {
    pub schema_version: u32,
    /// The delivery. A receiver deduplicates on it, because a send can repeat.
    pub id: String,
    pub event: MessageEvent,
    pub sent_at: String,
    pub tenant: Reference,
    /// Absent for a test message.
    pub alert: Option<AlertPayload>,
}

/// Input of one send. It holds no secret.
#[derive(Debug, Clone)]
pub struct Message {
    pub delivery_id: DeliveryId,
    pub event: MessageEvent,
    pub sent_at: OffsetDateTime,
    pub tenant_id: TenantId,
    pub tenant_name: String,
    pub alert: Option<Alert>,
    /// Base URL from the installation setting. Links are absent without it.
    pub public_origin: Option<String>,
}

fn rfc3339(time: OffsetDateTime) -> String {
    time.format(&Rfc3339).unwrap_or_default()
}

impl Message {
    pub fn payload(&self) -> Payload {
        Payload {
            schema_version: SCHEMA_VERSION,
            id: self.delivery_id.to_string(),
            event: self.event,
            sent_at: rfc3339(self.sent_at),
            tenant: Reference {
                id: self.tenant_id.to_string(),
                name: scrub(&self.tenant_name, 120),
            },
            alert: self.alert.as_ref().map(|alert| AlertPayload {
                id: alert.id.to_string(),
                kind: alert.kind,
                status: alert.status.as_str(),
                opened_at: rfc3339(alert.opened_at),
                resolved_at: alert.resolved_at.map(rfc3339),
                agent: alert
                    .agent_id
                    .zip(alert.agent_hostname.as_ref())
                    .map(|(id, name)| Reference {
                        id: id.to_string(),
                        name: scrub(name, 120),
                    }),
                plan: alert
                    .plan_id
                    .zip(alert.plan_name.as_ref())
                    .map(|(id, name)| Reference {
                        id: id.to_string(),
                        name: scrub(name, 120),
                    }),
                repository: alert.repository_id.zip(alert.repository_name.as_ref()).map(
                    |(id, name)| Reference {
                        id: id.to_string(),
                        name: scrub(name, 120),
                    },
                ),
                job: alert.job_id.map(|id| JobReference { id: id.to_string() }),
                message: scrub(&alert.detail, MAX_TEXT_CHARS),
                url: link(self.public_origin.as_deref(), alert),
            }),
        }
    }

    pub fn json(&self) -> Vec<u8> {
        serde_json::to_vec(&self.payload()).unwrap_or_default()
    }

    /// One line only, so no header can be injected.
    pub fn subject(&self) -> String {
        let tenant = scrub(&self.tenant_name, 60);
        match (&self.alert, self.event) {
            (None, _) => format!("[ResticHub] Test message ({tenant})"),
            (Some(alert), MessageEvent::Resolved) => format!(
                "[ResticHub] Resolved: {} - {} ({tenant})",
                kind_title(alert.kind),
                subject_name(alert)
            ),
            (Some(alert), _) => format!(
                "[ResticHub] {} - {} ({tenant})",
                kind_title(alert.kind),
                subject_name(alert)
            ),
        }
    }

    pub fn text(&self) -> String {
        let mut lines = Vec::new();
        let Some(alert) = &self.alert else {
            lines.push("This is a test message from ResticHub.".to_owned());
            lines.push(format!("Tenant: {}", scrub(&self.tenant_name, 120)));
            return lines.join("\n") + "\n";
        };
        let headline = if self.event == MessageEvent::Resolved {
            format!("Resolved: {}", kind_title(alert.kind))
        } else {
            kind_title(alert.kind).to_owned()
        };
        lines.push(headline);
        lines.push(String::new());
        lines.push(format!("Tenant: {}", scrub(&self.tenant_name, 120)));
        if let Some(name) = &alert.agent_hostname {
            lines.push(format!("Host: {}", scrub(name, 120)));
        }
        if let Some(name) = &alert.plan_name {
            lines.push(format!("Plan: {}", scrub(name, 120)));
        }
        if let Some(name) = &alert.repository_name {
            lines.push(format!("Repository: {}", scrub(name, 120)));
        }
        lines.push(format!("Details: {}", scrub(&alert.detail, MAX_TEXT_CHARS)));
        lines.push(format!("Opened: {}", rfc3339(alert.opened_at)));
        if let Some(resolved) = alert.resolved_at {
            lines.push(format!("Resolved: {}", rfc3339(resolved)));
        }
        if let Some(url) = link(self.public_origin.as_deref(), alert) {
            lines.push(format!("Link: {url}"));
        }
        lines.join("\n") + "\n"
    }
}

fn subject_name(alert: &Alert) -> String {
    let name = alert
        .plan_name
        .as_deref()
        .or(alert.repository_name.as_deref())
        .or(alert.agent_hostname.as_deref())
        .unwrap_or("-");
    scrub(name, 80)
}

pub fn kind_title(kind: AlertKind) -> &'static str {
    match kind {
        AlertKind::RpoViolated => "Backup is overdue",
        AlertKind::JobFailed => "Job failed",
        AlertKind::JobDeadLetter => "Job gave up after lock retries",
        AlertKind::JobWarnings => "Job finished with warnings",
        AlertKind::AgentOffline => "Host is offline",
        AlertKind::StaleLock => "Repository lock is stale",
    }
}

/// The link of an alert: the job when the alert names one, else the page of the subject.
pub fn link(origin: Option<&str>, alert: &Alert) -> Option<String> {
    let origin = origin?;
    Some(match (alert.job_id, alert.kind) {
        (Some(job), _) => format!("{origin}/jobs/{job}"),
        (None, AlertKind::RpoViolated) => format!("{origin}/plans"),
        (None, AlertKind::AgentOffline) => format!("{origin}/hosts"),
        (None, _) => format!("{origin}/repositories"),
    })
}

/// Makes text safe for a message: removes control characters, hides credentials and cuts the text
/// to `max_chars` characters.
///
/// A credential in a URL (`https://user:pass@host`), a value after `password=`, `token:` or
/// `secret=`, and a value after `Bearer` become `<redacted>`.
pub fn scrub(text: &str, max_chars: usize) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();
    let mut out: Vec<String> = Vec::with_capacity(tokens.len());
    let mut hide_next = false;
    for token in tokens {
        let lower = token.to_ascii_lowercase();
        let scheme = matches!(lower.as_str(), "bearer" | "basic");
        if hide_next && !scheme {
            hide_next = false;
            out.push(REDACTED.to_owned());
            continue;
        }
        if scheme {
            hide_next = true;
            out.push(token.to_owned());
            continue;
        }
        out.push(scrub_token(token, &lower, &mut hide_next));
    }
    let joined = out.join(" ");
    if joined.chars().count() <= max_chars {
        return joined;
    }
    let mut cut: String = joined.chars().take(max_chars.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

const SENSITIVE: [&str; 8] = [
    "password",
    "passwd",
    "secret",
    "token",
    "credential",
    "authorization",
    "apikey",
    "api_key",
];

fn is_sensitive(lower: &str) -> bool {
    SENSITIVE.iter().any(|word| lower.contains(word)) || lower.ends_with("key")
}

fn scrub_token(token: &str, lower: &str, hide_next: &mut bool) -> String {
    if let Some(scheme_end) = token.find("://") {
        let rest = &token[scheme_end + 3..];
        let authority_end = rest.find('/').unwrap_or(rest.len());
        if let Some(at) = rest[..authority_end].rfind('@') {
            return format!("{}{}{}", &token[..scheme_end + 3], REDACTED, &rest[at..]);
        }
    }
    if let Some(separator) = token.find(['=', ':']) {
        let (name, value) = (&lower[..separator], &token[separator + 1..]);
        if is_sensitive(name) {
            if value.is_empty() {
                *hide_next = true;
                return token.to_owned();
            }
            return format!("{}{}", &token[..=separator], REDACTED);
        }
    }
    token.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-MN-02: the scrubbed text hides credentials and stays bounded.
    #[test]
    fn ac_mn_02_scrub_hides_credentials() {
        assert_eq!(
            scrub("fatal: password=hunter2 for user", 500),
            "fatal: password=<redacted> for user"
        );
        assert_eq!(
            scrub("open https://alice:s3cret@example.com/repo failed", 500),
            "open https://<redacted>@example.com/repo failed"
        );
        assert_eq!(
            scrub("Authorization: Bearer abc.def", 500),
            "Authorization: Bearer <redacted>"
        );
        assert_eq!(scrub("token: abc", 500), "token: <redacted>");
        assert_eq!(
            scrub("AWS_SECRET_ACCESS_KEY=abcd", 500),
            "AWS_SECRET_ACCESS_KEY=<redacted>"
        );
        assert_eq!(
            scrub("line one\nline two\r\n\u{7}bell", 500),
            "line one line two bell"
        );
        let long = "x".repeat(2000);
        assert_eq!(scrub(&long, 50).chars().count(), 50);
    }
}
