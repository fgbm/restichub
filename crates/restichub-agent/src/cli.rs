//! Command line of the agent.
//!
//! The workspace has no argument parser, so this module reads the arguments itself and prints the
//! usage on `--help`.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};

pub const USAGE: &str = "\
restichub-agent - the ResticHub backup agent

USAGE:
  restichub-agent <command> [options]

COMMANDS:
  register           enroll this host, then install and start the service and wait for the connection
  install-service    download restic if absent, then write, enable and start the operating system service
  restic update      download the latest restic and replace the installed one
  uninstall-service  stop and remove the operating system service
  run                run the agent (the protocol stream)
  trust-server-scripts  turn the local trust mode for server scripts on or off
  status             print the identity, restic, the service and the connection; exit code 1 when not connected
  version            print the agent version
  help               print this help

TRUST-SERVER-SCRIPTS OPTIONS (run as root or Administrator):
  --enable           run scripts that the control plane sends (default: off)
  --disable          refuse scripts that the control plane sends
  --no-restart       do not restart the installed service; the control plane sees the new state after the next start

REGISTER OPTIONS:
  --url URL          control plane URL, for example https://cp.example
  --token TOKEN      one-time enrollment token (the documented default)
  --token-file PATH  read the token from a file
  --token-stdin      read the token from the standard input
  --ca-file PATH     CA certificate that verifies the control plane
  --force            replace an existing identity (re-register)
  --no-service       only enroll; do not install or start the service
  --trust-server-scripts  turn the trust mode on before the service starts: the control plane can then run any command on this host (default: off)
  --verbose          print each entry whose access control the repair changes (also install-service)
  --help             print this help

RESTIC OPTIONS (register, install-service, restic update):
  --restic-archive PATH    restic release archive for this host, instead of the download
  --restic-sums PATH       the SHA256SUMS file of that release
  --restic-sums-sig PATH   the SHA256SUMS.asc signature of that file
  HTTPS_PROXY, NO_PROXY    the proxy of the download (an http:// proxy URL)

RESTIC UPDATE OPTIONS:
  --allow-downgrade        install a release that is older than the installed one
  --no-restart             do not restart the installed service after the update";

/// Parsed subcommand with its options.
#[derive(Debug)]
pub enum Command {
    Register(RegisterOptions),
    InstallService(InstallOptions),
    ResticUpdate(UpdateOptions),
    UninstallService,
    Run,
    Status,
    /// Turns the local trust mode for server scripts on or off (FR-HK-08, ADR-0037).
    TrustServerScripts {
        enable: bool,
        no_restart: bool,
    },
    Version,
    Help,
}

/// The options of `register` (ADR-0032, FR-IN-07).
#[derive(Debug, Default, Clone)]
pub struct RegisterOptions {
    pub url: Option<String>,
    pub token: Option<String>,
    pub token_file: Option<PathBuf>,
    pub token_stdin: bool,
    pub ca_file: Option<PathBuf>,
    pub force: bool,
    pub no_service: bool,
    pub trust_server_scripts: bool,
    pub verbose: bool,
    pub restic: ResticFileOptions,
}

#[derive(Debug, Default, Clone)]
pub struct InstallOptions {
    pub verbose: bool,
    pub restic: ResticFileOptions,
}

/// The offline release files of restic (ADR-0038). All three or none.
#[derive(Debug, Default, Clone)]
pub struct ResticFileOptions {
    pub archive: Option<PathBuf>,
    pub sums: Option<PathBuf>,
    pub sums_signature: Option<PathBuf>,
}

#[derive(Debug, Default, Clone)]
pub struct UpdateOptions {
    pub restic: ResticFileOptions,
    pub allow_downgrade: bool,
    pub no_restart: bool,
}

/// Reads the command from the arguments (FR-IN-07, FR-IN-08).
///
/// # Errors
///
/// Returns an error on an unknown command or a bad option.
pub fn parse(args: Vec<String>) -> Result<Command> {
    let Some((first, rest)) = args.split_first() else {
        return Ok(Command::Help);
    };
    match first.as_str() {
        "register" => parse_register(rest).map(Command::Register),
        "install-service" => parse_install(rest).map(Command::InstallService),
        "restic" => match rest.split_first() {
            Some((sub, options)) if sub == "update" => {
                parse_update(options).map(Command::ResticUpdate)
            }
            _ => bail!("use `restic update`"),
        },
        "uninstall-service" => Ok(Command::UninstallService),
        "run" => Ok(Command::Run),
        "status" => Ok(Command::Status),
        "trust-server-scripts" => parse_trust(rest),
        "version" | "--version" | "-V" => Ok(Command::Version),
        "help" | "--help" | "-h" => Ok(Command::Help),
        other => bail!("unknown command: {other}"),
    }
}

fn parse_register(args: &[String]) -> Result<RegisterOptions> {
    let mut options = RegisterOptions::default();
    let mut index = 0;
    while index < args.len() {
        let name = args[index].split('=').next().unwrap_or_default().to_owned();
        match name.as_str() {
            "--url" => options.url = Some(take_value(args, &mut index, &name)?),
            "--token" => options.token = Some(take_value(args, &mut index, &name)?),
            "--token-file" => {
                options.token_file = Some(PathBuf::from(take_value(args, &mut index, &name)?));
            }
            "--token-stdin" => options.token_stdin = true,
            "--ca-file" => {
                options.ca_file = Some(PathBuf::from(take_value(args, &mut index, &name)?));
            }
            "--force" => options.force = true,
            "--no-service" => options.no_service = true,
            "--trust-server-scripts" => options.trust_server_scripts = true,
            "--verbose" => options.verbose = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => {
                if !take_restic_option(other, args, &mut index, &mut options.restic)? {
                    bail!("unknown argument: {other}");
                }
            }
        }
        index += 1;
    }
    Ok(options)
}

fn parse_trust(args: &[String]) -> Result<Command> {
    let mut choice = None;
    let mut no_restart = false;
    for arg in args {
        match arg.as_str() {
            "--enable" | "--disable" if choice.is_some() => {
                bail!("trust-server-scripts needs exactly one of --enable and --disable");
            }
            "--enable" => choice = Some(true),
            "--disable" => choice = Some(false),
            "--no-restart" => no_restart = true,
            "--help" | "-h" => return Ok(Command::Help),
            other => bail!("unknown argument: {other}"),
        }
    }
    let Some(enable) = choice else {
        bail!("trust-server-scripts needs exactly one of --enable and --disable");
    };
    Ok(Command::TrustServerScripts { enable, no_restart })
}

fn parse_install(args: &[String]) -> Result<InstallOptions> {
    let mut options = InstallOptions::default();
    let mut index = 0;
    while index < args.len() {
        let name = args[index].split('=').next().unwrap_or_default().to_owned();
        match name.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--verbose" => options.verbose = true,
            other => {
                if !take_restic_option(other, args, &mut index, &mut options.restic)? {
                    bail!("unknown argument: {other}");
                }
            }
        }
        index += 1;
    }
    Ok(options)
}

