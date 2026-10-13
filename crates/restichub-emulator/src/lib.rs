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
pub mod executor;
pub mod identity;
pub mod job;

#[cfg(feature = "test-util")]
pub mod control;

use prost_types::Timestamp;
use std::sync::{Mutex, MutexGuard};

/// Guards a mutex. A panic in another thread poisons the lock; the emulator
/// keeps the data usable, so this recovers it instead of panicking.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
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
