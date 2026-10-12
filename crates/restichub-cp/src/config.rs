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
use serde::Deserialize;

use crate::client_ip::TrustedProxies;

/// The name of the environment variable that holds the path of the configuration file.
pub const CONFIG_PATH_ENV: &str = "RESTICHUB_CONFIG";
/// The name of the environment variable that holds the listen address.
pub const LISTEN_ADDR_ENV: &str = "RESTICHUB_LISTEN_ADDR";
/// The name of the environment variable that holds the database URL.
pub const DATABASE_URL_ENV: &str = "RESTICHUB_DATABASE_URL";
/// The name of the environment variable that holds the path of the database password file.
pub const DATABASE_PASSWORD_FILE_ENV: &str = "RESTICHUB_DATABASE_PASSWORD_FILE";
/// The name of the environment variable that holds the database password.
pub const DATABASE_PASSWORD_ENV: &str = "RESTICHUB_DATABASE_PASSWORD";
/// The name of the environment variable that holds the URL of the migration login.
///
/// The migration login is the owner of the installation database. It is optional. Without it the
/// control plane does not run the migrations (D1).
pub const DATABASE_MIGRATION_URL_ENV: &str = "RESTICHUB_DATABASE_MIGRATION_URL";
/// The name of the environment variable that holds the path of the migration password file.
pub const DATABASE_MIGRATION_PASSWORD_FILE_ENV: &str = "RESTICHUB_DATABASE_MIGRATION_PASSWORD_FILE";
/// The name of the environment variable that holds the migration password.
pub const DATABASE_MIGRATION_PASSWORD_ENV: &str = "RESTICHUB_DATABASE_MIGRATION_PASSWORD";
/// The name of the environment variable that holds the log format.
pub const LOG_FORMAT_ENV: &str = "RESTICHUB_LOG_FORMAT";
/// The name of the environment variable that holds the log level.
pub const LOG_LEVEL_ENV: &str = "RESTICHUB_LOG_LEVEL";
/// The name of the environment variable that holds the trusted proxy list.
///
/// The value is a comma-separated list of networks, for example `10.0.0.0/8,192.168.0.0/16`.
pub const TRUSTED_PROXIES_ENV: &str = "RESTICHUB_TRUSTED_PROXIES";

const DEFAULT_LISTEN_ADDR: &str = "127.0.0.1:8080";
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

/// The command of the control plane binary.
#[derive(Debug, Clone)]
pub enum Command {
    /// Print the version and stop.
    Version,
    /// Run the server. This is the default command.
    Serve(Box<Cli>),
    /// Apply the database migrations and stop (N4).
    Migrate(Box<Cli>),
}

/// The command-line arguments of the control plane.
#[derive(Debug, Default, Clone)]
pub struct Cli {
    /// The path of the configuration file (`--config`).
    pub config_path: Option<PathBuf>,
}

/// The log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Human-readable text.
    Text,
    /// JSON objects.
    Json,
}

/// The error of the configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// An unknown command-line argument.
    #[error("the argument `{0}` is not known")]
    UnknownArgument(String),
    /// The `--config` argument has no value.
    #[error("the argument `--config` needs a path")]
    ConfigPathMissing,
    /// The configuration file cannot be read.
    #[error("cannot read the configuration file `{path}`")]
    ReadFile {
        /// The path of the file.
        path: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
    /// The configuration file cannot be parsed.
    #[error("cannot parse the configuration file `{path}`")]
    ParseFile {
        /// The path of the file.
        path: PathBuf,
        /// The cause.
        #[source]
        source: toml::de::Error,
    },
    /// A parameter has a value that is not valid.
    #[error("the parameter `{name}` is not valid: {reason}")]
    Invalid {
        /// The name of the parameter.
        name: &'static str,
        /// The reason.
        reason: String,
    },
    /// The database URL holds a password and a password file is also set.
    #[error(
        "the parameter `database_url` holds a password, but `database_password_file` is also set"
    )]
    PasswordFileWithUrlPassword,
    /// The migration URL holds a password and a migration password file is also set.
    #[error(
        "the parameter `database_migration_url` holds a password, but `database_migration_password_file` is also set"
    )]
    MigrationPasswordFileWithUrlPassword,
    /// A secret read failed.
    #[error(transparent)]
    Secret(#[from] restichub_secrets::SecretError),
}

