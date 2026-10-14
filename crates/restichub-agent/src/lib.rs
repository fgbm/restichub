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
pub mod identity;
pub mod keys;
pub mod paths;
pub mod pem;
pub mod restic;
pub mod service;
pub mod stream;
pub mod verify;
#[cfg(windows)]
pub mod winsec;
