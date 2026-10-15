//! The interpreters of a host and their fixed command lines (FR-HK-09).
//!
//! The flags are not configurable. They keep the profile, the policy and the startup files of the
//! account out of the run, so a script behaves the same on every start.

use std::path::{Path, PathBuf};

use restichub_proto::agent::v1::ScriptInterpreter;

use crate::hooks::{Trust, trust};

/// The interpreters that exist on this host and that the agent trusts.
pub(super) fn available(trust: Trust) -> Vec<ScriptInterpreter> {
    [
        ScriptInterpreter::Powershell,
        ScriptInterpreter::Cmd,
        ScriptInterpreter::Bash,
        ScriptInterpreter::Sh,
    ]
    .into_iter()
    .filter(|kind| locate(*kind, trust).is_some())
    .collect()
}

/// The absolute path of an interpreter, or `None` when the host lacks it or the agent does not
/// trust the file. The host account must own the whole chain of the path (threat T33).
pub(super) fn locate(kind: ScriptInterpreter, trust: Trust) -> Option<PathBuf> {
    candidates(kind).into_iter().find_map(|path| match trust {
        Trust::Host => trust::check_executable(&path, Trust::Host).ok(),
        // A test host has no interpreter that the current account owns.
        Trust::CurrentUser => path
            .canonicalize()
            .ok()
            .filter(|resolved| resolved.is_file()),
    })
}

#[cfg(windows)]
fn candidates(kind: ScriptInterpreter) -> Vec<PathBuf> {
    let root =
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let system32 = root.join("System32");
    match kind {
        ScriptInterpreter::Powershell => {
            vec![
                system32
                    .join("WindowsPowerShell")
                    .join("v1.0")
                    .join("powershell.exe"),
            ]
        }
        ScriptInterpreter::Cmd => vec![system32.join("cmd.exe")],
        _ => Vec::new(),
    }
}

#[cfg(not(windows))]
fn candidates(kind: ScriptInterpreter) -> Vec<PathBuf> {
    match kind {
        ScriptInterpreter::Bash => vec![PathBuf::from("/bin/bash"), PathBuf::from("/usr/bin/bash")],
        ScriptInterpreter::Sh => vec![PathBuf::from("/bin/sh"), PathBuf::from("/usr/bin/sh")],
        _ => Vec::new(),
    }
}

/// The name of the script file. PowerShell and cmd choose the language by the extension.
pub(super) fn file_name(kind: ScriptInterpreter) -> &'static str {
    match kind {
        ScriptInterpreter::Powershell => "script.ps1",
        ScriptInterpreter::Cmd => "script.cmd",
        _ => "script.sh",
    }
}

/// The bytes of the script file. Windows PowerShell reads UTF-8 only with a byte order mark, and a
/// batch file needs CRLF line ends for labels.
pub(super) fn file_bytes(kind: ScriptInterpreter, body: &str) -> Vec<u8> {
    match kind {
        ScriptInterpreter::Powershell => {
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(body.as_bytes());
            bytes
        }
        ScriptInterpreter::Cmd => body
            .replace("\r\n", "\n")
            .replace('\n', "\r\n")
            .into_bytes(),
        _ => body.as_bytes().to_vec(),
    }
}

/// The arguments of the interpreter, with the script file last.
pub(super) fn arguments(kind: ScriptInterpreter, file: &Path) -> Vec<String> {
    let file = file.to_string_lossy().into_owned();
    match kind {
        ScriptInterpreter::Powershell => [
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain([file])
        .collect(),
        ScriptInterpreter::Cmd => ["/D", "/E:ON", "/V:OFF", "/C"]
            .into_iter()
            .map(str::to_owned)
            .chain([file])
            .collect(),
        ScriptInterpreter::Bash => ["--noprofile", "--norc"]
            .into_iter()
            .map(str::to_owned)
            .chain([file])
            .collect(),
        ScriptInterpreter::Sh | ScriptInterpreter::Unspecified => vec![file],
    }
}
