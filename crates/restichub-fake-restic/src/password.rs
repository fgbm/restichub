//! Password resolution (FR-AG-06).
//!
//! The password comes from the environment or a file. The binary never reads a
//! password from an argument, and it never reads `RESTIC_PASSWORD_COMMAND`,
//! because the agent must not start a program from job data (ADR-0021).

use std::env;
use std::fs;

use crate::cli::Invocation;

/// Returns the repository password from the allowed sources.
pub fn resolve(invocation: &Invocation) -> Option<String> {
    if let Some(path) = &invocation.password_file {
        return read_password_file(path);
    }
    if let Ok(path) = env::var("RESTIC_PASSWORD_FILE")
        && !path.is_empty()
    {
        return read_password_file(&path);
    }
    if let Ok(password) = env::var("RESTIC_PASSWORD")
        && !password.is_empty()
    {
        return Some(password);
    }
    None
}

fn read_password_file(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
}
