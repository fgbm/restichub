//! ResticHub agent.
//!
//! The agent registers with a one-time token (ADR-0032), keeps its identity in the protected
//! configuration directory (FR-AG-04), runs as an operating system service (FR-AG-01) and keeps a
//! permanent outbound stream to the control plane ([`stream`], FR-ID-07).

pub mod acl;
pub mod cli;
pub mod commands;
pub mod enroll;
pub mod exec;
pub mod http_get;
pub mod identity;
pub mod jobs;
pub mod journal;
pub mod keys;
pub mod machine_id;
pub mod openpgp;
mod output;
pub mod paths;
pub mod pem;
pub mod preflight;
pub mod restic;
pub mod restic_install;
pub mod restic_release;
pub mod restic_store;
pub mod restore_roots;
pub mod rotation;
mod scrub;
pub mod service;
pub mod stream;
pub mod verify;
#[cfg(windows)]
pub mod winsec;
