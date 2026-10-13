//! Recorded restic CLI emulator for the agent tests.
//!
//! The binary writes the restic JSON messages of the emulated commands and
//! exits with a selected code. The environment variables are in the crate
//! README. Issue #32 adds the scenario file on top of them.

mod cli;
mod output;
mod password;

use std::env;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::process::ExitCode;

use cli::Invocation;
use output::ResticExit;

fn main() -> ExitCode {
    let args: Vec<OsString> = env::args_os().skip(1).collect();

    // The argument vector is recorded before the command line is read, so a
    // test sees the arguments of a refused call.
    if let Some(path) = env::var_os("FAKE_RESTIC_ARGV_FILE")
        && let Err(err) = record_argv(&path, &args)
    {
        eprintln!("restichub-fake-restic: cannot write the argument vector: {err}");
        return ExitCode::FAILURE;
    }

    let invocation = match cli::parse(&args) {
        Ok(invocation) => invocation,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(1);
        }
    };

    run(&invocation)
}

fn run(invocation: &Invocation) -> ExitCode {
    let exit = match selected_exit() {
        Ok(exit) => exit,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    if invocation.command.needs_repository() && password::resolve(invocation).is_none() {
        eprintln!("Fatal: unable to read the repository password");
        return ExitCode::from(1);
    }

    if let Err(err) = output::emit(invocation) {
        eprintln!("restichub-fake-restic: {err}");
        return ExitCode::FAILURE;
    }

    if !exit.is_success() && invocation.global.json {
        eprintln!("{}", exit.error_message());
    }
    ExitCode::from(exit.code())
}

fn selected_exit() -> Result<ResticExit, String> {
    match env::var("FAKE_RESTIC_EXIT") {
        Ok(value) => value
            .parse::<u8>()
            .map(ResticExit::from_byte)
            .map_err(|_| format!("FAKE_RESTIC_EXIT is not a valid exit code: {value}")),
        Err(env::VarError::NotPresent) => Ok(ResticExit::Success),
        Err(err) => Err(format!("FAKE_RESTIC_EXIT is not valid: {err}")),
    }
}

fn record_argv(path: &OsStr, args: &[OsString]) -> std::io::Result<()> {
    let arguments: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let encoded = serde_json::to_string(&arguments).map_err(std::io::Error::other)?;
    let mut file = std::fs::File::create(path)?;
    writeln!(file, "{encoded}")
}
