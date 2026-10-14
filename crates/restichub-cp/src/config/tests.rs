use std::collections::HashMap;
use std::path::PathBuf;

use restichub_secrets::EnvReader;

use super::{
    Cli, Command, Config, ConfigError, LogFormat, MigrationConfig, TenancyMode, parse_args,
};

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
    // The server path does not read migration credentials.
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
    // The password is a canary value. The test checks that no secret VALUE appears in the
    // `Debug` output; the field names are not secret.
    let canary = "canary-db-password-3f9a";
    let env = MapEnv::new(&[(
        "RESTICHUB_DATABASE_URL",
        &format!("postgres://user:{canary}@host/db"),
    )]);
    let config = Config::load_with(&Cli::default(), &env);
    let text = format!("{config:?}");
    assert!(!text.contains(canary), "{text}");
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

#[test]
fn the_kek_file_comes_from_the_environment() {
    let env = MapEnv::new(&[
        ("RESTICHUB_DATABASE_URL", "postgres://host/db"),
        ("RESTICHUB_KEK_FILE", "/etc/restichub/kek"),
    ]);
    let config = Config::load_with(&Cli::default(), &env);
    assert_eq!(
        config.ok().and_then(|config| config.secrets.kek_file),
        Some(PathBuf::from("/etc/restichub/kek"))
    );
}

#[test]
fn the_kek_file_comes_from_the_file() {
    let path = temp_path("secrets");
    std::fs::write(
        &path,
        "database_url = \"postgres://host/db\"\n[secrets]\nkek_file = \"/etc/restichub/kek\"\n",
    )
    .ok();
    let env = MapEnv::new(&[("RESTICHUB_CONFIG", path.to_str().unwrap_or_default())]);
    let config = Config::load_with(&Cli::default(), &env);
    std::fs::remove_file(&path).ok();
    assert_eq!(
        config.ok().and_then(|config| config.secrets.kek_file),
        Some(PathBuf::from("/etc/restichub/kek"))
    );
}

#[test]
fn the_tenancy_mode_defaults_to_single() {
    let env = MapEnv::new(&[("RESTICHUB_DATABASE_URL", "postgres://host/db")]);
    let config = Config::load_with(&Cli::default(), &env);
    let tenancy = config.ok().map(|config| config.tenancy);
    assert_eq!(
        tenancy.as_ref().map(|tenancy| tenancy.mode),
        Some(TenancyMode::Single)
    );
    assert_eq!(
        tenancy.map(|tenancy| tenancy.default_tenant),
        Some("default".to_owned())
    );
}

#[test]
fn a_bad_tenancy_mode_names_the_parameter() {
    let env = MapEnv::new(&[
        ("RESTICHUB_DATABASE_URL", "postgres://host/db"),
        ("RESTICHUB_TENANCY_MODE", "both"),
    ]);
    let error = Config::load_with(&Cli::default(), &env);
    assert!(matches!(
        error,
        Err(ConfigError::Invalid {
            name: "tenancy.mode",
            ..
        })
    ));
}

#[test]
fn the_tenancy_settings_come_from_the_file() {
    let path = temp_path("tenancy");
    std::fs::write(
        &path,
        "database_url = \"postgres://host/db\"\n[tenancy]\nmode = \"multi\"\ndefault_tenant = \"acme\"\n",
    )
    .ok();
    let env = MapEnv::new(&[("RESTICHUB_CONFIG", path.to_str().unwrap_or_default())]);
    let config = Config::load_with(&Cli::default(), &env);
    std::fs::remove_file(&path).ok();
    let tenancy = config.ok().map(|config| config.tenancy);
    assert_eq!(
        tenancy.as_ref().map(|tenancy| tenancy.mode),
        Some(TenancyMode::Multi)
    );
    assert_eq!(
        tenancy.map(|tenancy| tenancy.default_tenant),
        Some("acme".to_owned())
    );
}

