//! ResticHub control plane.
//!
//! The crate holds the configuration, the HTTP server, the local authentication and the error type
//! of the control plane (ADR-0023).

pub mod api_tokens;
pub mod audit;
pub mod auth;
pub mod client_ip;
pub mod clock;
pub mod config;
pub mod context;
pub mod delivery;
pub mod error;
pub mod instance;
pub mod json;
pub mod maintenance;
pub mod password;
pub mod pki;
pub mod rate_limit;
pub mod routes;
pub mod secrets;
pub mod server;
pub mod setup;
pub mod startup;
pub mod telemetry;
pub mod tenants;
pub mod token;
pub mod ui;
pub mod users;
pub mod validation;
pub mod views;
