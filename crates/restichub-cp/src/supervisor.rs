//! Supervision of the long-running background tasks (NFR-OPS-02).
//!
//! A panic in a pass would end a bare `tokio::spawn` loop for the life of the process while the
//! readiness probe stays green. The supervisor restarts the task after a pause, logs every stop
//! with the task name, and reports the task as failing to `/readyz` until the restarted task has run
//! for a stable period. The restart stays in the process: a restart of the whole process would also
//! drop the web server and every agent stream for one broken loop.

use std::any::Any;
use std::collections::BTreeSet;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::Interval;

const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
const STABLE_AFTER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    /// The pause before the first restart. It doubles for each failure in a row.
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    /// A restarted task counts as healthy after it has run for this long without a stop.
    pub stable_after: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            initial_backoff: INITIAL_BACKOFF,
            max_backoff: MAX_BACKOFF,
            stable_after: STABLE_AFTER,
        }
    }
}

/// Tells a task whether the supervisor starts it for the first time or after a stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    First,
    Restart,
}

impl Start {
    /// Consumes the tick of a new `interval` that fires at once, at the first start only.
    ///
    /// A restarted loop runs its pass at once. A pass that always breaks then stops the loop again
    /// within the pause, so the loop stays failing. A loop that waits a whole interval for its first
    /// pass would count as healthy after the stable period without a pass (NFR-OPS-02).
    pub async fn skip_first_tick(self, ticker: &mut Interval) {
        if self == Self::First {
            ticker.tick().await;
        }
    }
}

enum Exit {
    Stopped,
    Failed(String),
}

#[derive(Debug)]
struct Inner {
    policy: Policy,
    failing: Mutex<BTreeSet<&'static str>>,
}

impl Inner {
    fn set_failing(&self, name: &'static str, failing: bool) {
        let mut tasks = self.failing.lock().unwrap_or_else(PoisonError::into_inner);
        if failing {
            tasks.insert(name);
        } else {
            tasks.remove(name);
        }
    }
}

/// Starts the background tasks and answers which of them are failing.
#[derive(Debug, Clone)]
pub struct Supervisor {
    inner: Arc<Inner>,
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::with_policy(Policy::default())
    }
}

impl Supervisor {
    pub fn with_policy(policy: Policy) -> Self {
        Self {
            inner: Arc::new(Inner {
                policy,
                failing: Mutex::new(BTreeSet::new()),
            }),
        }
    }

    /// Runs a task that never returns. A return counts as a failure.
    ///
    /// The returned handle stops the supervisor and its task when it is aborted.
    pub fn spawn_loop<F, Fut>(&self, name: &'static str, mut make: F) -> JoinHandle<()>
    where
        F: FnMut(Start) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.supervise(name, move |start| {
            let task = make(start);
            async move {
                task.await;
                Exit::Failed("the task returned".to_owned())
            }
        })
    }

    /// Runs a service that ends with `Ok` when it stops on request. `Err` counts as a failure.
    pub fn spawn_service<F, Fut, E>(&self, name: &'static str, mut make: F) -> JoinHandle<()>
    where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), E>> + Send + 'static,
        E: std::error::Error,
    {
        self.supervise(name, move |_| {
            let task = make();
            async move {
                match task.await {
                    Ok(()) => Exit::Stopped,
                    Err(error) => Exit::Failed(error_chain(&error)),
                }
            }
        })
    }

    /// The names of the tasks that stopped and have not yet run for a stable period.
    pub fn failing(&self) -> Vec<&'static str> {
        let tasks = self
            .inner
            .failing
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        tasks.iter().copied().collect()
    }

    fn supervise<F, Fut>(&self, name: &'static str, make: F) -> JoinHandle<()>
    where
        F: FnMut(Start) -> Fut + Send + 'static,
        Fut: Future<Output = Exit> + Send + 'static,
    {
        tokio::spawn(run(Arc::clone(&self.inner), name, make))
    }
}

