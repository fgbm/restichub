#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};

mod rotation;
mod sinks;

pub(super) fn temp_dir(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("restichub-agent-log-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("cannot create the directory");
    dir
}

pub(super) fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("cannot list the directory")
        .map(|entry| {
            entry
                .expect("cannot read an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

pub(super) fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).expect("cannot read the log file")
}

/// The log files of `dir` in their order, as one text.
pub(super) fn read_all(dir: &Path) -> String {
    names(dir)
        .iter()
        .filter(|name| name.starts_with("agent-"))
        .map(|name| read(dir, name))
        .collect()
}
