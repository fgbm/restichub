//! The installation settings of the alerts: the public origin, the SMTP server and the webhook
//! policy (ADR-0036). The settings never live in the database.

use std::path::PathBuf;

use axum::http::Uri;
use ipnet::IpNet;
use restichub_secrets::{EnvReader, SecretString, read_secret};
use serde::Deserialize;

use crate::config::ConfigError;

use super::text::Language;

pub const PUBLIC_ORIGIN_ENV: &str = "RESTICHUB_PUBLIC_ORIGIN";
pub const SMTP_HOST_ENV: &str = "RESTICHUB_SMTP_HOST";
pub const SMTP_PORT_ENV: &str = "RESTICHUB_SMTP_PORT";
pub const SMTP_SECURITY_ENV: &str = "RESTICHUB_SMTP_SECURITY";
pub const SMTP_USERNAME_ENV: &str = "RESTICHUB_SMTP_USERNAME";
pub const SMTP_PASSWORD_FILE_ENV: &str = "RESTICHUB_SMTP_PASSWORD_FILE";
pub const SMTP_PASSWORD_ENV: &str = "RESTICHUB_SMTP_PASSWORD";
pub const SMTP_FROM_ENV: &str = "RESTICHUB_SMTP_FROM";
pub const NOTIFICATIONS_LANGUAGE_ENV: &str = "RESTICHUB_NOTIFICATIONS_LANGUAGE";
pub const WEBHOOK_ALLOW_HTTP_ENV: &str = "RESTICHUB_WEBHOOK_ALLOW_HTTP";
pub const WEBHOOK_ALLOW_NETWORKS_ENV: &str = "RESTICHUB_WEBHOOK_ALLOW_NETWORKS";

/// How the control plane secures the connection to the SMTP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SmtpSecurity {
    /// Plain connection, then `STARTTLS`. Port 587.
    #[default]
    StartTls,
    /// TLS from the first byte. Port 465.
    Tls,
    /// No TLS. The control plane then sends no credentials.
    None,
}

impl SmtpSecurity {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "starttls" => Some(Self::StartTls),
            "tls" => Some(Self::Tls),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    fn default_port(self) -> u16 {
        match self {
            Self::StartTls => 587,
            Self::Tls => 465,
            Self::None => 25,
        }
    }
}

/// The SMTP server. A `Debug` shows no password.
#[derive(Debug)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub security: SmtpSecurity,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub from: String,
}

/// The webhook policy of the installation (ADR-0036).
#[derive(Debug, Clone, Default)]
pub struct WebhookSettings {
    /// Plain HTTP targets. The default is HTTPS only.
    pub allow_http: bool,
    /// The networks that the address check admits although they are not public.
    pub allow_networks: Vec<IpNet>,
}

