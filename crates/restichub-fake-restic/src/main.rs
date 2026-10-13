//! Recorded restic CLI emulator for the agent tests.
//!
//! The binary writes the restic JSON messages of the emulated commands and
//! exits with a selected code. The scenario file and the environment variables
//! are in the crate README.

mod cancel;
mod cli;
mod lock;
mod output;
mod password;
mod scenario;

use std::env;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cli::{Command, Invocation};
use output::{Outcome, ResticExit};
use scenario::{LockMode, Resolved, Scenario};

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
    let loaded = match load_scenario() {
        Ok(loaded) => loaded,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    let name = invocation.command.name();
    let index = match &loaded {
        Some((path, _)) => scenario::next_invocation(path, name),
        None => 0,
    };
    let base = loaded
        .as_ref()
        .map(|(_, scenario)| scenario.for_command(name, index))
        .unwrap_or_default();

    let env_exit = match exit_override() {
        Ok(value) => value,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let plan = base.resolve(env_exit);

    cancel::install(&plan.cancel);

    if invocation.command.needs_repository() && password::resolve(invocation).is_none() {
        eprintln!("Fatal: unable to read the repository password");
        return ExitCode::from(1);
    }

    for line in &plan.stderr_lines {
        eprintln!("{line}");
    }

    let lock_dir = lock::lock_dir(invocation.repository.as_deref());
    if let Some(dir) = &lock_dir
        && let Some(exit) = apply_lock(invocation, dir, plan.lock)
    {
        return exit;
    }
    if invocation.command == Command::Unlock
        && let Some(dir) = &lock_dir
    {
        lock::remove_all(dir);
    }

    let outcome = match output::emit(invocation, &plan) {
        Ok(outcome) => outcome,
        Err(err) => {
            eprintln!("restichub-fake-restic: {err}");
            return ExitCode::FAILURE;
        }
    };

    // A clean end releases the lock, as restic does in its deferred unlock.
    // A hard kill or a crash never reaches this line, so the lock survives.
    if plan.lock == LockMode::Manage
        && outcome == Outcome::Completed
        && let Some(dir) = &lock_dir
    {
        lock::remove_all(dir);
    }

    let exit = end_exit(&plan, outcome);
    if !exit.is_success() && invocation.global.json {
        eprintln!("{}", exit.error_message());
    }
    ExitCode::from(exit.code())
}

fn load_scenario() -> Result<Option<(PathBuf, Scenario)>, String> {
    match env::var_os("FAKE_RESTIC_SCENARIO") {
        Some(path) if !path.is_empty() => {
            let path = PathBuf::from(path);
            let scenario = Scenario::load(&path)
                .map_err(|err| format!("cannot read the scenario {}: {err}", path.display()))?;
            Ok(Some((path, scenario)))
        }
        _ => Ok(None),
    }
}

/// Applies the lock mode of the command. It returns the exit code of a locked
/// repository, or nothing when the command may run.
fn apply_lock(invocation: &Invocation, dir: &Path, mode: LockMode) -> Option<ExitCode> {
    match mode {
        LockMode::None => None,
        LockMode::Stale => {
            let _ = lock::create(dir);
            Some(locked_exit(invocation))
        }
        LockMode::Manage => {
            if lock::locked(dir) {
                return Some(locked_exit(invocation));
            }
            match lock::create(dir) {
                Ok(path) => {
                    cancel::remember_lock_file(path);
                    None
                }
                Err(err) => {
                    eprintln!("restichub-fake-restic: cannot take the lock: {err}");
                    Some(ExitCode::FAILURE)
                }
            }
        }
    }
}

fn locked_exit(invocation: &Invocation) -> ExitCode {
    let message = lock::conflict_message();
    if invocation.global.json {
        eprintln!("{}", ResticExit::Locked.error_message_with(message));
    } else {
        eprintln!("{message}");
    }
    ExitCode::from(11)
}

fn end_exit(plan: &Resolved, outcome: Outcome) -> ResticExit {
    let code = match outcome {
        Outcome::Completed => plan.exit_code,
        Outcome::Stopped => {
            if plan.exit_code == 0 {
                1
            } else {
                plan.exit_code
            }
        }
    };
    ResticExit::from_byte(code)
}

fn exit_override() -> Result<Option<u8>, String> {
    match env::var("FAKE_RESTIC_EXIT") {
        Ok(value) => value
            .parse::<u8>()
            .map(Some)
            .map_err(|_| format!("FAKE_RESTIC_EXIT is not a valid exit code: {value}")),
        Err(env::VarError::NotPresent) => Ok(None),
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
