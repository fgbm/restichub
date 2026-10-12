//! ResticHub control plane binary.

use anyhow::Context;
use restichub_cp::config::{Cli, Command, Config};
use restichub_cp::server::{self, AppState};
use restichub_cp::telemetry;

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
        Command::Run(cli) => run(*cli).await,
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let config = Config::load(&cli)?;
    telemetry::init(&config)?;
    tracing::info!(listen_addr = %config.listen_addr, "starting the control plane");

    let database = connect_database(&config).await?;
    database.migrate().await?;

    let app = server::router(AppState::new(database));
    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await
        .context("cannot bind the listen address")?;
    tracing::info!("the control plane is ready");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("the server stopped with an error")
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

/// Waits for SIGTERM or Ctrl+C.
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