#[derive(Debug, Default)]
pub struct AlertsSettings {
    /// The base URL of the links in a message, without a trailing slash.
    pub public_origin: Option<String>,
    pub smtp: Option<SmtpSettings>,
    pub webhook: WebhookSettings,
    pub language: Language,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationsSection {
    language: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SmtpSection {
    host: Option<String>,
    port: Option<u16>,
    security: Option<String>,
    username: Option<String>,
    password_file: Option<PathBuf>,
    from: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookSection {
    allow_http: Option<bool>,
    allow_networks: Option<Vec<String>>,
}

impl AlertsSettings {
    /// Reads the settings. The environment overrides the file.
    pub fn load(
        env: &dyn EnvReader,
        public_origin: Option<String>,
        smtp: Option<&SmtpSection>,
        webhook: Option<&WebhookSection>,
        notifications: Option<&NotificationsSection>,
    ) -> Result<Self, ConfigError> {
        let public_origin = env
            .get(PUBLIC_ORIGIN_ENV)
            .or(public_origin)
            .map(|value| public_origin_value(&value))
            .transpose()?;
        Ok(Self {
            public_origin,
            smtp: smtp_settings(env, smtp)?,
            webhook: webhook_settings(env, webhook)?,
            language: language_setting(env, notifications)?,
        })
    }
}

fn language_setting(
    env: &dyn EnvReader,
    section: Option<&NotificationsSection>,
) -> Result<Language, ConfigError> {
    let value = env
        .get(NOTIFICATIONS_LANGUAGE_ENV)
        .or_else(|| section.and_then(|section| section.language.clone()));
    match value {
        None => Ok(Language::default()),
        Some(value) => Language::parse(&value).ok_or_else(|| ConfigError::Invalid {
            name: "notifications.language",
            reason: format!("`{value}` is not `en` or `ru`"),
        }),
    }
}

/// Accepts `scheme://host[:port]` with no user, path or query. Returns it without a slash at the end.
fn public_origin_value(value: &str) -> Result<String, ConfigError> {
    let invalid = |reason: &str| ConfigError::Invalid {
        name: "public_origin",
        reason: reason.to_owned(),
    };
    let trimmed = value.trim().trim_end_matches('/');
    let uri: Uri = trimmed
        .parse()
        .map_err(|error| invalid(&format!("it is not a URL: {error}")))?;
    if !matches!(uri.scheme_str(), Some("https" | "http")) {
        return Err(invalid("it must use the scheme `https` or `http`"));
    }
    let Some(authority) = uri.authority() else {
        return Err(invalid("it holds no host"));
    };
    if authority.as_str().contains('@') {
        return Err(invalid("it must not hold a user"));
    }
    if uri.query().is_some() || !matches!(uri.path(), "" | "/") {
        return Err(invalid(
            "it must hold the scheme, the host and the port only",
        ));
    }
    Ok(trimmed.to_owned())
}

fn smtp_settings(
    env: &dyn EnvReader,
    section: Option<&SmtpSection>,
) -> Result<Option<SmtpSettings>, ConfigError> {
    let section = section.cloned().unwrap_or_default();
    let Some(host) = env.get(SMTP_HOST_ENV).or(section.host) else {
        return Ok(None);
    };
    let invalid = |name: &'static str, reason: String| ConfigError::Invalid { name, reason };
    let security = match env.get(SMTP_SECURITY_ENV).or(section.security) {
        Some(value) => SmtpSecurity::parse(&value).ok_or_else(|| {
            invalid(
                "smtp.security",
                format!("`{value}` is not `starttls`, `tls` or `none`"),
            )
        })?,
        None => SmtpSecurity::default(),
    };
    let port = match env.get(SMTP_PORT_ENV) {
        Some(value) => value.trim().parse::<u16>().map_err(|error| {
            invalid(
                "smtp.port",
                format!("`{value}` is not a port number: {error}"),
            )
        })?,
        None => section.port.unwrap_or(security.default_port()),
    };
    let username = env.get(SMTP_USERNAME_ENV).or(section.username);
    let password_file = env
        .get(SMTP_PASSWORD_FILE_ENV)
        .map(PathBuf::from)
        .or(section.password_file);
    let password = if password_file.is_some() || env.get(SMTP_PASSWORD_ENV).is_some() {
        Some(read_secret(
            password_file.as_deref(),
            SMTP_PASSWORD_ENV,
            env,
        )?)
    } else {
        None
    };
    if password.is_some() && username.is_none() {
        return Err(invalid(
            "smtp.username",
            "a password is set without a user name".to_owned(),
        ));
    }
    // A password over a plain connection would travel in the clear (ADR-0036).
    if password.is_some() && security == SmtpSecurity::None {
        return Err(invalid(
            "smtp.security",
            "a password needs `starttls` or `tls`, because `none` would send it in the clear"
                .to_owned(),
        ));
    }
    let from = env
        .get(SMTP_FROM_ENV)
        .or(section.from)
        .ok_or_else(|| invalid("smtp.from", "the value is not set".to_owned()))?;
    if from.chars().any(|c| c.is_control() || c.is_whitespace()) || !from.contains('@') {
        return Err(invalid(
            "smtp.from",
            "it must be one address without spaces".to_owned(),
        ));
    }
    if host.trim().is_empty() || host.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(invalid("smtp.host", "it is not a host name".to_owned()));
    }
    Ok(Some(SmtpSettings {
        host,
        port,
        security,
        username,
        password,
        from,
    }))
}

fn webhook_settings(
    env: &dyn EnvReader,
    section: Option<&WebhookSection>,
) -> Result<WebhookSettings, ConfigError> {
    let section = section.cloned().unwrap_or_default();
    let allow_http =
        crate::config::boolean(env, WEBHOOK_ALLOW_HTTP_ENV, section.allow_http)?.unwrap_or(false);
    let entries: Vec<String> = match env.get(WEBHOOK_ALLOW_NETWORKS_ENV) {
        Some(value) => value
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_owned)
            .collect(),
        None => section.allow_networks.unwrap_or_default(),
    };
    let allow_networks = entries
        .iter()
        .map(|entry| {
            entry
                .parse::<IpNet>()
                .map_err(|error| ConfigError::Invalid {
                    name: "webhook.allow_networks",
                    reason: format!("`{entry}` is not a network in CIDR form: {error}"),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WebhookSettings {
        allow_http,
        allow_networks,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use restichub_secrets::EnvReader;

    use super::*;

    struct MapEnv(HashMap<String, String>);

    impl EnvReader for MapEnv {
        fn get(&self, name: &str) -> Option<String> {
            self.0.get(name).cloned()
        }
    }

    fn env(pairs: &[(&str, &str)]) -> MapEnv {
        MapEnv(
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        )
    }

    fn load(pairs: &[(&str, &str)]) -> Result<AlertsSettings, ConfigError> {
        AlertsSettings::load(&env(pairs), None, None, None, None)
    }

    /// AC-MN-05: the allow-list and the scheme rule come from the environment.
    #[test]
    fn ac_mn_05_webhook_policy_from_the_environment() {
        let settings = load(&[
            ("RESTICHUB_WEBHOOK_ALLOW_NETWORKS", "10.0.0.0/8, fd00::/8"),
            ("RESTICHUB_WEBHOOK_ALLOW_HTTP", "true"),
        ])
        .expect("the settings load");
        assert!(settings.webhook.allow_http);
        assert_eq!(settings.webhook.allow_networks.len(), 2);
        assert!(matches!(
            load(&[("RESTICHUB_WEBHOOK_ALLOW_NETWORKS", "10.0.0.0")]),
            Err(ConfigError::Invalid {
                name: "webhook.allow_networks",
                ..
            })
        ));
        let defaults = load(&[]).expect("the defaults load");
        assert!(!defaults.webhook.allow_http && defaults.webhook.allow_networks.is_empty());
    }

    #[test]
    fn the_public_origin_holds_no_path_or_user() {
        let ok = load(&[("RESTICHUB_PUBLIC_ORIGIN", "https://backup.example.com/")]).expect("ok");
        assert_eq!(
            ok.public_origin.as_deref(),
            Some("https://backup.example.com")
        );
        for bad in [
            "https://user@backup.example.com",
            "https://backup.example.com/app",
            "https://backup.example.com?x=1",
            "ftp://backup.example.com",
            "backup.example.com",
        ] {
            assert!(
                load(&[("RESTICHUB_PUBLIC_ORIGIN", bad)]).is_err(),
                "{bad} must be refused"
            );
        }
    }

    #[test]
    fn the_smtp_server_needs_a_host_and_a_sender() {
        assert!(load(&[]).expect("none").smtp.is_none());
        let settings = load(&[
            ("RESTICHUB_SMTP_HOST", "smtp.example.com"),
            ("RESTICHUB_SMTP_FROM", "restichub@example.com"),
        ])
        .expect("loads");
        let smtp = settings.smtp.expect("smtp");
        assert_eq!((smtp.port, smtp.security), (587, SmtpSecurity::StartTls));
        assert!(smtp.password.is_none());
        assert!(matches!(
            load(&[("RESTICHUB_SMTP_HOST", "smtp.example.com")]),
            Err(ConfigError::Invalid {
                name: "smtp.from",
                ..
            })
        ));
        let tls = load(&[
            ("RESTICHUB_SMTP_HOST", "smtp.example.com"),
            ("RESTICHUB_SMTP_FROM", "restichub@example.com"),
            ("RESTICHUB_SMTP_SECURITY", "tls"),
        ])
        .expect("tls");
        assert_eq!(tls.smtp.expect("smtp").port, 465);
    }

    /// AC-MN-08: a password with `security = none` is a configuration error.
    #[test]
    fn ac_mn_08_smtp_password_needs_tls() {
        let base = [
            ("RESTICHUB_SMTP_HOST", "smtp.example.com"),
            ("RESTICHUB_SMTP_FROM", "restichub@example.com"),
            ("RESTICHUB_SMTP_USERNAME", "restichub"),
            ("RESTICHUB_SMTP_PASSWORD", "hunter2-canary"),
        ];
        let mut plain = base.to_vec();
        plain.push(("RESTICHUB_SMTP_SECURITY", "none"));
        assert!(matches!(
            load(&plain),
            Err(ConfigError::Invalid {
                name: "smtp.security",
                ..
            })
        ));
        let mut tls = base.to_vec();
        tls.push(("RESTICHUB_SMTP_SECURITY", "tls"));
        assert!(load(&tls).is_ok());
        let mut no_password = vec![
            ("RESTICHUB_SMTP_HOST", "smtp.example.com"),
            ("RESTICHUB_SMTP_FROM", "restichub@example.com"),
        ];
        no_password.push(("RESTICHUB_SMTP_SECURITY", "none"));
        assert!(load(&no_password).is_ok(), "no password, no problem");
    }

    #[test]
    fn the_notification_language_is_en_or_ru() {
        assert_eq!(load(&[]).expect("default").language, Language::En);
        assert_eq!(
            load(&[("RESTICHUB_NOTIFICATIONS_LANGUAGE", "ru")])
                .expect("ru")
                .language,
            Language::Ru
        );
        assert!(matches!(
            load(&[("RESTICHUB_NOTIFICATIONS_LANGUAGE", "fr")]),
            Err(ConfigError::Invalid {
                name: "notifications.language",
                ..
            })
        ));
    }

    /// AC-MN-08: the password comes from the environment or a file, needs a user, and stays out
    /// of the `Debug` text.
    #[test]
    fn ac_mn_08_smtp_password_stays_hidden() {
        let settings = load(&[
            ("RESTICHUB_SMTP_HOST", "smtp.example.com"),
            ("RESTICHUB_SMTP_FROM", "restichub@example.com"),
            ("RESTICHUB_SMTP_USERNAME", "restichub"),
            ("RESTICHUB_SMTP_PASSWORD", "hunter2-canary"),
        ])
        .expect("loads");
        assert!(!format!("{settings:?}").contains("hunter2-canary"));
        assert!(matches!(
            load(&[
                ("RESTICHUB_SMTP_HOST", "smtp.example.com"),
                ("RESTICHUB_SMTP_FROM", "restichub@example.com"),
                ("RESTICHUB_SMTP_PASSWORD", "x"),
            ]),
            Err(ConfigError::Invalid {
                name: "smtp.username",
                ..
            })
        ));
    }
}
