//! The restic command line that the fake binary accepts (FR-PR-10).

use std::ffi::OsString;

/// One restic subcommand that the agent starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Backup,
    Forget,
    Prune,
    Check,
    Snapshots,
    Unlock,
    Restore,
    Init,
    Version,
    CatConfig,
}

impl Command {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "backup" => Self::Backup,
            "forget" => Self::Forget,
            "prune" => Self::Prune,
            "check" => Self::Check,
            "snapshots" => Self::Snapshots,
            "unlock" => Self::Unlock,
            "restore" => Self::Restore,
            "init" => Self::Init,
            "version" => Self::Version,
            _ => return None,
        })
    }

    /// True when the command opens a repository and therefore needs the password.
    pub fn needs_repository(self) -> bool {
        !matches!(self, Self::Version)
    }

    /// Key of this command in the scenario file.
    pub fn name(self) -> &'static str {
        match self {
            Self::Backup => "backup",
            Self::Forget => "forget",
            Self::Prune => "prune",
            Self::Check => "check",
            Self::Snapshots => "snapshots",
            Self::Unlock => "unlock",
            Self::Restore => "restore",
            Self::Init => "init",
            Self::Version => "version",
            Self::CatConfig => "cat_config",
        }
    }
}

/// Options that change the output.
#[derive(Debug, Clone, Default)]
pub struct GlobalFlags {
    pub json: bool,
}

/// Parsed arguments of one restic call.
#[derive(Debug, Clone)]
pub struct Invocation {
    pub command: Command,
    pub global: GlobalFlags,
    pub repository: Option<String>,
    pub password_file: Option<String>,
    /// Positional arguments after the command: the paths and the snapshot IDs.
    pub arguments: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlagKind {
    Bool,
    Value,
}

fn long_flag(name: &str) -> Option<FlagKind> {
    use FlagKind::{Bool, Value};
    Some(match name {
        // Global options.
        "repo"
        | "repository-file"
        | "password-file"
        | "key-hint"
        | "password-command"
        | "retry-lock"
        | "cache-dir"
        | "cacert"
        | "tls-client-cert"
        | "compression"
        | "pack-size"
        | "limit-upload"
        | "limit-download"
        | "option"
        | "http-user-agent"
        | "stuck-request-timeout" => Value,
        "quiet"
        | "verbose"
        | "no-lock"
        | "json"
        | "no-cache"
        | "insecure-no-password"
        | "insecure-tls"
        | "no-extra-verify"
        | "cleanup-cache" => Bool,
        // backup
        "host"
        | "tag"
        | "path"
        | "exclude"
        | "exclude-file"
        | "iexclude"
        | "exclude-if-present"
        | "exclude-larger-than"
        | "time"
        | "files-from"
        | "files-from-raw"
        | "files-from-verbatim"
        | "parent"
        | "group-by"
        | "read-concurrency"
        | "scan-mode" => Value,
        "with-atime" | "force" | "dry-run" | "skip-if-unchanged" | "no-scan" | "ignore-ctime"
        | "ignore-inode" => Bool,
        // forget and snapshots
        "latest"
        | "keep-last"
        | "keep-hourly"
        | "keep-daily"
        | "keep-weekly"
        | "keep-monthly"
        | "keep-yearly"
        | "keep-within"
        | "keep-within-hourly"
        | "keep-within-daily"
        | "keep-within-weekly"
        | "keep-within-monthly"
        | "keep-within-yearly"
        | "keep-tag" => Value,
        "compact" | "last" | "prune" | "unsafe-allow-remove-all" => Bool,
        // prune
        "max-unused" | "max-repack-size" => Value,
        "repack-cacheable-only" | "repack-small" | "repack-uncompressed" => Bool,
        // check
        "read-data-subset" => Value,
        "read-data" | "with-cache" | "check-unused" => Bool,
        // restore
        "target" | "overwrite" | "include" => Value,
        "sparse" | "verify" | "delete" => Bool,
        // unlock and init
        "remove-all" | "copy-chunker-params" => Bool,
        "repository-version" => Value,
        _ => return None,
    })
}

fn short_flag(name: char) -> Option<FlagKind> {
    Some(match name {
        'r' | 'p' | 'o' | 't' | 'g' | 'l' | 'H' | 'd' | 'w' | 'm' | 'y' => FlagKind::Value,
        'q' | 'v' | 'c' | 'n' => FlagKind::Bool,
        _ => return None,
    })
}

/// Parses the arguments after the program name.
///
/// An unknown flag or command gives the same message and the same exit code as
/// restic. In particular restic has no `--password` option, so the parser
/// rejects it here (ADR-0021).
pub fn parse(args: &[OsString]) -> Result<Invocation, String> {
    let mut invocation = Invocation {
        command: Command::Version,
        global: GlobalFlags::default(),
        repository: None,
        password_file: None,
        arguments: Vec::new(),
    };
    let mut command_name: Option<String> = None;

    let mut index = 0;
    let mut literal = false;
    while index < args.len() {
        let arg = args[index].to_string_lossy().into_owned();
        if literal {
            remember_positional(&mut command_name, &mut invocation.arguments, arg);
            index += 1;
            continue;
        }
        if arg == "--" {
            literal = true;
            index += 1;
            continue;
        }
        if let Some(rest) = arg.strip_prefix("--") {
            let (name, inline) = match rest.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (rest, None),
            };
            let kind = long_flag(name).ok_or_else(|| format!("unknown flag: --{name}"))?;
            let value = match (kind, inline) {
                (FlagKind::Value, Some(value)) => value,
                (FlagKind::Value, None) => {
                    index += 1;
                    args.get(index)
                        .map(|value| value.to_string_lossy().into_owned())
                        .ok_or_else(|| format!("flag needs an argument: --{name}"))?
                }
                (FlagKind::Bool, _) => String::new(),
            };
            apply_long(name, kind, &value, &mut invocation);
            index += 1;
            continue;
        }
        if arg.len() > 1 && arg.starts_with('-') {
            let rest = &arg[1..];
            let mut chars = rest.chars();
            while let Some(name) = chars.next() {
                let kind = short_flag(name)
                    .ok_or_else(|| format!("unknown shorthand flag: '{name}' in -{rest}"))?;
                if kind == FlagKind::Bool {
                    continue;
                }
                let tail: String = chars.collect();
                let value = if tail.is_empty() {
                    index += 1;
                    args.get(index)
                        .map(|value| value.to_string_lossy().into_owned())
                        .ok_or_else(|| format!("flag needs an argument: -{name}"))?
                } else {
                    tail
                };
                apply_short(name, &value, &mut invocation);
                break;
            }
            index += 1;
            continue;
        }
        remember_positional(&mut command_name, &mut invocation.arguments, arg);
        index += 1;
    }

