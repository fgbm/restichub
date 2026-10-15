//! The search path of a script: only directories that the host account controls (ADR-0037).
//!
//! The service runs as root or LocalSystem and inherits a `PATH` that often holds a directory which
//! a normal user can write, mostly on Windows. A script that calls `pg_dump` would then run a file
//! planted there with the rights of the service. The agent keeps a directory only when it passes
//! the same chain check as an interpreter ([`trust::check_directory`], AC-HK-08).

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::trust::{self, Trust};

/// The search path that a script gets and the entries that the agent dropped from it.
pub(super) struct SearchPath {
    pub(super) value: OsString,
    pub(super) dropped: Vec<PathBuf>,
}

/// Keeps the trusted directories of `path`, or of the default list when the service has none.
pub(super) fn trusted(path: Option<&OsStr>, trust: Trust) -> SearchPath {
    let entries: Vec<PathBuf> = match path {
        Some(path) => std::env::split_paths(path).collect(),
        None => default_entries(),
    };
    let mut kept: Vec<PathBuf> = Vec::new();
    let mut dropped = Vec::new();
    for entry in entries {
        match trust::check_directory(&entry, trust) {
            Ok(resolved) => {
                let resolved = without_verbatim_prefix(resolved);
                if !kept.contains(&resolved) {
                    kept.push(resolved);
                }
            }
            Err(_) => dropped.push(entry),
        }
    }
    SearchPath {
        value: std::env::join_paths(&kept).unwrap_or_default(),
        dropped,
    }
}

/// Turns `\\?\C:\dir` into `C:\dir` and `\\?\UNC\host\share` into `\\host\share`. `canonicalize`
/// returns the verbatim form on Windows, and cmd.exe, PowerShell and many programs do not search a
/// directory in that form. Any other path stays as it is.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest)
            if rest.as_bytes().get(1) == Some(&b':')
                && rest.as_bytes()[0].is_ascii_alphabetic() =>
        {
            PathBuf::from(rest)
        }
        _ => path,
    }
}

#[cfg(windows)]
fn default_entries() -> Vec<PathBuf> {
    let root =
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    vec![root.join("System32"), root]
}

#[cfg(not(windows))]
fn default_entries() -> Vec<PathBuf> {
    [
        "/usr/local/sbin",
        "/usr/local/bin",
        "/usr/sbin",
        "/usr/bin",
        "/sbin",
        "/bin",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

/// The text for the step log: one line, with the dropped entries.
pub(super) fn dropped_text(dropped: &[PathBuf]) -> String {
    let names: Vec<String> = dropped
        .iter()
        .map(|entry| entry.display().to_string())
        .collect();
    format!(
        "the search path of the script omits {} that the host account does not control: {}",
        if names.len() == 1 {
            "a directory"
        } else {
            "directories"
        },
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac_hk_08_the_verbatim_prefix_leaves_the_search_path() {
        let cases = [
            (r"\\?\C:\Windows\System32", r"C:\Windows\System32"),
            (r"\\?\UNC\host\share\bin", r"\\host\share\bin"),
            (r"C:\Windows", r"C:\Windows"),
            (r"\\?\Volume{1234}\bin", r"\\?\Volume{1234}\bin"),
            ("/usr/bin", "/usr/bin"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                without_verbatim_prefix(PathBuf::from(input)),
                PathBuf::from(expected),
                "{input}"
            );
        }
    }
}