fn parse_update(args: &[String]) -> Result<UpdateOptions> {
    let mut options = UpdateOptions::default();
    let mut index = 0;
    while index < args.len() {
        let name = args[index].split('=').next().unwrap_or_default().to_owned();
        match name.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--allow-downgrade" => options.allow_downgrade = true,
            "--no-restart" => options.no_restart = true,
            other => {
                if !take_restic_option(other, args, &mut index, &mut options.restic)? {
                    bail!("unknown argument: {other}");
                }
            }
        }
        index += 1;
    }
    Ok(options)
}

fn take_restic_option(
    name: &str,
    args: &[String],
    index: &mut usize,
    options: &mut ResticFileOptions,
) -> Result<bool> {
    let slot = match name {
        "--restic-archive" => &mut options.archive,
        "--restic-sums" => &mut options.sums,
        "--restic-sums-sig" => &mut options.sums_signature,
        _ => return Ok(false),
    };
    *slot = Some(PathBuf::from(take_value(args, index, name)?));
    Ok(true)
}

fn take_value(args: &[String], index: &mut usize, name: &str) -> Result<String> {
    if let Some((_, value)) = args[*index].split_once('=') {
        return Ok(value.to_owned());
    }
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| anyhow!("missing value for {name}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn parse_args(args: &[&str]) -> Result<Command> {
        parse(args.iter().map(|value| (*value).to_owned()).collect())
    }

    #[test]
    fn no_argument_prints_the_usage() {
        assert!(matches!(parse_args(&[]).expect("command"), Command::Help));
    }

    #[test]
    fn register_reads_the_url_and_the_token() {
        let command = parse_args(&["register", "--url", "https://cp.example", "--token", "re_x"])
            .expect("command");
        let Command::Register(options) = command else {
            panic!("expected register");
        };
        assert_eq!(options.url.as_deref(), Some("https://cp.example"));
        assert_eq!(options.token.as_deref(), Some("re_x"));
    }

    #[test]
    fn register_reads_an_inline_value() {
        let command =
            parse_args(&["register", "--url=https://cp.example", "--force"]).expect("command");
        let Command::Register(options) = command else {
            panic!("expected register");
        };
        assert_eq!(options.url.as_deref(), Some("https://cp.example"));
        assert!(options.force);
    }

    #[test]
    fn fr_hk_08_trust_server_scripts_needs_one_explicit_choice() {
        assert!(matches!(
            parse_args(&["trust-server-scripts", "--enable"]).expect("command"),
            Command::TrustServerScripts {
                enable: true,
                no_restart: false
            }
        ));
        assert!(matches!(
            parse_args(&["trust-server-scripts", "--disable"]).expect("command"),
            Command::TrustServerScripts {
                enable: false,
                no_restart: false
            }
        ));
        assert!(parse_args(&["trust-server-scripts"]).is_err());
        assert!(parse_args(&["trust-server-scripts", "--enable", "--disable"]).is_err());
        assert!(parse_args(&["trust-server-scripts", "--no-restart"]).is_err());
        assert!(parse_args(&["trust-server-scripts", "--enable", "--nope"]).is_err());
    }

    #[test]
    fn fr_hk_08_trust_server_scripts_reads_the_no_restart_flag() {
        assert!(matches!(
            parse_args(&["trust-server-scripts", "--no-restart", "--enable"]).expect("command"),
            Command::TrustServerScripts {
                enable: true,
                no_restart: true
            }
        ));
    }

    #[test]
    fn fr_hk_08_register_turns_the_trust_mode_on_only_when_told() {
        let plain = parse_args(&["register", "--url", "https://cp.example", "--token", "t"])
            .expect("command");
        assert!(matches!(plain, Command::Register(options) if !options.trust_server_scripts));
        let trusted = parse_args(&[
            "register",
            "--url",
            "https://cp.example",
            "--trust-server-scripts",
        ])
        .expect("command");
        assert!(matches!(trusted, Command::Register(options) if options.trust_server_scripts));
        assert!(parse_args(&["install-service", "--trust-server-scripts"]).is_err());
    }

    #[test]
    fn fr_ag_11_the_offline_restic_files_are_read() {
        let command = parse_args(&[
            "register",
            "--restic-archive",
            "/tmp/a.bz2",
            "--restic-sums=/tmp/sums",
            "--restic-sums-sig",
            "/tmp/sums.asc",
        ])
        .expect("command");
        let Command::Register(options) = command else {
            panic!("expected register");
        };
        assert_eq!(options.restic.archive, Some(PathBuf::from("/tmp/a.bz2")));
        assert_eq!(options.restic.sums, Some(PathBuf::from("/tmp/sums")));
        assert_eq!(
            options.restic.sums_signature,
            Some(PathBuf::from("/tmp/sums.asc"))
        );
    }

    #[test]
    fn fr_ag_11_restic_update_and_install_service_take_the_offline_files() {
        let update = parse_args(&["restic", "update", "--restic-sums", "s"]).expect("command");
        assert!(matches!(update, Command::ResticUpdate(options) if options.restic.sums.is_some()));
        let install = parse_args(&["install-service", "--restic-archive", "a"]).expect("command");
        assert!(
            matches!(install, Command::InstallService(options) if options.restic.archive.is_some())
        );
        assert!(parse_args(&["restic"]).is_err());
        assert!(parse_args(&["restic", "downgrade"]).is_err());
        assert!(parse_args(&["install-service", "--nope"]).is_err());
    }

    #[test]
    fn fr_ag_11_restic_update_reads_the_downgrade_and_restart_flags() {
        let plain = parse_args(&["restic", "update"]).expect("command");
        assert!(matches!(
            plain,
            Command::ResticUpdate(options) if !options.allow_downgrade && !options.no_restart
        ));
        let flags = parse_args(&["restic", "update", "--allow-downgrade", "--no-restart"])
            .expect("command");
        assert!(matches!(
            flags,
            Command::ResticUpdate(options) if options.allow_downgrade && options.no_restart
        ));
        assert!(parse_args(&["install-service", "--no-restart"]).is_err());
    }

    #[test]
    fn fr_in_07_register_installs_the_service_unless_told_otherwise() {
        let plain = parse_args(&["register", "--url", "https://cp.example", "--token", "t"])
            .expect("command");
        assert!(matches!(plain, Command::Register(options) if !options.no_service));
        let only = parse_args(&["register", "--url", "https://cp.example", "--no-service"])
            .expect("command");
        assert!(matches!(only, Command::Register(options) if options.no_service));
        assert!(parse_args(&["install-service", "--no-service"]).is_err());
    }

    #[test]
    fn fr_ag_04_register_and_install_service_print_each_repaired_entry_only_when_told() {
        let plain = parse_args(&["install-service"]).expect("command");
        assert!(matches!(plain, Command::InstallService(options) if !options.verbose));
        let verbose = parse_args(&["install-service", "--verbose"]).expect("command");
        assert!(matches!(verbose, Command::InstallService(options) if options.verbose));
        let register =
            parse_args(&["register", "--url", "https://cp.example", "--verbose"]).expect("command");
        assert!(matches!(register, Command::Register(options) if options.verbose));
        assert!(parse_args(&["restic", "update", "--verbose"]).is_err());
    }

    #[test]
    fn an_unknown_command_is_refused() {
        assert!(parse_args(&["nope"]).is_err());
    }

    #[test]
    fn an_unknown_option_is_refused() {
        assert!(parse_args(&["register", "--nope"]).is_err());
    }
}
