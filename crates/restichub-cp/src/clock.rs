//! The clock abstraction of the control plane.
//!
//! The session and token checks read the current time from a [`Clock`]. A test gives a
//! [`ManualClock`] and advances it, so the test checks the idle and absolute expiry without a
//! sleep.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use time::OffsetDateTime;

/// A source of the current time.
pub trait Clock: Send + Sync {
    /// Returns the current time in UTC.
    fn now(&self) -> OffsetDateTime;
}

/// The clock of a running control plane. It reads the system time.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

/// A clock that a test controls.
///
/// The clock holds a time in nanoseconds since the Unix epoch. [`ManualClock::advance`] moves it
/// forward.
#[derive(Debug)]
pub struct ManualClock {
    nanos: AtomicI64,
}

impl ManualClock {
    /// Creates a clock at the given time.
    pub fn new(start: OffsetDateTime) -> Self {
        Self {
            nanos: AtomicI64::new(start.unix_timestamp_nanos() as i64),
        }
    }

    /// Moves the clock forward.
    pub fn advance(&self, seconds: i64) {
        self.nanos
            .fetch_add(seconds * 1_000_000_000, Ordering::SeqCst);
    }

    /// Returns the clock as a shared value.
    pub fn shared(self) -> Arc<dyn Clock> {
        Arc::new(self)
    }
}

impl Clock for ManualClock {
    fn now(&self) -> OffsetDateTime {
        let nanos = self.nanos.load(Ordering::SeqCst);
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(nanos))
            .unwrap_or(OffsetDateTime::UNIX_EPOCH)
    }
}