    let name = command_name.ok_or_else(|| "no command specified".to_string())?;
    invocation.command = command_for(&name, &mut invocation.arguments)?;
    Ok(invocation)
}

fn command_for(name: &str, arguments: &mut Vec<String>) -> Result<Command, String> {
    if name == "cat" {
        match arguments.first().map(String::as_str) {
            Some("config") => {
                arguments.remove(0);
                return Ok(Command::CatConfig);
            }
            Some(other) => return Err(format!("invalid type \"{other}\"")),
            None => return Err("type not specified".to_string()),
        }
    }
    Command::from_name(name).ok_or_else(|| format!("unknown command \"{name}\" for \"restic\""))
}

fn remember_positional(command: &mut Option<String>, arguments: &mut Vec<String>, arg: String) {
    if command.is_none() {
        *command = Some(arg);
    } else {
        arguments.push(arg);
    }
}

fn apply_long(name: &str, kind: FlagKind, value: &str, invocation: &mut Invocation) {
    match (name, kind) {
        ("json", FlagKind::Bool) => invocation.global.json = true,
        ("repo", FlagKind::Value) => invocation.repository = Some(value.to_string()),
        ("password-file", FlagKind::Value) => invocation.password_file = Some(value.to_string()),
        _ => {}
    }
}

fn apply_short(name: char, value: &str, invocation: &mut Invocation) {
    match name {
        'r' => invocation.repository = Some(value.to_string()),
        'p' => invocation.password_file = Some(value.to_string()),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, parse};
    use std::ffi::OsString;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_attached_option_values() {
        let invocation = parse(&args(&[
            "backup",
            "--json",
            "--repo=/srv/restic-repo",
            "--password-file=/etc/restic/pw",
            "/data",
        ]))
        .expect("parse");
        assert_eq!(invocation.command, Command::Backup);
        assert!(invocation.global.json);
        assert_eq!(invocation.repository.as_deref(), Some("/srv/restic-repo"));
        assert_eq!(invocation.password_file.as_deref(), Some("/etc/restic/pw"));
        assert_eq!(invocation.arguments, vec!["/data".to_string()]);
    }

    #[test]
    fn parses_separate_option_values() {
        let invocation = parse(&args(&["-r", "/srv/restic-repo", "snapshots"])).expect("parse");
        assert_eq!(invocation.command, Command::Snapshots);
        assert_eq!(invocation.repository.as_deref(), Some("/srv/restic-repo"));
    }

    #[test]
    fn positional_after_double_dash_is_not_a_flag() {
        let invocation = parse(&args(&["backup", "--json", "--", "--weird-path"])).expect("parse");
        assert_eq!(invocation.arguments, vec!["--weird-path".to_string()]);
    }

    #[test]
    fn rejects_the_password_flag() {
        let error = parse(&args(&["--password", "secret", "backup"])).expect_err("error");
        assert_eq!(error, "unknown flag: --password");
    }

    #[test]
    fn rejects_an_unknown_command() {
        let error = parse(&args(&["explode"])).expect_err("error");
        assert_eq!(error, "unknown command \"explode\" for \"restic\"");
    }

    #[test]
    fn parses_cat_config() {
        let invocation = parse(&args(&["cat", "config"])).expect("parse");
        assert_eq!(invocation.command, Command::CatConfig);
        assert!(invocation.arguments.is_empty());
    }

    #[test]
    fn version_needs_no_repository() {
        assert!(!Command::Version.needs_repository());
        assert!(Command::Backup.needs_repository());
    }
}