/// The configuration of the control plane at run time.
///
/// A `Debug` of this type shows no secret. The field `database_password` hides the value, and the
/// field `database_url` holds no password.
#[derive(Debug)]
pub struct Config {
    /// The address the HTTP server listens on.
    pub listen_addr: SocketAddr,
    /// The database connection URL. The URL holds no password.
    pub database_url: String,
    /// The database password from a file or from the environment.
    pub database_password: Option<SecretString>,
    /// The maximum number of database connections.
    pub database_max_connections: u32,
    /// The time to wait for a database connection.
    pub database_acquire_timeout: Duration,
    /// The log output format.
    pub log_format: LogFormat,
    /// The tracing filter.
    pub log_level: String,
    /// The networks whose `X-Forwarded-For` header the control plane trusts.
    pub trusted_proxies: TrustedProxies,
}

impl Config {
    /// Loads the configuration from the process environment.
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

        let trusted_proxies = trusted_proxies(env, file.server)?;

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

/// The configuration file. Each field is optional so that the environment can override it.
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
}

/// The `[server]` section of the configuration file.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerConfig {
    trusted_proxies: Option<Vec<String>>,
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
/// The environment overrides the file. The environment value is a comma-separated list.
fn trusted_proxies(
    env: &dyn EnvReader,
    server: Option<ServerConfig>,
) -> Result<TrustedProxies, ConfigError> {
    let entries: Vec<String> = match env.get(TRUSTED_PROXIES_ENV) {
        Some(value) => value
            .split(',')
            .map(|entry| entry.trim().to_owned())
            .collect(),
        None => server
            .and_then(|server| server.trusted_proxies)
            .unwrap_or_default(),
    };
    TrustedProxies::parse(entries.iter().map(String::as_str)).map_err(|reason| {
        ConfigError::Invalid {
            name: "server.trusted_proxies",
            reason,
        }
    })
}

/// The configuration of the `migrate` command.
///
/// The command reads only the migration URL and the migration password. The server path does not
/// accept migration credentials (N4).
#[derive(Debug)]
pub struct MigrationConfig {
    /// The connection URL of the owner login. The URL holds no password.
    pub url: String,
    /// The password of the owner login from a file or from the environment.
    pub password: Option<SecretString>,
}

impl MigrationConfig {
    /// Loads the migration configuration from the process environment.
    pub fn load(cli: &Cli) -> Result<Self, ConfigError> {
        Self::load_with(cli, &SystemEnv)
    }

    /// Loads the migration configuration from a given environment. A test uses this function.
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
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use restichub_secrets::EnvReader;

    use super::{Cli, Command, Config, ConfigError, LogFormat, MigrationConfig, parse_args};

    /// A test environment.
    struct MapEnv(HashMap<String, String>);

    impl MapEnv {
        fn new(pairs: &[(&str, &str)]) -> Self {
            let map = pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect();
            Self(map)
        }
    }

