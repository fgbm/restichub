//! A script step in a transient systemd unit (ADR-0053).
//!
//! The unit of the agent is hardened for restic: `ProtectSystem=strict`, no writable and executable
//! memory, a system call filter and a reduced capability set. A script is arbitrary code that the
//! host administrator trusts, and those settings break ordinary tools: `chpst` needs `CAP_SETGID`,
//! Ruby FFI needs a writable and executable page, and a dump outside the writable roots fails.
//!
//! The agent therefore starts a script as its own transient service. The service runs in the file
//! system namespace of the service manager, so the sandbox of the agent does not apply. The
//! environment reaches the interpreter through a protected file and a shell that only sources it
//! and execs the interpreter, so a value keeps every byte and never appears in an argument.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result};

use super::TransientUnit;
use crate::exec::{Invocation, JobChild};

/// The name of the transient unit, up to its random part and the `.service` suffix.
pub(crate) const UNIT_PREFIX: &str = "restichub-script-";

pub(crate) const UNIT_SUFFIX: &str = ".service";

/// Starts a transient unit and waits for it (ADR-0053).
const SYSTEMD_RUN: &str = "systemd-run";

/// Signals a unit through the service manager (ADR-0053).
const SYSTEMCTL: &str = "systemctl";

/// The shell line of the unit: export every variable of the file, then run the interpreter.
///
/// `$0` is a label, `$1` is the environment file and `$@` is the interpreter with its arguments.
pub(crate) const SHELL_WRAPPER: &str = "set -a; . \"$1\"; shift; exec \"$@\"";

/// The grace period that the unit gets after the timeout of the agent.
const RUNTIME_BACKSTOP: Duration = Duration::from_secs(60);

/// True when this host runs systemd and has the launcher.
pub(crate) fn available() -> bool {
    Path::new("/run/systemd/system").is_dir() && on_path(SYSTEMD_RUN) && on_path(SYSTEMCTL)
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(name).is_file())
    })
}

/// Builds a unit name from the random part of the run (ADR-0053).
pub(crate) fn unit_name(random: &str) -> String {
    format!("{UNIT_PREFIX}{random}{UNIT_SUFFIX}")
}

/// The environment file of one run, in the shell syntax of [`SHELL_WRAPPER`].
///
/// A value is single-quoted, so a quote, a newline and a backslash keep their bytes. The name comes
/// from the protocol and is a shell identifier, so it needs no quoting.
pub(crate) fn environment_file(env: &[(String, zeroize::Zeroizing<String>)]) -> String {
    let mut text = String::new();
    for (name, value) in env {
        text.push_str(name);
        text.push('=');
        text.push_str(&shell_quote(value));
        text.push('\n');
    }
    text
}

/// Quotes one value for a POSIX shell.
pub(crate) fn shell_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for character in value.chars() {
        if character == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(character);
        }
    }
    quoted.push('\'');
    quoted
}

