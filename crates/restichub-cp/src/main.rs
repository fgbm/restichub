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
        Command::CompromiseSigningKey(cli) => compromise_signing_key(*cli).await,
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
    // stored keys. The server refuses to start without a CA (ADR-0026). The loaded store serves the
    // enrollment endpoint, so an issue reuses the in-memory issuing key.
    let ca = restichub_cp::pki::CaService::new(kek.clone());
    let ca_store = Arc::new(
        ca.ensure_initialized(&database)
            .await
            .context("cannot prepare the certificate authority")?,
    );

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

    // The periodic sweep moves an overdue job to `expired`, so a poll sees a terminal status
    // It is a maintenance task of the control plane, not job scheduling (ADR-0002).
    restichub_cp::maintenance::spawn_job_expiry(
        database.clone(),
        Arc::new(restichub_cp::clock::SystemClock),
        restichub_cp::maintenance::JOB_EXPIRY_INTERVAL,
    );

    // The scheduler turns a due plan into one job for each fire time (FR-SC-01, FR-SC-02, ADR-0010).
    // It runs in the control plane; the agent holds no schedule (ADR-0002).
    restichub_cp::scheduler::spawn(
        database.clone(),
        Arc::new(restichub_cp::clock::SystemClock),
        restichub_cp::scheduler::SCHEDULER_INTERVAL,
    );

    // The retention sweep deletes the old log text and keeps the job rows (FR-LG-02, AC-LG-01).
    restichub_cp::log_store::spawn_retention(
        database.clone(),
        Arc::new(restichub_cp::clock::SystemClock),
        config.log_retention_days,
        restichub_cp::log_store::RETENTION_INTERVAL,
    );

    let auth = restichub_cp::server::AuthState::new(config.auth.clone());
    restichub_cp::maintenance::spawn_limiter_sweep(
        auth.user_limiter.clone(),
        auth.ip_limiter.clone(),
        auth.create_user_limiter.clone(),
        auth.create_user_tenant_limiter.clone(),
        restichub_cp::maintenance::LIMITER_SWEEP_INTERVAL,
    );
    let alert_sender = restichub_cp::alerts::sender::Sender::new(config.alerts.clone())
        .context("cannot prepare the alert sender")?;
    let state = AppState::with_auth(database, auth)
        .with_trusted_proxies(config.trusted_proxies.clone())
        .with_kek(kek.clone())
        .with_sender(alert_sender.clone())
        .with_tenancy(config.tenancy.clone())
        .with_queue(config.queue.clone())
        .with_log_retention_days(config.log_retention_days)
        .with_agent_cert_lifetime(config.agent_cert_lifetime)
        .with_pki(
            ca_store.clone(),
            job_signing.clone(),
            config.agent_endpoint.clone(),
        );
    // The dispatcher sends the queued jobs to the online agents (FR-SC-03, ADR-0010). It runs in the
    // control plane; the agent holds no schedule (ADR-0002).
    restichub_cp::queue::spawn(state.clone(), state.auth().clock.clone());
    // The alert loop evaluates the rules and sends the messages (FR-MN-03, FR-MN-04, ADR-0036). It
    // is a control plane task, not job scheduling (ADR-0002).
    restichub_cp::alerts::spawn(
        state.db().clone(),
        alert_sender,
        kek,
        state.auth().clock.clone(),
        restichub_cp::alerts::INTERVAL,
    );
    let app = server::router(state.clone());
    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await
        .context("cannot bind the listen address")?;

    // The batch flush writes the `last_seen_at` buffer of the agent streams.
    restichub_cp::agent_stream::spawn_last_seen_flush(
        state.db().clone(),
        state.agent_registry_shared(),
        state.auth().clock.clone(),
        restichub_cp::agent_stream::LAST_SEEN_INTERVAL,
    );

    // The sweep closes the streams of this instance that an Admin revoked on another instance
    // (FR-ID-04).
    restichub_cp::agent_stream::spawn_revocation_sweep(
        state.clone(),
        restichub_cp::agent_stream::REVOCATION_SWEEP_INTERVAL,
    );

    // One shutdown signal stops the web server and the agent listener.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = shutdown_tx.send(());
    });

    // The agent listener serves the gRPC stream and the enrollment endpoint over TLS (ADR-0026).
    if let Some(addr) = config.agent_listen_addr {
        let root = ca_store.root().cert_der.clone();
        let tls = match restichub_cp::agent_listener::configured_tls_config(&config, &root)? {
            Some(tls) => tls,
            None => {
                let sans = restichub_cp::agent_listener::server_cert_sans(
                    config.agent_endpoint.as_deref(),
                    addr,
                );
                let issued = ca
                    .issue_server_certificate(&ca_store, &sans)
                    .context("cannot issue the agent listener certificate")?;
                restichub_cp::agent_listener::tls_config_from_issued(&issued, &root)?
            }
        };
        let limits = restichub_cp::agent_listener::ListenerLimits {
            max_connections: config.agent_max_connections,
            ..Default::default()
        };
        let agent_state = state.clone();
        let mut agent_shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            let stop = async move {
                let _ = agent_shutdown.changed().await;
            };
            if let Err(error) =
                restichub_cp::agent_listener::serve(agent_state, addr, tls, limits, stop).await
            {
                tracing::error!(%error, "the agent listener stopped");
            }
        });
        tracing::info!(%addr, "the agent listener is ready");
    } else {
        tracing::warn!(
            "RESTICHUB_AGENT_LISTEN is not set: the agent listener and the enrollment endpoint are off"
        );
    }

    tracing::info!("the control plane is ready");

    let mut web_shutdown = shutdown_rx;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = web_shutdown.changed().await;
    })
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

/// Rotates the job signing key set and retires the current key for a compromise (ADR-0030). It
/// works in the single and the multi tenant mode; the running instances deliver the signed update
/// to their agents at their next sweep.
async fn compromise_signing_key(cli: Cli) -> anyhow::Result<()> {
    let config = Config::load(&cli)?;
    telemetry::init_command()?;
    let database = connect_database(&config).await?;
    database.verify_runtime_role().await?;
    let kek: Arc<dyn KekProvider> = load_kek_provider(
        &KekConfig {
            file_path: config.secrets.kek_file.clone(),
        },
        &SystemEnv,
    )
    .context("cannot load the key encryption key")?
    .into();
    let signing = restichub_cp::job_signing::JobSigningService::new(kek);
    let event = restichub_db::audit::AuditEvent::new(
        restichub_db::audit::AuditAction::SigningKeyCompromise,
        "signing_key",
    );
    let current = signing
        .current_key_id(&database)
        .await
        .ok_or_else(|| anyhow::anyhow!("the installation has no current job signing key"))?;
    let retired = signing
        .mark_compromised(&database, time::OffsetDateTime::now_utc(), &current, &event)
        .await
        .context("cannot rotate the job signing key set")?;
    for id in retired {
        println!("retired for a compromise: {id}");
    }
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
