//! The restic download against a local HTTPS server (ADR-0038, FR-AG-11, NFR-SEC-06).

#![allow(clippy::expect_used)]

mod http;
#[cfg(unix)]
mod install;
mod server;
