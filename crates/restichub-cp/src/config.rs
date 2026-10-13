//! The configuration of the control plane (ADR-0023, section Configuration).
//!
//! The control plane reads a TOML file and the environment variables with the prefix
//! `RESTICHUB_`. The environment variables override the file values. The database password and
//! the key encryption key come from a file or from the environment, never from a command-line
//! argument.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use restichub_secrets::{EnvReader, SecretString, SystemEnv, read_secret};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::client_ip::TrustedProxies;

pub const CONFIG_PATH_ENV: &str = "RESTICHUB_CONFIG";
pub const LISTEN_ADDR_ENV: &str = "RESTICHUB_LISTEN_ADDR";
pub const DATABASE_URL_ENV: &str = "RESTICHUB_DATABASE_URL";
pub const DATABASE_PASSWORD_FILE_ENV: &str = "RESTICHUB_DATABASE_PASSWORD_FILE";
pub const DATABASE_PASSWORD_ENV: &str = "RESTICHUB_DATABASE_PASSWORD";
/// The migration login is the owner of the installation database. It is optional. Without it the
/// control plane does not run the migrations.
pub const DATABASE_MIGRATION_URL_ENV: &str = "RESTICHUB_DATABASE_MIGRATION_URL";
pub const DATABASE_MIGRATION_PASSWORD_FILE_ENV: &str = "RESTICHUB_DATABASE_MIGRATION_PASSWORD_FILE";
pub const DATABASE_MIGRATION_PASSWORD_ENV: &str = "RESTICHUB_DATABASE_MIGRATION_PASSWORD";
/// The key encryption key (KEK) comes from this file or from the environment variable
/// `RESTICHUB_KEK` ([`restichub_secrets::KEK_ENV`]). If both are set, the start fails (ADR-0023,
/// section Secrets).
pub const KEK_FILE_ENV: &str = "RESTICHUB_KEK_FILE";
pub const LOG_FORMAT_ENV: &str = "RESTICHUB_LOG_FORMAT";
pub const LOG_LEVEL_ENV: &str = "RESTICHUB_LOG_LEVEL";
/// The value is a comma-separated list of networks, for example `10.0.0.0/8,192.168.0.0/16`.
pub const TRUSTED_PROXIES_ENV: &str = "RESTICHUB_TRUSTED_PROXIES";
pub const SESSION_IDLE_ENV: &str = "RESTICHUB_AUTH_SESSION_IDLE_SECS";
pub const SESSION_ABSOLUTE_ENV: &str = "RESTICHUB_AUTH_SESSION_ABSOLUTE_SECS";
pub const SIGN_IN_USER_RATE_ENV: &str = "RESTICHUB_AUTH_SIGN_IN_USER_RATE";
pub const SIGN_IN_IP_RATE_ENV: &str = "RESTICHUB_AUTH_SIGN_IN_IP_RATE";
pub const INSECURE_COOKIES_ENV: &str = "RESTICHUB_SERVER_INSECURE_COOKIES";
pub const MAX_CONCURRENT_HASHES_ENV: &str = "RESTICHUB_AUTH_MAX_CONCURRENT_HASHES";
/// The gRPC endpoint of the agent listener, reported to an agent at enrollment (ADR-0026).
pub const AGENT_ENDPOINT_ENV: &str = "RESTICHUB_AGENT_ENDPOINT";
/// The tenancy mode: `single` or `multi` (ADR-0024).
pub const TENANCY_MODE_ENV: &str = "RESTICHUB_TENANCY_MODE";
/// The tenant that single mode uses and that the switch from multi mode expects.
pub const TENANCY_DEFAULT_TENANT_ENV: &str = "RESTICHUB_TENANCY_DEFAULT_TENANT";

const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:8080";
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_SESSION_IDLE: Duration = Duration::from_secs(30 * 60);
const DEFAULT_SESSION_ABSOLUTE: Duration = Duration::from_secs(12 * 60 * 60);
const DEFAULT_SIGN_IN_USER_RATE: u32 = 10;
const DEFAULT_SIGN_IN_IP_RATE: u32 = 30;
const DEFAULT_TENANT_NAME: &str = "default";

