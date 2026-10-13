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
  register           enroll this host with a control plane
  install-service    write and enable the operating system service
  uninstall-service  stop and remove the operating system service
  run                run the agent (the protocol stream arrives with #50)
  status             print the stored identity facts
  version            print the agent version
  help               print this help

REGISTER OPTIONS:
  --url URL          control plane URL, for example https://cp.example
  --token TOKEN      one-time enrollment token (the documented default)
  --token-file PATH  read the token from a file
  --token-stdin      read the token from the standard input
  --ca-file PATH     CA certificate that verifies the control plane
  --force            replace an existing identity (re-register)
  --help             print this help";

/// Parsed subcommand with its options.
#[derive(Debug)]
pub enum Command {
    Register(RegisterOptions),
    InstallService,
    UninstallService,
    Run,
    Status,
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
        "install-service" => Ok(Command::InstallService),
        "uninstall-service" => Ok(Command::UninstallService),
        "run" => Ok(Command::Run),
        "status" => Ok(Command::Status),
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
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
        index += 1;
    }
    Ok(options)
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
    fn an_unknown_command_is_refused() {
        assert!(parse_args(&["nope"]).is_err());
    }

    #[test]
    fn an_unknown_option_is_refused() {
        assert!(parse_args(&["register", "--nope"]).is_err());
    }
}
