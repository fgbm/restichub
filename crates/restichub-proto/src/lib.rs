//! Protocol Buffers contract between the ResticHub agent and the control plane.
//!
//! The `.proto` files and the generated code will go here. See ADR-0008.

/// Version of the agent protocol. The agent and the control plane compare it at connection time.
pub const PROTOCOL_VERSION: u32 = 0;
