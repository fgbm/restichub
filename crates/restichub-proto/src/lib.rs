//! Protocol Buffers contract between the ResticHub agent and the control plane.
//!
//! The `.proto` files are in `proto/`. The build script generates the Rust
//! code. See ADR-0008. Job signing is in [`signing`], and argument validation
//! is in [`validate`].

/// The agent and the control plane compare it at connection time. A different
/// major version is incompatible.
pub const PROTOCOL_VERSION_MAJOR: u32 = 1;

/// A minor version adds messages and fields only. Version 1 adds the certificate rotation
/// messages and the version refusal (FR-ID-03, FR-PR-09). See `AgentHello` and `ServerHello`.
pub const PROTOCOL_VERSION_MINOR: u32 = 1;

/// The largest stream message that either side accepts or sends. A step result stays below
/// [`MAX_STEP_RESULT_BYTES`], so it always fits with room for the other fields (FR-AG-08).
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// The most data bytes in one log chunk. The agent splits a longer text; the control plane refuses
/// a longer chunk (FR-LG-07).
pub const MAX_LOG_CHUNK_BYTES: usize = 64 * 1024;

/// The largest encoded size of one step result that the agent sends (FR-AG-08).
pub const MAX_STEP_RESULT_BYTES: usize = 3 * 1024 * 1024;

pub mod agent {
    pub mod v1 {
        tonic::include_proto!("restichub.agent.v1");
    }
}

pub mod signing;

pub mod version;

mod capability;
pub use capability::AgentCapability;

pub mod restore_path;

/// Argument validation of a job before signing and before execution (FR-PR-10).
pub mod validate;

mod debug;