/// The tenancy mode of the installation (ADR-0024).
///
/// The mode changes the API surface and the user interface. It never changes the storage model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TenancyMode {
    #[default]
    Single,
    Multi,
}

impl TenancyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Multi => "multi",
        }
    }

    pub fn is_single(self) -> bool {
        matches!(self, Self::Single)
    }

    /// `None` on an unknown string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "single" => Some(Self::Single),
            "multi" => Some(Self::Multi),
            _ => None,
        }
    }
}

/// The tenancy mode of the process and the name of the tenant that single mode uses (ADR-0024).
#[derive(Debug, Clone)]
pub struct TenancyConfig {
    pub mode: TenancyMode,
    /// The tenant that single mode uses and that the switch from multi mode expects.
    pub default_tenant: String,
}

impl TenancyConfig {
    pub fn single(default_tenant: impl Into<String>) -> Self {
        Self {
            mode: TenancyMode::Single,
            default_tenant: default_tenant.into(),
        }
    }

    pub fn multi() -> Self {
        Self {
            mode: TenancyMode::Multi,
            default_tenant: DEFAULT_TENANT_NAME.to_owned(),
        }
    }
}

impl Default for TenancyConfig {
    fn default() -> Self {
        Self::single(DEFAULT_TENANT_NAME)
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    Version,
    Serve(Box<Cli>),
    /// Apply the database migrations and stop.
    Migrate(Box<Cli>),
}

#[derive(Debug, Default, Clone)]
pub struct Cli {
    pub config_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Text,
    Json,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("the argument `{0}` is not known")]
    UnknownArgument(String),
    #[error("the argument `--config` needs a path")]
    ConfigPathMissing,
    #[error("cannot read the configuration file `{path}`")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot parse the configuration file `{path}`")]
    ParseFile {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("the parameter `{name}` is not valid: {reason}")]
    Invalid { name: &'static str, reason: String },
    #[error(
        "the parameter `database_url` holds a password, but `database_password_file` is also set"
    )]
    PasswordFileWithUrlPassword,
    #[error(
        "the parameter `database_migration_url` holds a password, but `database_migration_password_file` is also set"
    )]
    MigrationPasswordFileWithUrlPassword,
    #[error(transparent)]
    Secret(#[from] restichub_secrets::SecretError),
}

/// Local authentication configuration (ADR-0023, section Local authentication).
#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub session_idle: Duration,
    pub session_absolute: Duration,
    /// Sign-in attempts for each `(tenant, username)` pair in one minute.
    pub sign_in_user_rate: u32,
    /// Sign-in attempts for each source IP address in one minute.
    pub sign_in_ip_rate: u32,
    /// Turns off the `Secure` attribute and the `__Host-` prefix of the session cookie.
    ///
    /// The value is for local development over plain HTTP. It is a loud risk.
    pub insecure_cookies: bool,
    /// The largest number of concurrent password hashes.
    ///
    /// The default is the number of available CPUs. Argon2id uses about 19 MiB of memory for each
    /// hash, so the semaphore bounds the memory of the process.
    pub max_concurrent_hashes: u32,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            session_idle: DEFAULT_SESSION_IDLE,
            session_absolute: DEFAULT_SESSION_ABSOLUTE,
            sign_in_user_rate: DEFAULT_SIGN_IN_USER_RATE,
            sign_in_ip_rate: DEFAULT_SIGN_IN_IP_RATE,
            insecure_cookies: false,
            max_concurrent_hashes: default_max_concurrent_hashes(),
        }
    }
}

fn default_max_concurrent_hashes() -> u32 {
    std::thread::available_parallelism()
        .map(|count| count.get() as u32)
        .unwrap_or(4)
        .max(1)
}

