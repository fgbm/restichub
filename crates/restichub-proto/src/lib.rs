//! Protocol Buffers contract between the ResticHub agent and the control plane.
//!
//! The `.proto` files are in `proto/`. The build script generates the Rust
//! code. See ADR-0008. Job signing is in [`signing`], and argument validation
//! is in [`validate`].

/// Major version of the agent protocol. The agent and the control plane compare
/// it at connection time. A different major version is incompatible.
pub const PROTOCOL_VERSION_MAJOR: u32 = 1;

/// Minor version of the agent protocol. The control plane adds capabilities
/// only. See `AgentHello` and `ServerHello`.
pub const PROTOCOL_VERSION_MINOR: u32 = 0;

/// Messages and services of the `restichub.agent.v1` package.
pub mod agent {
    /// Version 1 of the agent protocol.
    pub mod v1 {
        tonic::include_proto!("restichub.agent.v1");
    }
}

pub mod signing;

/// Argument validation of a job before signing and before execution (FR-PR-10).
pub mod validate;

/// Manual `Debug` implementations that keep secrets out of log output.
mod debug;
