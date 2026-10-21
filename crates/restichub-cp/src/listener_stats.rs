//! Process counters of the agent listener for `/metrics` and the operator log.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::time::Duration;

/// The agent listener writes one log line per interval with the events of that interval.
pub const LOG_INTERVAL: Duration = Duration::from_secs(60);

/// The limit at which the listener closed a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalReason {
    Cap,
    AnonymousShare,
    AnonymousPerIp,
    Handshakes,
    HandshakesPerIp,
}

impl RefusalReason {
    pub const ALL: [Self; 5] = [
        Self::Cap,
        Self::AnonymousShare,
        Self::AnonymousPerIp,
        Self::Handshakes,
        Self::HandshakesPerIp,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cap => "cap",
            Self::AnonymousShare => "anonymous_share",
            Self::AnonymousPerIp => "anonymous_per_ip",
            Self::Handshakes => "handshakes",
            Self::HandshakesPerIp => "handshakes_per_ip",
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Cap => 0,
            Self::AnonymousShare => 1,
            Self::AnonymousPerIp => 2,
            Self::Handshakes => 3,
            Self::HandshakesPerIp => 4,
        }
    }
}

#[derive(Debug, Default)]
pub struct ListenerStats {
    refused: [AtomicU64; RefusalReason::ALL.len()],
    accept_errors: AtomicU64,
    /// The OS error code of the last accept error, 0 when it had none.
    last_accept_error: AtomicI32,
}

impl ListenerStats {
    pub fn record_refusal(&self, reason: RefusalReason) {
        self.refused[reason.index()].fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_accept_error(&self, error: &io::Error) {
        self.last_accept_error
            .store(error.raw_os_error().unwrap_or(0), Ordering::Relaxed);
        self.accept_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn refused(&self, reason: RefusalReason) -> u64 {
        self.refused[reason.index()].load(Ordering::Relaxed)
    }

    pub fn accept_errors(&self) -> u64 {
        self.accept_errors.load(Ordering::Relaxed)
    }

    fn counts(&self) -> Counts {
        Counts {
            refused: RefusalReason::ALL.map(|reason| self.refused(reason)),
            accept_errors: self.accept_errors(),
        }
    }

    /// Writes a warning each [`LOG_INTERVAL`] that had events, with the count of each kind.
    ///
    /// A periodic line keeps the hot path free of locks, gives each reason its own count and
    /// reports the end of a burst at the next tick.
    pub async fn report(self: Arc<Self>, connections: impl Fn() -> usize, max_connections: usize) {
        let mut interval =
            tokio::time::interval_at(tokio::time::Instant::now() + LOG_INTERVAL, LOG_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut previous = self.counts();
        loop {
            interval.tick().await;
            let current = self.counts();
            let Some(delta) = current.since(previous) else {
                continue;
            };
            previous = current;
            let refused = |reason: RefusalReason| delta.refused[reason.index()];
            let last_error = match self.last_accept_error.load(Ordering::Relaxed) {
                code if code != 0 && delta.accept_errors > 0 => {
                    Some(io::Error::from_raw_os_error(code).to_string())
                }
                _ => None,
            };
            tracing::warn!(
                cap = refused(RefusalReason::Cap),
                anonymous_share = refused(RefusalReason::AnonymousShare),
                anonymous_per_ip = refused(RefusalReason::AnonymousPerIp),
                handshakes = refused(RefusalReason::Handshakes),
                handshakes_per_ip = refused(RefusalReason::HandshakesPerIp),
                accept_errors = delta.accept_errors,
                last_accept_error = last_error.as_deref(),
                connections = connections(),
                max_connections,
                "the agent listener closed connections at its limits or cannot accept them; the counts are for the last interval"
            );
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Counts {
    refused: [u64; RefusalReason::ALL.len()],
    accept_errors: u64,
}

impl Counts {
    /// Returns the events after `earlier`, or `None` when there are none.
    fn since(self, earlier: Self) -> Option<Self> {
        let mut delta = Self {
            refused: [0; RefusalReason::ALL.len()],
            accept_errors: self.accept_errors.saturating_sub(earlier.accept_errors),
        };
        for (slot, (now, then)) in delta
            .refused
            .iter_mut()
            .zip(self.refused.iter().zip(earlier.refused))
        {
            *slot = now.saturating_sub(then);
        }
        (delta != Self::default()).then_some(delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_reason_has_the_index_of_its_place_in_all() {
        for (index, reason) in RefusalReason::ALL.into_iter().enumerate() {
            assert_eq!(reason.index(), index, "{reason:?}");
        }
    }

    #[test]
    fn each_reason_has_its_own_counter() {
        let stats = ListenerStats::default();
        stats.record_refusal(RefusalReason::Cap);
        stats.record_refusal(RefusalReason::Cap);
        stats.record_refusal(RefusalReason::HandshakesPerIp);
        assert_eq!(stats.refused(RefusalReason::Cap), 2);
        assert_eq!(stats.refused(RefusalReason::HandshakesPerIp), 1);
        assert_eq!(stats.refused(RefusalReason::AnonymousShare), 0);
    }

    #[test]
    fn mixed_reasons_in_one_interval_count_each_reason() {
        let stats = ListenerStats::default();
        let start = stats.counts();
        for _ in 0..3 {
            stats.record_refusal(RefusalReason::Cap);
        }
        stats.record_refusal(RefusalReason::Handshakes);
        stats.record_refusal(RefusalReason::HandshakesPerIp);
        stats.record_accept_error(&io::Error::from_raw_os_error(24));

        let delta = stats
            .counts()
            .since(start)
            .expect("the interval had events");
        assert_eq!(delta.refused[RefusalReason::Cap.index()], 3);
        assert_eq!(delta.refused[RefusalReason::Handshakes.index()], 1);
        assert_eq!(delta.refused[RefusalReason::HandshakesPerIp.index()], 1);
        assert_eq!(delta.refused[RefusalReason::AnonymousShare.index()], 0);
        assert_eq!(delta.accept_errors, 1);
        assert_eq!(stats.last_accept_error.load(Ordering::Relaxed), 24);
    }

    #[test]
    fn a_burst_then_silence_shows_in_full_at_the_next_tick() {
        let stats = ListenerStats::default();
        let start = stats.counts();
        for _ in 0..600 {
            stats.record_accept_error(&io::Error::from_raw_os_error(24));
        }
        let first_tick = stats.counts();
        let delta = first_tick.since(start).expect("the burst");
        assert_eq!(delta.accept_errors, 600);
        assert_eq!(delta.refused, [0; RefusalReason::ALL.len()]);
        assert_eq!(stats.counts().since(first_tick), None);
    }
}
