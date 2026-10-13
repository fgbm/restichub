use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use restichub_cp::config::{Cli, Command, Config, MigrationConfig};
use restichub_cp::server::{self, AppState};
use restichub_cp::telemetry;
use restichub_secrets::{KekConfig, KekProvider, SystemEnv, load_kek_provider};

const MIGRATION_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match restichub_cp::config::parse_args(std::env::args().skip(1))? {
        Command::Version => {
            println!(
                "restichub-cp {} (protocol {}.{})",
                env!("CARGO_PKG_VERSION"),
                restichub_proto::PROTOCOL_VERSION_MAJOR,
                restichub_proto::PROTOCOL_VERSION_MINOR
            );
            Ok(())
        }
        Command::Serve(cli) => serve(*cli).await,
        Command::Migrate(cli) => migrate(*cli).await,
    }
}

async fn serve(cli: Cli) -> anyhow::Result<()> {
    let config = Config::load(&cli)?;
    telemetry::init(&config)?;
    // Refuse a core dump before the key encryption key is in memory (T18).
    restichub_cp::harden::disable_core_dumps();
    tracing::info!(listen_addr = %config.listen_addr, "starting the control plane");
    if config.auth.insecure_cookies {
        tracing::warn!(
            "server.insecure_cookies is on: the session cookie drops the Secure attribute and the __Host- prefix; a plain HTTP network can then read the session and another host on the same parent domain can set a cookie with the same name (cookie tossing); use this setting only for local development over plain HTTP"
        );
    }
    // Build the dummy password hash before the first sign-in, so no request pays for it.
    restichub_cp::password::warm_dummy();

    let database = connect_database(&config).await?;
    database.verify_runtime_role().await?;
    database.verify_migrations().await?;

    // Load the key encryption key and verify the `kek_check` row (AC-RP-02).
    let kek: Arc<dyn KekProvider> = load_kek_provider(
        &KekConfig {
            file_path: config.secrets.kek_file.clone(),
        },
        &SystemEnv,
    )
    .context("cannot load the key encryption key")?
    .into();
    restichub_cp::startup::ensure_kek_check(&database, kek.as_ref())
        .await
        .context("cannot verify the key encryption key")?;

    // Create the installation CA on the first start, or load it and prove that the KEK opens the
    // stored keys. The server refuses to start without a CA (ADR-0026).
    let ca = restichub_cp::pki::CaService::new(kek.clone());
    ca.ensure_initialized(&database)
        .await
        .context("cannot prepare the certificate authority")?;

    // Create the job signing key set on the first start, or load it and prove that the KEK opens
    // the stored keys. The key identifiers are public; a key never reaches the log (AC-PR-02).
    let job_signing = restichub_cp::job_signing::JobSigningService::new(kek.clone());
    let key_set = job_signing
        .ensure_initialized(&database)
        .await
        .context("cannot prepare the job signing keys")?;
    tracing::info!(
        current_key_id = %key_set.current_key_id(),
        next_key_id = %key_set.next_key_id(),
        "the job signing key set is ready"
    );

    // Single mode refuses the start unless the database matches the mode (ADR-0024).
    restichub_cp::startup::ensure_tenancy_mode(&database, &config.tenancy)
        .await
        .context("cannot check the tenancy mode")?;

    // The setup token protects the first administrator of both modes (ADR-0024).
    restichub_cp::setup::ensure_setup_token(
        &database,
        &restichub_cp::clock::SystemClock,
        &config.tenancy,
    )
    .await
    .context("cannot prepare the initial setup")?;

    // The periodic maintenance removes the expired sessions and setup tokens.
    restichub_cp::maintenance::spawn(
        database.clone(),
        restichub_cp::maintenance::CLEANUP_INTERVAL,
    );

    // The periodic retirement bounds the job signing key rotation overlap without a manual call
    // (ADR-0026). It is a control plane task, not job scheduling (ADR-0002).
    restichub_cp::maintenance::spawn_signing_key_retirement(
        database.clone(),
        job_signing.clone(),
        Arc::new(restichub_cp::clock::SystemClock),
        restichub_cp::maintenance::SIGNING_KEY_RETIRE_INTERVAL,
    );

    let auth = restichub_cp::server::AuthState::new(config.auth.clone());
    restichub_cp::maintenance::spawn_limiter_sweep(
        auth.user_limiter.clone(),
        auth.ip_limiter.clone(),
        auth.create_user_limiter.clone(),
        auth.create_user_tenant_limiter.clone(),
        restichub_cp::maintenance::LIMITER_SWEEP_INTERVAL,
    );
    let app = server::router(
        AppState::with_auth(database, auth)
            .with_trusted_proxies(config.trusted_proxies.clone())
            .with_kek(kek)
            .with_tenancy(config.tenancy.clone()),
    );
    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await
        .context("cannot bind the listen address")?;
    tracing::info!("the control plane is ready");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("the server stopped with an error")
}

/// Applies the migrations with the owner login and stops.
async fn migrate(cli: Cli) -> anyhow::Result<()> {
    telemetry::init_command()?;
    let migration = MigrationConfig::load(&cli)?;
    let database_config = restichub_db::DatabaseConfig {
        url: migration.url.clone(),
        max_connections: 1,
        acquire_timeout: MIGRATION_ACQUIRE_TIMEOUT,
    };
    let password = migration
        .password
        .as_ref()
        .map(|password| password.expose_secret());
    let database = restichub_db::Database::connect(&database_config, password).await?;
    database.migrate().await?;
    database.close().await;
    Ok(())
}

async fn connect_database(config: &Config) -> anyhow::Result<restichub_db::Database> {
    let database_config = restichub_db::DatabaseConfig {
        url: config.database_url.clone(),
        max_connections: config.database_max_connections,
        acquire_timeout: config.database_acquire_timeout,
    };
    let password = config
        .database_password
        .as_ref()
        .map(|password| password.expose_secret());
    let database = restichub_db::Database::connect(&database_config, password).await?;
    Ok(database)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {}
            Err(error) => {
                tracing::error!(%error, "cannot listen for Ctrl+C");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(%error, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("shutdown signal received");
}
