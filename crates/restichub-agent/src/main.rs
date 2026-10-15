//! ResticHub agent.

use std::process::ExitCode;

use restichub_agent::cli::{self, Command};
use restichub_agent::{commands, paths, service};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("restichub-agent: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    match cli::parse(std::env::args().skip(1).collect())? {
        Command::Help => {
            println!("{}", cli::USAGE);
            Ok(())
        }
        Command::Version => {
            println!(
                "restichub-agent {} (protocol {}.{})",
                restichub_common::BUILD_VERSION,
                restichub_proto::PROTOCOL_VERSION_MAJOR,
                restichub_proto::PROTOCOL_VERSION_MINOR
            );
            Ok(())
        }
        Command::Register(options) => {
            init_tracing();
            commands::register(&options, &paths::config_dir())
        }
        Command::Status => {
            init_tracing();
            commands::status(&paths::config_dir())
        }
        Command::Run => {
            init_tracing();
            #[cfg(windows)]
            if let Ok(()) = service::run_as_service() {
                return Ok(());
            }
            commands::run(&paths::config_dir(), None)
        }
        Command::InstallService(options) => {
            init_tracing();
            service::install(&options)
        }
        Command::ResticUpdate(options) => {
            init_tracing();
            commands::restic_update(&options)
        }
        Command::UninstallService => {
            init_tracing();
            service::uninstall()
        }
    }
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}
