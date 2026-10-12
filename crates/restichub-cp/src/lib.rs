//! ResticHub control plane.
//!
//! The crate holds the configuration, the HTTP server and the error type of the control plane
//! (ADR-0023). Later tasks add the domain logic.

pub mod audit;
pub mod client_ip;
pub mod config;
pub mod context;
pub mod error;
pub mod server;
pub mod telemetry;
