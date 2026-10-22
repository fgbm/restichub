//! ResticHub agent.

use std::process::ExitCode;

use restichub_agent::cli::{self, Command};
#[cfg(windows)]
use restichub_agent::service;
use restichub_agent::{commands, logging, paths};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("restichub-agent: {error:#}");
            if commands::needs_administrator(&error) {
                return ExitCode::from(commands::EXIT_NO_IDENTITY);
            }
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
            logging::init_console();
            commands::register(&options, &paths::config_dir())
        }
        Command::Status => {
            logging::init_console();
            commands::status(&paths::config_dir())
        }
        Command::TrustServerScripts { enable, no_restart } => {
            commands::trust_server_scripts(enable, no_restart)
        }
        Command::Run => {
            // The service sets up its own log sinks; only a console run reaches the next line.
            #[cfg(windows)]
            if let Ok(()) = service::run_as_service() {
                return Ok(());
            }
            logging::init_console();
            commands::run(&paths::config_dir(), None)
        }
        Command::InstallService(options) => {
            logging::init_console();
            commands::install_service(&options)
        }
        Command::ResticUpdate(options) => {
            logging::init_console();
            commands::restic_update(&options)
        }
        Command::UninstallService => {
            logging::init_console();
            commands::uninstall_service()
        }
    }
}
