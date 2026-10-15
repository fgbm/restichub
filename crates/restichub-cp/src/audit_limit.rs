//! The limit on audit records that one agent can cause by refused messages (AC-PR-01, AC-PR-05).
//!
//! The audit log only grows, so a compromised host must not fill it with refusals. Per agent and
//! reason, the first records of each window are written. The rest only count, and one summary
//! record holds the count when the window ends or the stream closes. The limiter is per process.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use restichub_db::AgentId;

pub const DEFAULT_LIMIT: u32 = 10;
pub const WINDOW: Duration = Duration::from_secs(60);

/// The kind of refusal that the limit counts. The reason is a stable code, never agent input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subject {
    Rejection(&'static str),
    Refusal(&'static str),
}

/// What the caller does with a refusal.
#[derive(Debug, PartialEq, Eq)]
pub enum Admission {
    /// Write the record. `summary` is the count that the previous window suppressed.
    Write {
        summary: Option<u64>,
    },
    Skip,
}

#[derive(Debug)]
struct Window {
    start: Instant,
    written: u32,
    suppressed: u64,
}

#[derive(Debug)]
pub struct AuditLimiter {
    limit: u32,
    window: Duration,
    windows: Mutex<HashMap<(AgentId, Subject), Window>>,
}

impl AuditLimiter {
    pub fn new(limit: u32) -> Self {
        Self::with_window(limit, WINDOW)
    }

    pub fn with_window(limit: u32, window: Duration) -> Self {
        Self {
            limit: limit.max(1),
            window,
            windows: Mutex::new(HashMap::new()),
        }
    }

    pub fn admit(&self, agent: AgentId, subject: Subject, now: Instant) -> Admission {
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let entry = windows.entry((agent, subject)).or_insert(Window {
            start: now,
            written: 0,
            suppressed: 0,
        });
        if now.saturating_duration_since(entry.start) >= self.window {
            let summary = Some(entry.suppressed).filter(|count| *count > 0);
            *entry = Window {
                start: now,
                written: 1,
                suppressed: 0,
            };
            return Admission::Write { summary };
        }
        if entry.written < self.limit {
            entry.written += 1;
            return Admission::Write { summary: None };
        }
        entry.suppressed += 1;
        Admission::Skip
    }

    /// Removes the windows of the agent and returns the counts that no record holds yet.
    pub fn close(&self, agent: AgentId) -> Vec<(Subject, u64)> {
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let mut pending = Vec::new();
        windows.retain(|(owner, subject), window| {
            if *owner != agent {
                return true;
            }
            if window.suppressed > 0 {
                pending.push((*subject, window.suppressed));
            }
            false
        });
        pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUBJECT: Subject = Subject::Rejection("bad_signature");

    #[test]
    fn the_first_records_of_a_window_pass_and_the_rest_count() {
        let limiter = AuditLimiter::with_window(2, Duration::from_secs(60));
        let agent = AgentId::new();
        let start = Instant::now();
        assert_eq!(
            limiter.admit(agent, SUBJECT, start),
            Admission::Write { summary: None }
        );
        assert_eq!(
            limiter.admit(agent, SUBJECT, start),
            Admission::Write { summary: None }
        );
        assert_eq!(limiter.admit(agent, SUBJECT, start), Admission::Skip);
        assert_eq!(limiter.admit(agent, SUBJECT, start), Admission::Skip);
        assert_eq!(limiter.close(agent), vec![(SUBJECT, 2)]);
    }

    #[test]
    fn a_new_window_reports_the_suppressed_count_once() {
        let limiter = AuditLimiter::with_window(1, Duration::from_secs(60));
        let agent = AgentId::new();
        let start = Instant::now();
        limiter.admit(agent, SUBJECT, start);
        limiter.admit(agent, SUBJECT, start);
        let later = start + Duration::from_secs(61);
        assert_eq!(
            limiter.admit(agent, SUBJECT, later),
            Admission::Write { summary: Some(1) }
        );
        assert!(limiter.close(agent).is_empty());
    }

    #[test]
    fn the_windows_of_other_reasons_and_agents_are_separate() {
        let limiter = AuditLimiter::with_window(1, Duration::from_secs(60));
        let (first, second) = (AgentId::new(), AgentId::new());
        let start = Instant::now();
        limiter.admit(first, SUBJECT, start);
        assert_eq!(limiter.admit(first, SUBJECT, start), Admission::Skip);
        assert_eq!(
            limiter.admit(first, Subject::Refusal("foreign_job"), start),
            Admission::Write { summary: None }
        );
        assert_eq!(
            limiter.admit(second, SUBJECT, start),
            Admission::Write { summary: None }
        );
    }
}