/// The configuration of the control plane at run time.
///
/// A `Debug` of this type shows no secret. The field `database_password` hides the value, and the
/// field `database_url` holds no password.
#[derive(Debug)]
pub struct Config {
    pub listen_addr: SocketAddr,
    /// Holds no password.
    pub database_url: String,
    pub database_password: Option<SecretString>,
    pub database_max_connections: u32,
    pub database_acquire_timeout: Duration,
    pub log_format: LogFormat,
    pub log_level: String,
    pub trusted_proxies: TrustedProxies,
    pub auth: AuthConfig,
    pub secrets: SecretsConfig,
    pub tenancy: TenancyConfig,
    /// Reported to an agent at enrollment; the address of the agent listener (ADR-0026).
    pub agent_endpoint: Option<String>,
}

/// Secret handling configuration (ADR-0023, section Secrets).
#[derive(Debug, Clone, Default)]
pub struct SecretsConfig {
    /// The path of the file that holds the key encryption key (KEK).
    ///
    /// If the path is not set, the KEK comes from the environment variable `RESTICHUB_KEK`. If both
    /// are set, the start fails.
    pub kek_file: Option<PathBuf>,
}

impl Config {
    pub fn load(cli: &Cli) -> Result<Self, ConfigError> {
        Self::load_with(cli, &SystemEnv)
    }

    /// Loads the configuration from a given environment. A test uses this function.
    ///
    /// The precedence is: the default value, then the configuration file, then the environment.
    pub fn load_with(cli: &Cli, env: &dyn EnvReader) -> Result<Self, ConfigError> {
        let file_path = cli
            .config_path
            .clone()
            .or_else(|| env.get(CONFIG_PATH_ENV).map(PathBuf::from));
        let file = match file_path {
            Some(path) => FileConfig::from_path(&path)?,
            None => FileConfig::default(),
        };

        let listen_addr = listen_addr(
            env.get(LISTEN_ADDR_ENV)
                .or(file.listen_addr)
                .unwrap_or_else(|| DEFAULT_LISTEN_ADDR.to_owned()),
        )?;

        let log_format = log_format(env.get(LOG_FORMAT_ENV).or(file.log_format).as_deref())?;

        let log_level = env
            .get(LOG_LEVEL_ENV)
            .or(file.log_level)
            .unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_owned());

        let trusted_proxies = trusted_proxies(env, file.server.as_ref())?;
        let auth = auth_config(env, file.auth.as_ref(), file.server.as_ref())?;
        let secrets = secrets_config(env, file.secrets.as_ref());
        let tenancy = tenancy_config(env, file.tenancy.as_ref())?;
        let agent_endpoint = env.get(AGENT_ENDPOINT_ENV).or(file.agent_endpoint);

        let database_url = env
            .get(DATABASE_URL_ENV)
            .or(file.database_url)
            .ok_or_else(|| ConfigError::Invalid {
                name: "database_url",
                reason: "the value is not set".to_owned(),
            })?;

        let password_file = env
            .get(DATABASE_PASSWORD_FILE_ENV)
            .map(PathBuf::from)
            .or(file.database_password_file);

        let (database_url, url_password) = split_url_password(&database_url);
        if password_file.is_some() && url_password.is_some() {
            return Err(ConfigError::PasswordFileWithUrlPassword);
        }

        let password = if password_file.is_some() || env.get(DATABASE_PASSWORD_ENV).is_some() {
            Some(read_secret(
                password_file.as_deref(),
                DATABASE_PASSWORD_ENV,
                env,
            )?)
        } else {
            None
        };

        let database_password = match (password, url_password) {
            (Some(password), None) => Some(password),
            (None, Some(password)) => Some(SecretString::new(password)),
            (Some(_), Some(_)) => {
                return Err(ConfigError::Invalid {
                    name: "database_url",
                    reason: "a password is set in the URL and in the environment".to_owned(),
                });
            }
            (None, None) => None,
        };

