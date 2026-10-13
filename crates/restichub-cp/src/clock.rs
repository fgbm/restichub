//! Clock abstraction of the control plane.
//!
//! The session and token checks read the current time from a [`Clock`]. A test gives a
//! [`ManualClock`] and advances it, so the test checks the idle and absolute expiry without a
//! sleep.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use time::OffsetDateTime;

pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

/// The clock holds a time in nanoseconds since the Unix epoch. [`ManualClock::advance`] moves it
/// forward.
#[derive(Debug)]
pub struct ManualClock {
    nanos: AtomicI64,
}

impl ManualClock {
    pub fn new(start: OffsetDateTime) -> Self {
        Self {
            nanos: AtomicI64::new(start.unix_timestamp_nanos() as i64),
        }
    }

    pub fn advance(&self, seconds: i64) {
        self.nanos
            .fetch_add(seconds * 1_000_000_000, Ordering::SeqCst);
    }

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