/// The arguments of `systemd-run` for one script (ADR-0053).
pub(crate) fn run_arguments(invocation: &Invocation, unit: &TransientUnit) -> Vec<OsString> {
    let mut arguments: Vec<OsString> = [
        "--quiet",
        "--collect",
        "--wait",
        "--pipe",
        "--property=KillMode=control-group",
        "--property=PrivateTmp=yes",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    arguments.push(OsString::from(format!("--unit={}", unit.name)));
    arguments.push(OsString::from(format!(
        "--property=RuntimeMaxSec={}",
        unit.runtime.as_secs().max(1)
    )));
    arguments.push(OsString::from(format!(
        "--property=WorkingDirectory={}",
        invocation.working_dir.display()
    )));
    if let Some(path) = &invocation.search_path {
        arguments.push(OsString::from(format!(
            "--setenv=PATH={}",
            path.to_string_lossy()
        )));
    }
    arguments.extend([
        OsString::from("/bin/sh"),
        OsString::from("-c"),
        OsString::from(SHELL_WRAPPER),
        OsString::from("restichub-script"),
        unit.env_file.clone().into_os_string(),
        invocation.program.clone().into_os_string(),
    ]);
    arguments.extend(invocation.args.iter().map(OsString::from));
    arguments
}

/// Starts the transient unit of one script (ADR-0053).
///
/// # Errors
///
/// Returns an error when the operating system refuses to start `systemd-run`.
pub(crate) fn spawn(invocation: &Invocation, unit: &TransientUnit) -> Result<JobChild> {
    let mut command = Command::new(SYSTEMD_RUN);
    command
        .args(run_arguments(invocation, unit))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::exec::apply_process_group(&mut command);
    let child = command
        .spawn()
        .context("cannot start the transient script unit")?;
    Ok(JobChild::for_unit(child, unit.name.clone()))
}

/// The limit that ends a unit when no cancel reaches it (ADR-0053).
pub(crate) fn runtime_backstop(timeout: Duration) -> Duration {
    timeout.saturating_add(RUNTIME_BACKSTOP)
}

/// Stops the units that a crashed agent left behind (ADR-0053).
pub(crate) fn stop_leftovers() {
    let pattern = format!("{UNIT_PREFIX}*{UNIT_SUFFIX}");
    let listed = Command::new(SYSTEMCTL)
        .args([
            "list-units",
            "--type=service",
            "--all",
            "--plain",
            "--no-legend",
        ])
        .arg(&pattern)
        .output();
    let listed = match listed {
        Ok(listed) if listed.status.success() => listed,
        Ok(listed) => {
            tracing::warn!(
                status = %listed.status,
                "cannot list the transient script units"
            );
            return;
        }
        Err(error) => {
            tracing::warn!(%error, "cannot run systemctl");
            return;
        }
    };
    for line in String::from_utf8_lossy(&listed.stdout).lines() {
        let Some(name) = line.split_whitespace().next() else {
            continue;
        };
        if !name.starts_with(UNIT_PREFIX) || !name.ends_with(UNIT_SUFFIX) {
            continue;
        }
        match Command::new(SYSTEMCTL).args(["stop", name]).output() {
            Ok(output) if output.status.success() => {
                tracing::info!(unit = name, "stopped a leftover script unit");
            }
            Ok(output) => {
                tracing::warn!(unit = name, status = %output.status, "cannot stop a leftover script unit")
            }
            Err(error) => {
                tracing::warn!(%error, unit = name, "cannot stop a leftover script unit");
                return;
            }
        }
    }
}

/// The signal that a unit receives for a graceful stop or a forced stop (ADR-0053).
pub(crate) fn kill_arguments(unit: &str, signal: &str) -> [OsString; 3] {
    [
        OsString::from("kill"),
        OsString::from(format!("--signal={signal}")),
        OsString::from(unit),
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::path::PathBuf;
    use std::time::Duration;

    use zeroize::Zeroizing;

    use super::{kill_arguments, run_arguments, shell_quote};
    use crate::exec::Invocation;
    use crate::scripts::TransientUnit;

    fn invocation() -> Invocation {
        Invocation {
            program: PathBuf::from("/bin/bash"),
            args: vec![
                "--noprofile".to_owned(),
                "--norc".to_owned(),
                "/run/script.sh".to_owned(),
            ],
            env: vec![("PGPASSWORD".to_owned(), Zeroizing::new("secret".to_owned()))],
            working_dir: PathBuf::from("/var/cache/restichub-agent/scripts/0123abcd"),
            timeout: Duration::from_secs(600),
            secrets: Vec::new(),
            search_path: Some(std::ffi::OsString::from("/usr/bin:/bin")),
            unit: None,
        }
    }

    fn unit() -> TransientUnit {
        TransientUnit {
            name: "restichub-script-0123abcd.service".to_owned(),
            env_file: PathBuf::from("/var/cache/restichub-agent/scripts/0123abcd/environment.sh"),
            runtime: Duration::from_secs(660),
        }
    }

    #[test]
    fn adr_0053_the_unit_leaves_the_agent_sandbox_and_keeps_its_own_limits() {
        let arguments: Vec<String> = run_arguments(&invocation(), &unit())
            .into_iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        for required in [
            "--quiet",
            "--collect",
            "--wait",
            "--pipe",
            "--unit=restichub-script-0123abcd.service",
            "--property=KillMode=control-group",
            "--property=PrivateTmp=yes",
            "--property=RuntimeMaxSec=660",
            "--property=WorkingDirectory=/var/cache/restichub-agent/scripts/0123abcd",
            "--setenv=PATH=/usr/bin:/bin",
            "set -a; . \"$1\"; shift; exec \"$@\"",
            "/var/cache/restichub-agent/scripts/0123abcd/environment.sh",
        ] {
            assert!(
                arguments.iter().any(|argument| argument == required),
                "{required} is missing"
            );
        }
        assert!(
            !arguments.iter().any(|argument| argument.contains("secret")),
            "the secret never reaches an argument"
        );
        assert_eq!(arguments.last().map(String::as_str), Some("/run/script.sh"));
    }

    #[test]
    fn adr_0053_a_script_stops_with_a_signal_to_the_whole_unit() {
        let arguments: Vec<String> = kill_arguments("restichub-script-x.service", "SIGINT")
            .into_iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            arguments,
            vec![
                "kill".to_owned(),
                "--signal=SIGINT".to_owned(),
                "restichub-script-x.service".to_owned()
            ]
        );
    }

    #[test]
    fn adr_0053_a_quote_and_a_newline_survive_the_quoting() {
        assert_eq!(shell_quote("plain"), "'plain'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("a\nb"), "'a\nb'");
        assert_eq!(shell_quote("$HOME\"\\"), "'$HOME\"\\'");
    }
}
