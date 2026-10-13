//! Cancel handling for the fake restic (FR-AG-10, AC-AG-06, AC-AG-07).
//!
//! The `ctrlc` crate is the signal layer. It is safe Rust, and it covers both
//! targets with one dependency: SIGINT, SIGTERM and SIGHUP on Unix (the
//! `termination` feature), and the console events Ctrl+C and Ctrl+Break on
//! Windows. restic also exits with code 130 on both targets, so the fake does
//! the same.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use crate::scenario::Cancel;

/// The lock file to remove on a graceful cancel, so that the repository holds
/// no lock after the exit (AC-AG-06).
static LOCK_FILE: OnceLock<PathBuf> = OnceLock::new();

pub fn remember_lock_file(path: PathBuf) {
    let _ = LOCK_FILE.set(path);
}

/// Registers the handler for the signals of the scenario.
///
/// An ignoring scenario returns from the handler, so the process continues and
/// the caller must kill it (AC-AG-07).
pub fn install(cancel: &Cancel) {
    let ignore = cancel.ignore;
    let delay = Duration::from_millis(cancel.delay_ms);
    if let Err(error) = ctrlc::set_handler(move || {
        if ignore {
            return;
        }
        // The line follows cmd/restic/cleanup.go, which prints "\rsignal %v
        // received, cleaning up \n" on the signal.
        eprint!("\rsignal interrupt received, cleaning up \n");
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        if let Some(path) = LOCK_FILE.get() {
            let _ = std::fs::remove_file(path);
        }
        std::process::exit(130);
    }) {
        eprintln!("restichub-fake-restic: cannot install the cancel handler: {error}");
    }
}
