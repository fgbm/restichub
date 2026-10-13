//! ResticHub agent emulator for load tests (bench S2).
//!
//! One process runs N emulated agents over the protocol from `proto/`, which
//! `restichub-proto` generates. Each agent holds its own identity material. The
//! default executor waits for the step times of a profile; the optional
//! executor spawns `restichub-fake-restic`. An agent has no schedule (ADR-0002).
//!
//! Until epic #1 lands, [`identity`] makes test certificates in memory and the
//! `test-util` feature adds an in-process control plane in the `control`
//! module.

pub mod agent;
pub mod cli;
pub mod enroll;
pub mod executor;
pub mod identity;
pub mod job;
pub mod metrics;
pub mod profile;
pub mod runner;

#[cfg(feature = "test-util")]
pub mod control;

use prost_types::Timestamp;
use std::sync::{Mutex, MutexGuard};

/// Guards a mutex. A panic in another thread poisons the lock; the emulator
/// keeps the data usable, so this recovers it instead of panicking.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// Draws a value in `[0, 1)` from the system entropy source.
pub(crate) fn random_fraction() -> f64 {
    let mut bytes = [0u8; 8];
    // getrandom does not fail on a supported host. Zero is a safe fallback.
    let _ = getrandom::fill(&mut bytes);
    (u64::from_le_bytes(bytes) as f64) / (u64::MAX as f64)
}

/// Returns the current time as a Protocol Buffers timestamp.
#[must_use]
pub fn now_timestamp() -> Timestamp {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp {
        seconds: now.as_secs() as i64,
        nanos: now.subsec_nanos() as i32,
    }
}