#[test]
fn a_reserved_default_tenant_name_is_refused() {
    let env = MapEnv::new(&[
        ("RESTICHUB_DATABASE_URL", "postgres://host/db"),
        ("RESTICHUB_TENANCY_DEFAULT_TENANT", "platform"),
    ]);
    let error = Config::load_with(&Cli::default(), &env);
    assert!(matches!(
        error,
        Err(ConfigError::Invalid {
            name: "tenancy.default_tenant",
            ..
        })
    ));
}

/// the reported agent endpoint must use `https`, because the agent dials it (ADR-0033).
#[test]
fn a_plain_http_agent_endpoint_is_refused() {
    let env = MapEnv::new(&[
        ("RESTICHUB_DATABASE_URL", "postgres://host/db"),
        ("RESTICHUB_AGENT_ENDPOINT", "http://agents.example:8443"),
    ]);
    let error = Config::load_with(&Cli::default(), &env).expect_err("http must be refused");
    assert!(error.to_string().contains("https"), "{error}");
}

/// an `https` agent endpoint is accepted and reported as it is.
#[test]
fn an_https_agent_endpoint_is_accepted() {
    let env = MapEnv::new(&[
        ("RESTICHUB_DATABASE_URL", "postgres://host/db"),
        ("RESTICHUB_AGENT_ENDPOINT", "https://agents.example:8443"),
    ]);
    let config = Config::load_with(&Cli::default(), &env).expect("https is accepted");
    assert_eq!(
        config.agent_endpoint.as_deref(),
        Some("https://agents.example:8443")
    );
}

/// an endpoint with a userinfo is refused, because the agent refuses it too (ADR-0033).
#[test]
fn an_agent_endpoint_with_a_userinfo_is_refused() {
    let env = MapEnv::new(&[
        ("RESTICHUB_DATABASE_URL", "postgres://host/db"),
        (
            "RESTICHUB_AGENT_ENDPOINT",
            "https://user:pass@agents.example:8443",
        ),
    ]);
    let error = Config::load_with(&Cli::default(), &env).expect_err("a userinfo must be refused");
    assert!(error.to_string().contains("userinfo"), "{error}");
}

#[test]
fn ac_pr_04_a_lease_below_fifteen_seconds_is_refused() {
    let load = |lease: &str| {
        let env = MapEnv::new(&[
            ("RESTICHUB_DATABASE_URL", "postgres://host/db"),
            ("RESTICHUB_QUEUE_LEASE_SECS", lease),
        ]);
        Config::load_with(&Cli::default(), &env)
    };
    assert!(matches!(
        load("14"),
        Err(ConfigError::Invalid {
            name: "RESTICHUB_QUEUE_LEASE_SECS",
            ..
        })
    ));
    assert_eq!(
        load("15").ok().map(|config| config.queue.lease.as_secs()),
        Some(15)
    );
}

#[test]
fn fr_lg_02_the_log_retention_defaults_to_thirty_days_and_never_drops_below_one() {
    let load = |value: Option<&str>| {
        let mut pairs = vec![("RESTICHUB_DATABASE_URL", "postgres://host/db")];
        if let Some(value) = value {
            pairs.push(("RESTICHUB_LOG_RETENTION_DAYS", value));
        }
        Config::load_with(&Cli::default(), &MapEnv::new(&pairs))
            .map(|config| config.log_retention_days)
    };
    assert_eq!(load(None).ok(), Some(30));
    assert_eq!(load(Some("7")).ok(), Some(7));
    assert_eq!(load(Some("0")).ok(), Some(1));
    assert_eq!(load(Some("36500")).ok(), Some(36_500));
    assert!(matches!(
        load(Some("36501")),
        Err(ConfigError::Invalid {
            name: "RESTICHUB_LOG_RETENTION_DAYS",
            ..
        })
    ));
    assert!(matches!(
        load(Some("many")),
        Err(ConfigError::Invalid {
            name: "RESTICHUB_LOG_RETENTION_DAYS",
            ..
        })
    ));
}
