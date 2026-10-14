//! ResticHub agent.
//!
//! The agent registers with a one-time token (ADR-0032), keeps its identity in the protected
//! configuration directory (FR-AG-04) and runs as an operating system service (FR-AG-01). The
//! protocol stream arrives with #50; [`commands::run`] is a stub until then.

pub mod acl;
pub mod cli;
pub mod commands;
pub mod enroll;
pub mod identity;
pub mod paths;
pub mod pem;
pub mod restic;
pub mod service;
pub mod verify;
#[cfg(windows)]
pub mod winsec;
