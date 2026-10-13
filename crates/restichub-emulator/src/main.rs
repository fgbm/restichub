//! The emulator binary. The options are in the crate README.

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match restichub_emulator::cli::run_from_env().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("restichub-emulator: {error:#}");
            ExitCode::FAILURE
        }
    }
}