        Ok(Self {
            listen_addr,
            database_url,
            database_password,
            database_max_connections: file
                .database_max_connections
                .unwrap_or(DEFAULT_MAX_CONNECTIONS),
            database_acquire_timeout: file
                .database_acquire_timeout_secs
                .map(Duration::from_secs)
                .unwrap_or(DEFAULT_ACQUIRE_TIMEOUT),
            log_format,
            log_level,
            trusted_proxies,
            auth,
            secrets,
            tenancy,
            agent_endpoint,
        })
    }
}

/// Parses the command-line arguments.
///
/// The function reads the subcommands `serve` (the default) and `migrate`, and the arguments
/// `--config <path>`, `--config=<path>`, `--version` and `-V`.
pub fn parse_args<I>(args: I) -> Result<Command, ConfigError>
where
    I: IntoIterator<Item = String>,
{
    let mut config_path = None;
    let mut migrate = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" | "-V" => return Ok(Command::Version),
            "serve" => migrate = false,
            "migrate" => migrate = true,
            "--config" => {
                let value = args.next().ok_or(ConfigError::ConfigPathMissing)?;
                config_path = Some(PathBuf::from(value));
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--config=") {
                    config_path = Some(PathBuf::from(value));
                } else {
                    return Err(ConfigError::UnknownArgument(arg));
                }
            }
        }
    }
    let cli = Box::new(Cli { config_path });
    Ok(if migrate {
        Command::Migrate(cli)
    } else {
        Command::Serve(cli)
    })
}

