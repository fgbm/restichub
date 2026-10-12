//! ResticHub control plane.
//!
//! The crate holds the configuration, the HTTP server and the error type of the control plane
//! (ADR-0023). Later tasks add the domain logic.

pub mod config;
pub mod error;
pub mod server;
pub mod telemetry;