    impl EnvReader for MapEnv {
        fn get(&self, name: &str) -> Option<String> {
            self.0.get(name).cloned()
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "restichub-cp-config-{}-{name}.toml",
            std::process::id()
        ));
        path
    }

    #[test]
    fn defaults_apply_without_a_file() {
        let env = MapEnv::new(&[("RESTICHUB_DATABASE_URL", "postgres://host/db")]);
        let config = Config::load_with(&Cli::default(), &env);
        assert_eq!(
            config.ok().map(|config| config.listen_addr.to_string()),
            Some("127.0.0.1:8080".to_owned())
        );
    }

    #[test]
    fn database_url_is_required() {
        let env = MapEnv::new(&[]);
        let error = Config::load_with(&Cli::default(), &env);
        assert!(matches!(
            error,
            Err(ConfigError::Invalid {
                name: "database_url",
                ..
            })
        ));
    }

    #[test]
    fn environment_overrides_the_file() {
        let path = temp_path("override");
        std::fs::write(
            &path,
            "listen_addr = \"127.0.0.1:1111\"\ndatabase_url = \"postgres://file/db\"\n",
        )
        .ok();
        let env = MapEnv::new(&[
            ("RESTICHUB_CONFIG", path.to_str().unwrap_or_default()),
            ("RESTICHUB_LISTEN_ADDR", "0.0.0.0:2222"),
        ]);
        let config = Config::load_with(&Cli::default(), &env);
        std::fs::remove_file(&path).ok();
        assert_eq!(
            config.ok().map(|config| config.listen_addr.to_string()),
            Some("0.0.0.0:2222".to_owned())
        );
    }

    #[test]
    fn the_file_gives_the_database_url() {
        let path = temp_path("file-url");
        std::fs::write(&path, "database_url = \"postgres://file/db\"\n").ok();
        let env = MapEnv::new(&[("RESTICHUB_CONFIG", path.to_str().unwrap_or_default())]);
        let config = Config::load_with(&Cli::default(), &env);
        std::fs::remove_file(&path).ok();
        assert_eq!(
            config.ok().map(|config| config.database_url),
            Some("postgres://file/db".to_owned())
        );
    }

    #[test]
    fn a_cli_path_wins_over_the_environment() {
        let cli_path = temp_path("cli");
        let env_path = temp_path("env");
        std::fs::write(&cli_path, "database_url = \"postgres://cli/db\"\n").ok();
        std::fs::write(&env_path, "database_url = \"postgres://env/db\"\n").ok();
        let env = MapEnv::new(&[("RESTICHUB_CONFIG", env_path.to_str().unwrap_or_default())]);
        let cli = Cli {
            config_path: Some(cli_path.clone()),
        };
        let config = Config::load_with(&cli, &env);
        std::fs::remove_file(&cli_path).ok();
        std::fs::remove_file(&env_path).ok();
        assert_eq!(
            config.ok().map(|config| config.database_url),
            Some("postgres://cli/db".to_owned())
        );
    }

    #[test]
    fn the_url_password_moves_out_of_the_url() {
        let env = MapEnv::new(&[("RESTICHUB_DATABASE_URL", "postgres://user:secret@host/db")]);
        let config = Config::load_with(&Cli::default(), &env);
        let config = config.ok().map(|config| {
            (
                config.database_url.clone(),
                config
                    .database_password
                    .as_ref()
                    .map(|password| password.expose_secret().to_owned()),
            )
        });
        assert_eq!(
            config,
            Some((
                "postgres://user@host/db".to_owned(),
                Some("secret".to_owned())
            ))
        );
    }

    #[test]
    fn a_password_file_with_a_url_password_is_an_error() {
        let password_file = temp_path("pw");
        std::fs::write(&password_file, "from-file").ok();
        let env = MapEnv::new(&[
            ("RESTICHUB_DATABASE_URL", "postgres://user:secret@host/db"),
            (
                "RESTICHUB_DATABASE_PASSWORD_FILE",
                password_file.to_str().unwrap_or_default(),
            ),
        ]);
        let error = Config::load_with(&Cli::default(), &env);
        std::fs::remove_file(&password_file).ok();
        assert!(matches!(
            error,
            Err(ConfigError::PasswordFileWithUrlPassword)
        ));
    }

    #[test]
    fn a_bad_listen_addr_names_the_parameter() {
        let env = MapEnv::new(&[
            ("RESTICHUB_DATABASE_URL", "postgres://host/db"),
            ("RESTICHUB_LISTEN_ADDR", "not-an-address"),
        ]);
        let error = Config::load_with(&Cli::default(), &env);
        assert!(matches!(
            error,
            Err(ConfigError::Invalid {
                name: "listen_addr",
                ..
            })
        ));
    }

    #[test]
    fn a_bad_log_format_names_the_parameter() {
        let env = MapEnv::new(&[
            ("RESTICHUB_DATABASE_URL", "postgres://host/db"),
            ("RESTICHUB_LOG_FORMAT", "yaml"),
        ]);
        let error = Config::load_with(&Cli::default(), &env);
        assert!(matches!(
            error,
            Err(ConfigError::Invalid {
                name: "log_format",
                ..
            })
        ));
    }

    #[test]
    fn the_log_format_from_the_environment_is_json() {
        let env = MapEnv::new(&[
            ("RESTICHUB_DATABASE_URL", "postgres://host/db"),
            ("RESTICHUB_LOG_FORMAT", "json"),
        ]);
        let config = Config::load_with(&Cli::default(), &env);
        assert_eq!(
            config.ok().map(|config| config.log_format),
            Some(LogFormat::Json)
        );
    }

    #[test]
    fn parse_version_argument() {
        let command = parse_args(["--version".to_owned()]);
        assert!(matches!(command, Ok(Command::Version)));
    }

    #[test]
    fn parse_config_argument() {
        let command = parse_args(["--config".to_owned(), "/etc/restichub.toml".to_owned()]);
        assert!(
            matches!(command, Ok(Command::Serve(cli)) if cli.config_path == Some(PathBuf::from("/etc/restichub.toml")))
        );
    }

    #[test]
    fn parse_migrate_subcommand() {
        let command = parse_args([
            "migrate".to_owned(),
            "--config".to_owned(),
            "/etc/restichub.toml".to_owned(),
        ]);
        assert!(
            matches!(command, Ok(Command::Migrate(cli)) if cli.config_path == Some(PathBuf::from("/etc/restichub.toml")))
        );
    }

    #[test]
    fn parse_serve_subcommand_is_the_default() {
        let command = parse_args(["serve".to_owned()]);
        assert!(matches!(command, Ok(Command::Serve(_))));
        let command = parse_args(std::iter::empty::<String>());
        assert!(matches!(command, Ok(Command::Serve(_))));
    }

    #[test]
    fn parse_unknown_argument() {
        let command = parse_args(["--nope".to_owned()]);
        assert!(matches!(command, Err(ConfigError::UnknownArgument(_))));
    }

    #[test]
    fn the_migration_url_is_required_for_the_migrate_command() {
        let env = MapEnv::new(&[("RESTICHUB_DATABASE_URL", "postgres://host/db")]);
        let config = MigrationConfig::load_with(&Cli::default(), &env);
        assert!(matches!(
            config,
            Err(ConfigError::Invalid {
                name: "database_migration_url",
                ..
            })
        ));
    }

    #[test]
    fn the_migration_url_moves_the_password_out_of_the_url() {
        let env = MapEnv::new(&[(
            "RESTICHUB_DATABASE_MIGRATION_URL",
            "postgres://owner:secret@host/db",
        )]);
        let config = MigrationConfig::load_with(&Cli::default(), &env);
        let pair = config.ok().map(|config| {
            (
                config.url,
                config
                    .password
                    .as_ref()
                    .map(|password| password.expose_secret().to_owned()),
            )
        });
        assert_eq!(
            pair,
            Some((
                "postgres://owner@host/db".to_owned(),
                Some("secret".to_owned())
            ))
        );
    }

    #[test]
    fn a_migration_password_file_with_a_url_password_is_an_error() {
        let password_file = temp_path("migration-pw");
        std::fs::write(&password_file, "from-file").ok();
        let env = MapEnv::new(&[
            (
                "RESTICHUB_DATABASE_MIGRATION_URL",
                "postgres://owner:secret@host/db",
            ),
            (
                "RESTICHUB_DATABASE_MIGRATION_PASSWORD_FILE",
                password_file.to_str().unwrap_or_default(),
            ),
        ]);
        let error = MigrationConfig::load_with(&Cli::default(), &env);
        std::fs::remove_file(&password_file).ok();
        assert!(matches!(
            error,
            Err(ConfigError::MigrationPasswordFileWithUrlPassword)
        ));
    }

    #[test]
    fn the_server_config_holds_no_migration_settings() {
        // The server path does not read migration credentials (N4).
        let env = MapEnv::new(&[
            ("RESTICHUB_DATABASE_URL", "postgres://host/db"),
            (
                "RESTICHUB_DATABASE_MIGRATION_URL",
                "postgres://owner@host/db",
            ),
        ]);
        let config = Config::load_with(&Cli::default(), &env);
        assert!(config.is_ok());
    }

    #[test]
    fn debug_hides_the_database_password() {
        let env = MapEnv::new(&[("RESTICHUB_DATABASE_URL", "postgres://user:secret@host/db")]);
        let config = Config::load_with(&Cli::default(), &env);
        let text = format!("{config:?}");
        assert!(!text.contains("secret"), "{text}");
    }

    #[test]
    fn the_trusted_proxy_list_comes_from_the_environment() {
        let env = MapEnv::new(&[
            ("RESTICHUB_DATABASE_URL", "postgres://host/db"),
            ("RESTICHUB_TRUSTED_PROXIES", "10.0.0.0/8, 192.168.0.0/16"),
        ]);
        let config = Config::load_with(&Cli::default(), &env);
        let address = "10.1.2.3"
            .parse()
            .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
        assert!(config.is_ok_and(|config| config.trusted_proxies.contains(address)));
    }

    #[test]
    fn a_bad_trusted_proxy_list_names_the_parameter() {
        let env = MapEnv::new(&[
            ("RESTICHUB_DATABASE_URL", "postgres://host/db"),
            ("RESTICHUB_TRUSTED_PROXIES", "not-a-network"),
        ]);
        let error = Config::load_with(&Cli::default(), &env);
        assert!(matches!(
            error,
            Err(ConfigError::Invalid {
                name: "server.trusted_proxies",
                ..
            })
        ));
    }
}
