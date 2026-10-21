//! Process counters of the agent listener for `/metrics` and the operator log.

use std::io;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Repeated refusals, and repeated accept errors, write at most one log line in this interval.
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
}

#[derive(Debug, Default)]
pub struct ListenerStats {
    refused: [AtomicU64; RefusalReason::ALL.len()],
    accept_errors: AtomicU64,
    refusal_log: LogThrottle,
    accept_error_log: LogThrottle,
}

impl ListenerStats {
    pub fn record_refusal(
        &self,
        reason: RefusalReason,
        connections: usize,
        max_connections: usize,
    ) {
        self.refused[reason as usize].fetch_add(1, Ordering::Relaxed);
        if let Some(refused) = self.refusal_log.record(Instant::now()) {
            tracing::warn!(
                reason = reason.as_str(),
                refused,
                connections,
                max_connections,
                "the agent listener closes connections at its limits; refused is the count since the last line"
            );
        }
    }

    pub fn record_accept_error(&self, error: &io::Error) {
        self.accept_errors.fetch_add(1, Ordering::Relaxed);
        if let Some(errors) = self.accept_error_log.record(Instant::now()) {
            tracing::warn!(
                %error,
                errors,
                "the agent listener cannot accept connections; errors is the count since the last line"
            );
        }
    }

    pub fn refused(&self, reason: RefusalReason) -> u64 {
        self.refused[reason as usize].load(Ordering::Relaxed)
    }

    pub fn accept_errors(&self) -> u64 {
        self.accept_errors.load(Ordering::Relaxed)
    }
}

/// Counts events and lets one log line through per [`LOG_INTERVAL`].
#[derive(Debug, Default)]
struct LogThrottle {
    state: Mutex<ThrottleState>,
}

#[derive(Debug, Default)]
struct ThrottleState {
    last_line: Option<Instant>,
    pending: u64,
}

impl LogThrottle {
    /// Returns the events since the last line when a line is due.
    fn record(&self, now: Instant) -> Option<u64> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.pending += 1;
        if state
            .last_line
            .is_some_and(|last| now.saturating_duration_since(last) < LOG_INTERVAL)
        {
            return None;
        }
        state.last_line = Some(now);
        Some(std::mem::take(&mut state.pending))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_event_logs_and_the_next_ones_wait_for_the_interval() {
        let throttle = LogThrottle::default();
        let start = Instant::now();
        assert_eq!(throttle.record(start), Some(1));
        assert_eq!(throttle.record(start + Duration::from_secs(1)), None);
        assert_eq!(throttle.record(start + Duration::from_secs(2)), None);
        assert_eq!(throttle.record(start + LOG_INTERVAL), Some(3));
        assert_eq!(throttle.record(start + LOG_INTERVAL), None);
    }

    #[test]
    fn each_reason_has_its_own_counter() {
        let stats = ListenerStats::default();
        stats.record_refusal(RefusalReason::Cap, 2, 2);
        stats.record_refusal(RefusalReason::Cap, 2, 2);
        stats.record_refusal(RefusalReason::HandshakesPerIp, 0, 2);
        assert_eq!(stats.refused(RefusalReason::Cap), 2);
        assert_eq!(stats.refused(RefusalReason::HandshakesPerIp), 1);
        assert_eq!(stats.refused(RefusalReason::AnonymousShare), 0);
    }
}
