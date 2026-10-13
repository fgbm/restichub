//! Lock simulation for the fake restic (FR-SC-05, FR-SC-06, AC-SC-05, AC-SC-06).
//!
//! restic locks a repository with a file in the `locks` directory and exits
//! with code 11 when the repository is already locked. The fake writes one
//! marker file and never reaches a real repository. The marker survives a hard
//! kill, so a test can observe the stale lock that FR-SC-06 recovers from.

use std::io;
use std::path::{Path, PathBuf};

/// Storage ID of the marker. restic names a lock file with its storage ID.
pub const LOCK_ID: &str = "b2f8a1c94e7d60351a8f2b4c6d9e0f3a7c5b1d2e4f6089a3b7c1d5e9f2a406b8";

/// The `locks` directory of the repository, or nothing without a repository.
pub fn lock_dir(repository: Option<&str>) -> Option<PathBuf> {
    let repository = repository?;
    Some(Path::new(repository).join("locks"))
}

/// Full path inside the lock directory.
pub fn marker(dir: &Path) -> PathBuf {
    dir.join(LOCK_ID)
}

/// True when at least one marker file exists.
pub fn locked(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

/// Writes the marker file and returns its path.
pub fn create(dir: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = marker(dir);
    std::fs::write(&path, document())?;
    Ok(path)
}

/// Removes every lock of the directory. The `unlock` command calls it.
pub fn remove_all(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The message of a locked repository, in the shape of restic
/// `cmd/restic/main.go` for exit code 11.
pub fn conflict_message() -> String {
    format!(
        "repository is already locked exclusively by PID {} on {} by backup (UID 0, GID 0)\n\
         lock was created at 2026-01-01 00:00:00 (0s ago)\n\
         storage ID {LOCK_ID}\n\
         the `unlock` command can be used to remove stale locks",
        std::process::id(),
        hostname(),
    )
}

fn document() -> String {
    serde_json::json!({
        "time": "2026-01-01T00:00:00Z",
        "exclusive": true,
        "hostname": hostname(),
        "username": "backup",
        "pid": std::process::id(),
    })
    .to_string()
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "test-host".to_string())
}
