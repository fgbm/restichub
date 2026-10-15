//! Limits the progress writes of one job to one for each second (FR-AG-08, NFR-PERF-04).
//!
//! Each progress message opens a database transaction. An honest agent sends one message every two
//! seconds, but a stream can send them as fast as the database answers. The throttle writes the
//! first message at once. It keeps only the latest message that arrives in the next second and
//! writes it when the second ends. The protocol does not change.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use restichub_db::JobId;

pub const INTERVAL: Duration = Duration::from_secs(1);

/// The throttle forgets expired jobs when it holds more than this many.
const PRUNE_AT: usize = 256;
/// A hard cap: a job above it is not throttled, so the memory stays bounded.
const MAX_JOBS: usize = 10_000;

#[derive(Debug)]
struct Slot<T> {
    window_end: Instant,
    pending: Option<T>,
    flush_scheduled: bool,
}

/// What the caller does with a progress message.
#[derive(Debug, PartialEq, Eq)]
pub enum Admit<T> {
    WriteNow(T),
    /// The throttle keeps the message. The caller waits for this long, then calls
    /// [`ProgressThrottle::take_pending`].
    FlushAfter(Duration),
    /// The throttle keeps the message, and a flush is already due.
    Held,
}

#[derive(Debug)]
pub struct ProgressThrottle<T> {
    slots: Mutex<HashMap<JobId, Slot<T>>>,
}

impl<T> Default for ProgressThrottle<T> {
    fn default() -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
        }
    }
}

impl<T> ProgressThrottle<T> {
    pub fn admit(&self, job: JobId, message: T, now: Instant) -> Admit<T> {
        let mut slots = lock(&self.slots);
        if slots.len() >= PRUNE_AT {
            slots.retain(|_, slot| slot.flush_scheduled || now < slot.window_end);
        }
        match slots.get_mut(&job) {
            Some(slot) if now < slot.window_end => {
                slot.pending = Some(message);
                if slot.flush_scheduled {
                    Admit::Held
                } else {
                    slot.flush_scheduled = true;
                    Admit::FlushAfter(slot.window_end.saturating_duration_since(now))
                }
            }
            Some(slot) => {
                slot.window_end = now + INTERVAL;
                slot.pending = None;
                Admit::WriteNow(message)
            }
            None => {
                if slots.len() < MAX_JOBS {
                    slots.insert(
                        job,
                        Slot {
                            window_end: now + INTERVAL,
                            pending: None,
                            flush_scheduled: false,
                        },
                    );
                }
                Admit::WriteNow(message)
            }
        }
    }

    /// The latest held message, and the start of the next window. `None` when nothing waits.
    pub fn take_pending(&self, job: JobId, now: Instant) -> Option<T> {
        let mut slots = lock(&self.slots);
        let slot = slots.get_mut(&job)?;
        slot.flush_scheduled = false;
        let message = slot.pending.take()?;
        slot.window_end = now + INTERVAL;
        Some(message)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> JobId {
        JobId::new()
    }

    #[test]
    fn fr_ag_08_the_first_progress_is_written_at_once() {
        let throttle = ProgressThrottle::default();
        assert_eq!(throttle.admit(job(), 1, Instant::now()), Admit::WriteNow(1));
    }

    #[test]
    fn fr_ag_08_progress_inside_one_second_keeps_only_the_latest() {
        let throttle = ProgressThrottle::default();
        let id = job();
        let start = Instant::now();
        assert_eq!(throttle.admit(id, 1, start), Admit::WriteNow(1));
        let second = start + Duration::from_millis(100);
        assert_eq!(
            throttle.admit(id, 2, second),
            Admit::FlushAfter(Duration::from_millis(900))
        );
        assert_eq!(
            throttle.admit(id, 3, start + Duration::from_millis(200)),
            Admit::Held
        );
        let flush = start + INTERVAL;
        assert_eq!(throttle.take_pending(id, flush), Some(3));
        assert_eq!(throttle.take_pending(id, flush), None);
    }

    #[test]
    fn fr_ag_08_progress_after_the_window_is_written_at_once() {
        let throttle = ProgressThrottle::default();
        let id = job();
        let start = Instant::now();
        assert_eq!(throttle.admit(id, 1, start), Admit::WriteNow(1));
        assert_eq!(
            throttle.admit(id, 2, start + INTERVAL + Duration::from_millis(1)),
            Admit::WriteNow(2)
        );
    }

    #[test]
    fn fr_ag_08_a_flush_opens_a_new_window() {
        let throttle = ProgressThrottle::default();
        let id = job();
        let start = Instant::now();
        throttle.admit(id, 1, start);
        throttle.admit(id, 2, start + Duration::from_millis(10));
        let flush = start + INTERVAL;
        assert_eq!(throttle.take_pending(id, flush), Some(2));
        assert_eq!(
            throttle.admit(id, 3, flush + Duration::from_millis(10)),
            Admit::FlushAfter(Duration::from_millis(990))
        );
    }

    #[test]
    fn fr_ag_08_jobs_do_not_share_a_window() {
        let throttle = ProgressThrottle::default();
        let now = Instant::now();
        assert_eq!(throttle.admit(job(), 1, now), Admit::WriteNow(1));
        assert_eq!(throttle.admit(job(), 1, now), Admit::WriteNow(1));
    }
}
