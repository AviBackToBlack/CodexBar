//! Single-flight data operations for `serve` `/usage` and `/cost` (upstream
//! `CLIServeOperationCoordinator`, simplified).
//!
//! Each key runs at most one operation at a time. An operation runs as its
//! own task, so a request that gives up at its deadline never cancels it: the
//! work keeps running, and a later request for the same key joins it instead
//! of stacking a second fetch. Finished values are not cached; the first
//! request after an operation completes starts a fresh one. Upstream also
//! queues a successor whose config fingerprint changed behind the running
//! operation; here every successor joins it, and the next operation reads the
//! new settings.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use futures::future::{BoxFuture, FutureExt, Shared};
use tokio::sync::Semaphore;
use tokio::time::Instant;

/// Operations running at once across every key. Matches the desktop shell's
/// provider refresh bound.
const MAX_RUNNING_OPERATIONS: usize = 8;

/// Why a request got no value from its operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OperationMiss {
    /// The request deadline passed first; the operation keeps running.
    TimedOut,
    /// The operation panicked.
    Failed,
}

type SharedOutcome<T> = Shared<BoxFuture<'static, Option<T>>>;

struct Running<T: Clone> {
    generation: u64,
    outcome: SharedOutcome<T>,
}

struct Slots<T: Clone> {
    next_generation: u64,
    running: HashMap<String, Running<T>>,
}

/// Per-key single-flight coordinator shared by every request of one `serve`
/// process.
pub(super) struct ServeOperations<T: Clone> {
    slots: Arc<Mutex<Slots<T>>>,
    permits: Arc<Semaphore>,
}

impl<T: Clone> Clone for ServeOperations<T> {
    fn clone(&self) -> Self {
        Self {
            slots: Arc::clone(&self.slots),
            permits: Arc::clone(&self.permits),
        }
    }
}

impl<T: Clone> fmt::Debug for ServeOperations<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServeOperations").finish_non_exhaustive()
    }
}

impl<T: Clone> Default for ServeOperations<T> {
    fn default() -> Self {
        Self {
            slots: Arc::new(Mutex::new(Slots {
                next_generation: 0,
                running: HashMap::new(),
            })),
            permits: Arc::new(Semaphore::new(MAX_RUNNING_OPERATIONS)),
        }
    }
}

impl<T> ServeOperations<T>
where
    T: Clone + Send + Sync + 'static,
{
    /// The value for `key`: join its running operation, or start `operation`
    /// as a detached task. A deadline that has already passed returns
    /// [`OperationMiss::TimedOut`] without starting any work.
    pub(super) async fn value<F, Fut>(
        &self,
        key: &str,
        deadline: Option<Instant>,
        operation: F,
    ) -> Result<T, OperationMiss>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T> + Send + 'static,
    {
        if deadline.is_some_and(|deadline| deadline <= Instant::now()) {
            return Err(OperationMiss::TimedOut);
        }
        let outcome = self.join_or_start(key, operation);
        let value = match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, outcome)
                .await
                .map_err(|_| OperationMiss::TimedOut)?,
            None => outcome.await,
        };
        value.ok_or(OperationMiss::Failed)
    }

    fn join_or_start<F, Fut>(&self, key: &str, operation: F) -> SharedOutcome<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T> + Send + 'static,
    {
        // The lock is held until the new operation is registered, so its
        // finish guard (which takes the same lock) can never run first and
        // leave a finished entry behind.
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(running) = slots.running.get(key) {
            return running.outcome.clone();
        }
        let generation = slots.next_generation;
        slots.next_generation = generation.wrapping_add(1);
        let finish = FinishGuard {
            slots: Arc::clone(&self.slots),
            key: key.to_string(),
            generation,
        };
        let permits = Arc::clone(&self.permits);
        let work = operation();
        let task = tokio::spawn(async move {
            // Dropped when the work ends or unwinds, freeing the key.
            let _finish = finish;
            // The semaphore is never closed, so a permit always arrives.
            let _permit = permits.acquire_owned().await.ok();
            work.await
        });
        let outcome = async move { task.await.ok() }.boxed().shared();
        slots.running.insert(
            key.to_string(),
            Running {
                generation,
                outcome: outcome.clone(),
            },
        );
        outcome
    }

    #[cfg(test)]
    pub(super) fn is_running(&self, key: &str) -> bool {
        self.slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .running
            .contains_key(key)
    }
}

