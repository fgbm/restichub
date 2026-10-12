//! Spike P1: cancel `restic` from a Windows service that has no console.
//!
//! The binary has four subcommands:
//!
//! - `run` runs all mechanisms of one mode and writes a JSON report.
//! - `service-install` and `service-uninstall` manage the service.
//! - `break-helper` sends `CTRL_BREAK_EVENT` to the console of another process.
//!
//! The service entry point re-reads the command line, so the same image runs
//! the experiment as LocalSystem.

mod api;
mod mechanisms;
mod report;
mod service;

use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};

use mechanisms::{PreSpawn, Spawn, Spec, Stop};
use report::{MechanismResult, Report};

/// The mode of one experiment run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Console,
    Service,
}

impl Mode {
    fn parse(text: &str) -> Result<Mode, String> {
        match text {
            "console" => Ok(Mode::Console),
            "service" => Ok(Mode::Service),
            other => Err(format!("unknown mode: {other}")),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Console => "console",
            Mode::Service => "service",
        }
    }
}

/// The input of one experiment run.
#[derive(Clone, Debug)]
pub struct Config {
    pub mode: Mode,
    pub restic: PathBuf,
    pub repo: PathBuf,
    pub source: PathBuf,
    pub report: PathBuf,
}

impl Config {
    fn parse(args: &[String]) -> Result<Config, String> {
        let mut mode = Mode::Console;
        let mut restic = None;
        let mut repo = None;
        let mut source = None;
        let mut report = None;

        let mut index = 0;
        while index + 1 < args.len() {
            let value = args[index + 1].clone();
            match args[index].as_str() {
                "--mode" => mode = Mode::parse(&value)?,
                "--restic" => restic = Some(PathBuf::from(value)),
                "--repo" => repo = Some(PathBuf::from(value)),
                "--source" => source = Some(PathBuf::from(value)),
                "--report" => report = Some(PathBuf::from(value)),
                other => return Err(format!("unknown argument: {other}")),
            }
            index += 2;
        }

        if index < args.len() {
            return Err(format!("argument without a value: {}", args[index]));
        }

        Ok(Config {
            mode,
            restic: restic.ok_or("--restic is required")?,
            repo: repo.ok_or("--repo is required")?,
            source: source.ok_or("--source is required")?,
            report: report.ok_or("--report is required")?,
        })
    }

    /// Read the configuration from the command line of the service image.
    pub fn from_env_run_args() -> Result<Config, String> {
        let args: Vec<String> = env::args().skip(1).collect();
        let rest = match args.first().map(String::as_str) {
            Some("run") => &args[1..],
            _ => &args[..],
        };
        Config::parse(rest)
    }

    /// The arguments that the service stores in the registry.
    fn service_launch_arguments(&self) -> Vec<OsString> {
        vec![
            OsString::from("run"),
            OsString::from("--mode"),
            OsString::from("service"),
            OsString::from("--restic"),
            self.restic.clone().into_os_string(),
            OsString::from("--repo"),
            self.repo.clone().into_os_string(),
            OsString::from("--source"),
            self.source.clone().into_os_string(),
            OsString::from("--report"),
            self.report.clone().into_os_string(),
        ]
    }
}

/// The entry point of the Windows build.
pub fn main_entry() {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Err(error) = dispatch(&args) {
        eprintln!("p1-cancel: {error}");
        std::process::exit(1);
    }
}

fn dispatch(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("run") => {
            let config = Config::parse(&args[1..])?;
            if config.mode == Mode::Service {
                service::run_as_service()
            } else {
                run_experiment_and_write(&config)
            }
        }
        Some("service-install") => {
            let mut config = Config::parse(&args[1..])?;
            config.mode = Mode::Service;
            service::install(&config)
        }
        Some("service-uninstall") => service::uninstall(),
        Some("break-helper") => {
            let pid = parse_pid(&args[1..])?;
            break_helper(pid)
        }
        _ => Err(usage()),
    }
}