/// Each field is optional so that the environment can override it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    listen_addr: Option<String>,
    database_url: Option<String>,
    database_password_file: Option<PathBuf>,
    database_migration_url: Option<String>,
    database_migration_password_file: Option<PathBuf>,
    database_max_connections: Option<u32>,
    database_acquire_timeout_secs: Option<u64>,
    log_format: Option<String>,
    log_level: Option<String>,
    server: Option<ServerConfig>,
    auth: Option<AuthSection>,
    secrets: Option<SecretsSection>,
    tenancy: Option<TenancySection>,
    agent_endpoint: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerConfig {
    trusted_proxies: Option<Vec<String>>,
    insecure_cookies: Option<bool>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthSection {
    session_idle_secs: Option<u64>,
    session_absolute_secs: Option<u64>,
    sign_in_user_rate: Option<u32>,
    sign_in_ip_rate: Option<u32>,
    max_concurrent_hashes: Option<u32>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretsSection {
    kek_file: Option<PathBuf>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TenancySection {
    mode: Option<TenancyMode>,
    default_tenant: Option<String>,
}

impl FileConfig {
    fn from_path(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| ConfigError::ParseFile {
            path: path.to_path_buf(),
            source,
        })
    }
}

fn listen_addr(value: String) -> Result<SocketAddr, ConfigError> {
    value.parse().map_err(|_| ConfigError::Invalid {
        name: "listen_addr",
        reason: format!("`{value}` is not a socket address"),
    })
}

fn log_format(value: Option<&str>) -> Result<LogFormat, ConfigError> {
    match value {
        None | Some("text") => Ok(LogFormat::Text),
        Some("json") => Ok(LogFormat::Json),
        Some(value) => Err(ConfigError::Invalid {
            name: "log_format",
            reason: format!("`{value}` is not `text` or `json`"),
        }),
    }
}

/// Reads the trusted proxy list (ADR-0023, section Audit log).
///
/// Environment values override the file. The environment value is a comma-separated list.
fn trusted_proxies(
    env: &dyn EnvReader,
    server: Option<&ServerConfig>,
) -> Result<TrustedProxies, ConfigError> {
    let entries: Vec<String> = match env.get(TRUSTED_PROXIES_ENV) {
        Some(value) => value
            .split(',')
            .map(|entry| entry.trim().to_owned())
            .collect(),
        None => server
            .and_then(|server| server.trusted_proxies.clone())
            .unwrap_or_default(),
    };
    TrustedProxies::parse(entries.iter().map(String::as_str)).map_err(|reason| {
        ConfigError::Invalid {
            name: "server.trusted_proxies",
            reason,
        }
    })
}

/// Reads the local authentication settings.
///
/// Environment values override the file.
fn auth_config(
    env: &dyn EnvReader,
    auth: Option<&AuthSection>,
    server: Option<&ServerConfig>,
) -> Result<AuthConfig, ConfigError> {
    let defaults = AuthConfig::default();
    let session_idle_secs = number(
        env,
        SESSION_IDLE_ENV,
        auth.and_then(|a| a.session_idle_secs),
    )?
    .unwrap_or(defaults.session_idle.as_secs());
    let session_absolute_secs = number(
        env,
        SESSION_ABSOLUTE_ENV,
        auth.and_then(|a| a.session_absolute_secs),
    )?
    .unwrap_or(defaults.session_absolute.as_secs());
    let sign_in_user_rate = number(
        env,
        SIGN_IN_USER_RATE_ENV,
        auth.and_then(|a| a.sign_in_user_rate),
    )?
    .unwrap_or(defaults.sign_in_user_rate);
    let sign_in_ip_rate = number(
        env,
        SIGN_IN_IP_RATE_ENV,
        auth.and_then(|a| a.sign_in_ip_rate),
    )?
    .unwrap_or(defaults.sign_in_ip_rate);
    let insecure_cookies = boolean(
        env,
        INSECURE_COOKIES_ENV,
        server.and_then(|s| s.insecure_cookies),
    )?
    .unwrap_or(false);
    let max_concurrent_hashes = number(
        env,
        MAX_CONCURRENT_HASHES_ENV,
        auth.and_then(|a| a.max_concurrent_hashes),
    )?
    .unwrap_or(defaults.max_concurrent_hashes)
    .max(1);
    if session_idle_secs == 0 || session_absolute_secs == 0 {
        return Err(ConfigError::Invalid {
            name: "auth.session_idle_secs",
            reason: "the value must be at least 1".to_owned(),
        });
    }
    Ok(AuthConfig {
        session_idle: Duration::from_secs(session_idle_secs),
        session_absolute: Duration::from_secs(session_absolute_secs),
        sign_in_user_rate,
        sign_in_ip_rate,
        insecure_cookies,
        max_concurrent_hashes,
    })
}

/// Reads the secret handling settings.
///
/// Environment values override the file. The function does not reject both sources: the KEK provider
/// [`restichub_secrets::load_kek_provider`] refuses a file and the environment variable at the same
/// time with a clear cause.
fn secrets_config(env: &dyn EnvReader, secrets: Option<&SecretsSection>) -> SecretsConfig {
    let kek_file = env
        .get(KEK_FILE_ENV)
        .map(PathBuf::from)
        .or_else(|| secrets.and_then(|section| section.kek_file.clone()));
    SecretsConfig { kek_file }
}

/// Reads the tenancy settings (ADR-0024).
///
/// Environment values override the file. The mode default is `single`. The name of the default
/// tenant obeys the tenant name rule, so the reserved name `platform` is refused.
fn tenancy_config(
    env: &dyn EnvReader,
    tenancy: Option<&TenancySection>,
) -> Result<TenancyConfig, ConfigError> {
    let mode = match env.get(TENANCY_MODE_ENV) {
        Some(value) => TenancyMode::parse(&value).ok_or_else(|| ConfigError::Invalid {
            name: "tenancy.mode",
            reason: format!("`{value}` is not `single` or `multi`"),
        })?,
        None => tenancy.and_then(|section| section.mode).unwrap_or_default(),
    };
    let default_tenant = env
        .get(TENANCY_DEFAULT_TENANT_ENV)
        .or_else(|| tenancy.and_then(|section| section.default_tenant.clone()))
        .unwrap_or_else(|| DEFAULT_TENANT_NAME.to_owned());
    let default_tenant =
        crate::validation::validate_tenant_name("tenancy.default_tenant", &default_tenant)
            .map_err(|error| ConfigError::Invalid {
                name: "tenancy.default_tenant",
                reason: error.reason.as_str().to_owned(),
            })?;
    Ok(TenancyConfig {
        mode,
        default_tenant,
    })
}

fn number<T: std::str::FromStr>(
    env: &dyn EnvReader,
    name: &'static str,
    file: Option<T>,
) -> Result<Option<T>, ConfigError> {
    match env.get(name) {
        Some(value) => match value.trim().parse::<T>() {
            Ok(parsed) => Ok(Some(parsed)),
            Err(_) => Err(ConfigError::Invalid {
                name,
                reason: format!("`{value}` is not a number"),
            }),
        },
        None => Ok(file),
    }
}

fn boolean(
    env: &dyn EnvReader,
    name: &'static str,
    file: Option<bool>,
) -> Result<Option<bool>, ConfigError> {
    match env.get(name) {
        Some(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(Some(true)),
            "0" | "false" | "no" | "off" => Ok(Some(false)),
            _ => Err(ConfigError::Invalid {
                name,
                reason: format!("`{value}` is not a boolean"),
            }),
        },
        None => Ok(file),
    }
}

/// The command reads only the migration URL and the migration password. The server path does not
/// accept migration credentials.
#[derive(Debug)]
pub struct MigrationConfig {
    /// Holds no password.
    pub url: String,
    pub password: Option<SecretString>,
}

impl MigrationConfig {
    pub fn load(cli: &Cli) -> Result<Self, ConfigError> {
        Self::load_with(cli, &SystemEnv)
    }

    pub fn load_with(cli: &Cli, env: &dyn EnvReader) -> Result<Self, ConfigError> {
        let file_path = cli
            .config_path
            .clone()
            .or_else(|| env.get(CONFIG_PATH_ENV).map(PathBuf::from));
        let file = match file_path {
            Some(path) => FileConfig::from_path(&path)?,
            None => FileConfig::default(),
        };
        let url = env
            .get(DATABASE_MIGRATION_URL_ENV)
            .or(file.database_migration_url)
            .ok_or_else(|| ConfigError::Invalid {
                name: "database_migration_url",
                reason: "the value is not set".to_owned(),
            })?;
        let password_file = env
            .get(DATABASE_MIGRATION_PASSWORD_FILE_ENV)
            .map(PathBuf::from)
            .or(file.database_migration_password_file);
        let (url, url_password) = split_url_password(&url);
        if password_file.is_some() && url_password.is_some() {
            return Err(ConfigError::MigrationPasswordFileWithUrlPassword);
        }
        let password =
            if password_file.is_some() || env.get(DATABASE_MIGRATION_PASSWORD_ENV).is_some() {
                Some(read_secret(
                    password_file.as_deref(),
                    DATABASE_MIGRATION_PASSWORD_ENV,
                    env,
                )?)
            } else {
                None
            };
        let password = match (password, url_password) {
            (Some(password), None) => Some(password),
            (None, Some(password)) => Some(SecretString::new(password)),
            (Some(_), Some(_)) => {
                return Err(ConfigError::Invalid {
                    name: "database_migration_url",
                    reason: "a password is set in the URL and in the environment".to_owned(),
                });
            }
            (None, None) => None,
        };
        Ok(Self { url, password })
    }
}

/// Removes the password from a URL and returns the URL and the password.
///
/// The control plane keeps the password out of `Debug` output. The function returns the URL
/// without a change if the URL holds no password.
fn split_url_password(url: &str) -> (String, Option<String>) {
    let Some(scheme_end) = url.find("://") else {
        return (url.to_owned(), None);
    };
    let authority_start = scheme_end + 3;
    let rest = &url[authority_start..];
    let Some(at) = rest.find('@') else {
        return (url.to_owned(), None);
    };
    let userinfo = &rest[..at];
    let Some(colon) = userinfo.find(':') else {
        return (url.to_owned(), None);
    };
    let user = &userinfo[..colon];
    let password = &userinfo[colon + 1..];
    if password.is_empty() {
        return (url.to_owned(), None);
    }
    let without_password = format!("{}{user}{}", &url[..authority_start], &rest[at..]);
    (without_password, Some(password.to_owned()))
}

#[cfg(test)]
mod tests;
