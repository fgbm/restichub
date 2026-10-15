//! ResticHub control plane.
//!
//! The crate holds the configuration, the HTTP server, the local authentication and the error type
//! of the control plane (ADR-0023).

pub mod agent_listener;
pub mod agent_stream;
pub mod agents;
pub mod api_tokens;
pub mod audit;
pub mod auth;
pub mod browse;
pub mod client_ip;
pub mod clock;
pub mod config;
pub mod context;
pub mod delivery;
pub mod enrollment;
pub mod enrollment_tokens;
pub mod error;
pub mod harden;
pub mod instance;
pub mod job_api;
pub mod job_data;
pub mod job_detail;
pub mod job_signing;
pub mod jobs;
pub mod json;
pub mod log_store;
pub mod maintenance;
pub mod maintenance_scheduler;
pub mod metrics;
pub mod password;
pub mod pki;
pub mod plans;
pub mod platform_users;
pub mod profiles;
pub mod queue;
pub mod rate_limit;
pub mod repositories;
pub mod repository_maintenance;
pub mod restore;
pub mod routes;
pub mod schedule;
pub mod scheduler;
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
