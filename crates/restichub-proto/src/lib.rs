//! Protocol Buffers contract between the ResticHub agent and the control plane.
//!
//! The `.proto` files are in `proto/`. The build script generates the Rust
//! code. See ADR-0008. Job signing is in [`signing`], and argument validation
//! is in [`validate`].

/// The agent and the control plane compare it at connection time. A different
/// major version is incompatible.
pub const PROTOCOL_VERSION_MAJOR: u32 = 1;

/// The control plane adds capabilities only. See `AgentHello` and
/// `ServerHello`.
pub const PROTOCOL_VERSION_MINOR: u32 = 0;

pub mod agent {
    pub mod v1 {
        tonic::include_proto!("restichub.agent.v1");
    }
}

pub mod signing;

/// Argument validation of a job before signing and before execution (FR-PR-10).
pub mod validate;

mod debug;