struct FinishGuard<T: Clone> {
    slots: Arc<Mutex<Slots<T>>>,
    key: String,
    generation: u64,
}

impl<T: Clone> Drop for FinishGuard<T> {
    fn drop(&mut self) {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        if slots
            .running
            .get(&self.key)
            .is_some_and(|running| running.generation == self.generation)
        {
            slots.running.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OperationMiss, ServeOperations};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::oneshot;
    use tokio::time::Instant;

    fn soon() -> Option<Instant> {
        Some(Instant::now() + Duration::from_millis(50))
    }

    async fn wait_until_idle(operations: &ServeOperations<u32>, key: &str) {
        for _ in 0..200 {
            if !operations.is_running(key) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("operation {key} never finished");
    }

    #[tokio::test(start_paused = true)]
    async fn a_timed_out_operation_keeps_running_and_a_successor_joins_it() {
        let operations = ServeOperations::<u32>::default();
        let starts = Arc::new(AtomicUsize::new(0));
        let (release, released) = oneshot::channel::<u32>();

        let counter = Arc::clone(&starts);
        let first = operations
            .value("usage:claude", soon(), move || async move {
                counter.fetch_add(1, Ordering::SeqCst);
                released.await.unwrap_or(0)
            })
            .await;
        assert_eq!(first, Err(OperationMiss::TimedOut));
        assert!(
            operations.is_running("usage:claude"),
            "timed-out work is not cancelled"
        );

        let counter = Arc::clone(&starts);
        let joiner = operations.clone();
        let second = tokio::spawn(async move {
            joiner
                .value("usage:claude", None, move || async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    99
                })
                .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        release.send(7).unwrap();
        assert_eq!(
            second.await.unwrap(),
            Ok(7),
            "the successor gets the late value"
        );
        assert_eq!(
            starts.load(Ordering::SeqCst),
            1,
            "no second fetch was stacked"
        );
        wait_until_idle(&operations, "usage:claude").await;
    }

    #[tokio::test(start_paused = true)]
    async fn finished_values_are_not_cached() {
        let operations = ServeOperations::<u32>::default();
        let starts = Arc::new(AtomicUsize::new(0));
        for expected in 1..=2 {
            let counter = Arc::clone(&starts);
            let value = operations
                .value("cost:codex", None, move || async move {
                    u32::try_from(counter.fetch_add(1, Ordering::SeqCst) + 1).unwrap_or(0)
                })
                .await;
            assert_eq!(value, Ok(expected));
            wait_until_idle(&operations, "cost:codex").await;
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_expired_deadline_starts_no_work() {
        let operations = ServeOperations::<u32>::default();
        let starts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&starts);
        let value = operations
            .value(
                "usage:codex",
                Some(Instant::now() - Duration::from_millis(1)),
                move || async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    1
                },
            )
            .await;
        assert_eq!(value, Err(OperationMiss::TimedOut));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        assert!(!operations.is_running("usage:codex"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_panicking_operation_frees_its_key() {
        let operations = ServeOperations::<u32>::default();
        let failed = operations
            .value("usage:codex", None, || async {
                panic!("provider bug");
            })
            .await;
        assert_eq!(failed, Err(OperationMiss::Failed));
        wait_until_idle(&operations, "usage:codex").await;
        let value = operations.value("usage:codex", None, || async { 3 }).await;
        assert_eq!(value, Ok(3));
    }

    #[tokio::test(start_paused = true)]
    async fn different_keys_run_independently() {
        let operations = ServeOperations::<u32>::default();
        let (_hold, held) = oneshot::channel::<u32>();
        let slow = operations
            .value("usage:claude", soon(), move || async move {
                held.await.unwrap_or(0)
            })
            .await;
        assert_eq!(slow, Err(OperationMiss::TimedOut));
        let fast = operations
            .value("usage:codex", soon(), || async { 5 })
            .await;
        assert_eq!(fast, Ok(5));
    }
}