/// Aborts the task when the supervisor is dropped, so aborting the supervisor stops its task too.
struct AbortOnDrop(JoinHandle<Exit>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn run<F, Fut>(inner: Arc<Inner>, name: &'static str, mut make: F)
where
    F: FnMut(Start) -> Fut,
    Fut: Future<Output = Exit> + Send + 'static,
{
    let policy = inner.policy;
    let mut backoff = policy.initial_backoff;
    let mut start = Start::First;
    loop {
        let mut task = AbortOnDrop(tokio::spawn(make(start)));
        let joined = match tokio::time::timeout(policy.stable_after, &mut task.0).await {
            Ok(joined) => joined,
            Err(_) => {
                inner.set_failing(name, false);
                backoff = policy.initial_backoff;
                (&mut task.0).await
            }
        };
        let reason = match joined {
            Ok(Exit::Stopped) => return,
            Ok(Exit::Failed(reason)) => reason,
            Err(error) if error.is_panic() => {
                format!("panicked: {}", panic_message(error.into_panic().as_ref()))
            }
            Err(_) => return,
        };
        inner.set_failing(name, true);
        tracing::error!(task = name, %reason, restart_in = ?backoff, "a background task stopped");
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(policy.max_backoff);
        start = Start::Restart;
    }
}

fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// The text of a panic payload. The payload of `panic!` is a `&str` or a `String`.
pub fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic with no text")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    const POLICY: Policy = Policy {
        initial_backoff: Duration::from_secs(1),
        max_backoff: Duration::from_secs(4),
        stable_after: Duration::from_secs(10),
    };

    fn counter() -> Arc<AtomicUsize> {
        Arc::new(AtomicUsize::new(0))
    }

    #[tokio::test(start_paused = true)]
    async fn ac_nfr_11_a_panicking_loop_is_restarted_and_reported_until_it_is_stable() {
        let supervisor = Supervisor::with_policy(POLICY);
        let starts = counter();
        let seen = Arc::clone(&starts);
        supervisor.spawn_loop("test_loop", move |_| {
            let run = seen.fetch_add(1, Ordering::SeqCst);
            async move {
                assert!(run > 0, "the first pass breaks");
                std::future::pending::<()>().await;
            }
        });

        assert!(supervisor.failing().is_empty());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(supervisor.failing(), ["test_loop"]);
        assert_eq!(starts.load(Ordering::SeqCst), 1);

        tokio::time::sleep(POLICY.initial_backoff).await;
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        assert_eq!(supervisor.failing(), ["test_loop"]);

        tokio::time::sleep(POLICY.stable_after).await;
        assert!(supervisor.failing().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn ac_nfr_11_a_loop_that_returns_counts_as_failed() {
        let supervisor = Supervisor::with_policy(POLICY);
        let starts = counter();
        let seen = Arc::clone(&starts);
        supervisor.spawn_loop("test_loop", move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            async {}
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(supervisor.failing(), ["test_loop"]);
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(starts.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test(start_paused = true)]
    async fn ac_nfr_11_the_pause_doubles_up_to_the_maximum() {
        let supervisor = Supervisor::with_policy(POLICY);
        let starts = counter();
        let seen = Arc::clone(&starts);
        supervisor.spawn_loop("test_loop", move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            async { panic!("always") }
        });

        // The starts are at 0 s, 1 s, 3 s, 7 s and 11 s: the pauses are 1 s, 2 s, 4 s and 4 s.
        tokio::time::sleep(Duration::from_millis(6_500)).await;
        assert_eq!(starts.load(Ordering::SeqCst), 3);
        tokio::time::sleep(Duration::from_millis(5_000)).await;
        assert_eq!(starts.load(Ordering::SeqCst), 5);
    }

    #[tokio::test(start_paused = true)]
    async fn a_restarted_hourly_loop_that_breaks_every_pass_stays_failing() {
        let supervisor = Supervisor::with_policy(POLICY);
        supervisor.spawn_loop("hourly_loop", |start| async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(3600));
            start.skip_first_tick(&mut ticker).await;
            ticker.tick().await;
            panic!("every pass breaks");
        });

        tokio::time::sleep(Duration::from_secs(3601)).await;
        for second in 0..2 * 3600 {
            assert_eq!(supervisor.failing(), ["hourly_loop"], "second {second}");
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn only_the_first_start_skips_the_first_tick() {
        let supervisor = Supervisor::with_policy(POLICY);
        let starts = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&starts);
        supervisor.spawn_loop("test_loop", move |start| {
            seen.lock().expect("lock").push(start);
            async { panic!("stops") }
        });

        tokio::time::sleep(Duration::from_secs(4)).await;
        let starts = starts.lock().expect("lock");
        assert_eq!(starts[0], Start::First);
        assert!(starts.len() > 1);
        assert!(starts[1..].iter().all(|start| *start == Start::Restart));
    }

    #[tokio::test(start_paused = true)]
    async fn a_service_that_stops_cleanly_is_not_restarted() {
        let supervisor = Supervisor::with_policy(POLICY);
        let starts = counter();
        let seen = Arc::clone(&starts);
        let handle = supervisor.spawn_service("test_service", move || {
            seen.fetch_add(1, Ordering::SeqCst);
            async { Ok::<(), std::io::Error>(()) }
        });

        handle.await.expect("the supervisor ends");
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert!(supervisor.failing().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn ac_nfr_11_a_service_error_is_restarted() {
        let supervisor = Supervisor::with_policy(POLICY);
        let starts = counter();
        let seen = Arc::clone(&starts);
        supervisor.spawn_service("test_service", move || {
            let run = seen.fetch_add(1, Ordering::SeqCst);
            async move {
                if run == 0 {
                    Err(std::io::Error::other("cannot bind"))
                } else {
                    std::future::pending().await
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(supervisor.failing(), ["test_service"]);
        tokio::time::sleep(POLICY.initial_backoff).await;
        assert_eq!(starts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn aborting_the_supervisor_stops_its_task() {
        let supervisor = Supervisor::with_policy(POLICY);
        let alive = Arc::new(());
        let held = Arc::clone(&alive);
        let handle = supervisor.spawn_loop("test_loop", move |_| {
            let held = Arc::clone(&held);
            async move {
                let _held = held;
                std::future::pending::<()>().await;
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(Arc::strong_count(&alive), 3);

        handle.abort();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(Arc::strong_count(&alive), 1);
    }
}