fn parse_pid(args: &[String]) -> Result<u32, String> {
    let mut index = 0;
    while index + 1 < args.len() {
        if args[index] == "--pid" {
            return args[index + 1]
                .parse()
                .map_err(|error| format!("invalid pid: {error}"));
        }
        index += 2;
    }
    Err("--pid is required".to_string())
}

/// Send `CTRL_BREAK_EVENT` to the console of the process with this pid.
///
/// This is mechanism C. The helper leaves its own console, attaches to the
/// console of the child, and sends the event to all attached processes.
fn break_helper(pid: u32) -> Result<(), String> {
    api::free_console();
    api::attach_console(pid).map_err(|error| format!("AttachConsole({pid}): {error}"))?;
    api::ignore_ctrl_c();
    api::generate_break(0)
        .map_err(|error| format!("GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0): {error}"))?;
    std::thread::sleep(Duration::from_millis(300));
    Ok(())
}

/// Run every mechanism and write the JSON report.
pub fn run_experiment_and_write(config: &Config) -> Result<(), String> {
    let mut results = Vec::new();
    for spec in specs() {
        match mechanisms::run(&spec, config) {
            Ok(result) => results.push(result),
            Err(error) => results.push(MechanismResult::failed(
                spec.id,
                config.mode.as_str(),
                spec.description,
                error,
            )),
        }
    }

    let report = Report {
        mode: config.mode.as_str().to_string(),
        restic_path: config.restic.to_string_lossy().into_owned(),
        restic_version: report::restic_version(&config.restic),
        repository: config.repo.to_string_lossy().into_owned(),
        source: config.source.to_string_lossy().into_owned(),
        mechanisms: results,
    };

    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(parent) = config.report.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    std::fs::write(&config.report, json)
        .map_err(|error| format!("write {}: {error}", config.report.display()))
}

/// The mechanisms under test, in run order.
///
/// Mechanism B allocates a console. It runs after the mechanisms that need no
/// console of the parent.
fn specs() -> Vec<Spec> {
    vec![
        Spec {
            id: "A",
            description: "child with CREATE_NEW_PROCESS_GROUP; the spike sends CTRL_BREAK to the group",
            spawn: Spawn::Std(CREATE_NEW_PROCESS_GROUP),
            pre_spawn: None,
            stop: Stop::CtrlBreak,
        },
        Spec {
            id: "C1",
            description: "child with CREATE_NO_WINDOW; break-helper sends CTRL_BREAK to the console",
            spawn: Spawn::Std(CREATE_NO_WINDOW),
            pre_spawn: None,
            stop: Stop::BreakHelper,
        },
        Spec {
            id: "C2",
            description: "child with CREATE_NEW_CONSOLE and a hidden window; break-helper sends CTRL_BREAK",
            spawn: Spawn::RawHiddenConsole,
            pre_spawn: None,
            stop: Stop::BreakHelper,
        },
        Spec {
            id: "B",
            description: "AllocConsole, then CREATE_NEW_PROCESS_GROUP; the spike sends CTRL_BREAK to the group",
            spawn: Spawn::Std(CREATE_NEW_PROCESS_GROUP),
            pre_spawn: Some(PreSpawn::AllocConsole),
            stop: Stop::CtrlBreak,
        },
        Spec {
            id: "D",
            description: "hard kill baseline: TerminateJobObject on the child job",
            spawn: Spawn::Std(CREATE_NEW_PROCESS_GROUP),
            pre_spawn: None,
            stop: Stop::TerminateJob,
        },
    ]
}

fn usage() -> String {
    "usage:\n  \
     p1-cancel run --mode console|service --restic <path> --repo <path> --source <dir> --report <file>\n  \
     p1-cancel service-install --restic <path> --repo <path> --source <dir> --report <file>\n  \
     p1-cancel service-uninstall\n  \
     p1-cancel break-helper --pid <pid>"
        .to_string()
}
