use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};

/// A sliding rate limiter that allows a maximum number of tasks to be executed concurrently in a given window.
#[derive(Clone)]
pub struct SlidingRateLimiter {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    /// The window of time in which the maximum number of tasks can be executed concurrently.
    window: Duration,
    /// The maximum number of tasks that can be executed concurrently in a given window.
    max_tasks: usize,

    /// The timestamps of the tasks that have been executed.
    timestamps: VecDeque<Instant>,
    /// The waiters for the tasks.
    waiters: VecDeque<Arc<tokio::sync::Notify>>,
}

struct WaiterGuard {
    inner: Arc<Mutex<Inner>>,
    notify: Arc<tokio::sync::Notify>,
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        // Spawn a task to clean up since Drop can't be async
        let inner = self.inner.clone();
        let notify = self.notify.clone();
        tokio::spawn(async move {
            let mut inner = inner.lock().await;
            inner.waiters.retain(|w| !Arc::ptr_eq(w, &notify));
            // Wake next waiter in case we were at the front
            if let Some(next) = inner.waiters.front() {
                next.notify_one();
            }
        });
    }
}

impl SlidingRateLimiter {
    pub fn new(max_tasks: usize, window: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                window,
                max_tasks,
                timestamps: VecDeque::new(),
                waiters: VecDeque::new(),
            })),
        }
    }

    pub async fn acquire(self) {
        let notify = {
            let mut inner = self.inner.lock().await;
            let notify = Arc::new(tokio::sync::Notify::new());
            inner.waiters.push_back(notify.clone());
            notify
        };

        // Guard ensures we're removed from the waiter queue if this future is dropped/cancelled
        let _guard = WaiterGuard {
            inner: self.inner.clone(),
            notify: notify.clone(),
        };

        loop {
            // Create the Notified future BEFORE dropping the lock,
            // so we can't miss a notify_one() that fires after drop(inner)
            // but before we call .await
            let notified = notify.notified();

            let wait = {
                let mut inner = self.inner.lock().await;
                let now = Instant::now();

                while inner
                    .timestamps
                    .front()
                    .is_some_and(|t| now.duration_since(*t) >= inner.window)
                {
                    inner.timestamps.pop_front();
                }

                let is_next = inner
                    .waiters
                    .front()
                    .is_some_and(|w| Arc::ptr_eq(w, &notify));

                if is_next && inner.timestamps.len() < inner.max_tasks {
                    inner.timestamps.push_back(now);
                    inner.waiters.pop_front();

                    if let Some(next) = inner.waiters.front() {
                        next.notify_one();
                    }

                    return;
                }

                inner
                    .timestamps
                    .front()
                    .map(|t| inner.window.saturating_sub(now.duration_since(*t)))
                    .unwrap_or(Duration::ZERO)
            }; // lock is dropped here, before any awaits

            // Race the sleep and the notification against each other
            // so we wake up as soon as either condition is met
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = notified => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{self, Duration};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn test_sliding_rate_limiter_time_control_and_thread_count() {
        let limiter = SlidingRateLimiter::new(3, Duration::from_secs(10));
        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = vec![];

        // Track thread IDs (or just number of threads/tasks)
        let thread_counter = Arc::new(AtomicUsize::new(0));

        for _ in 0..6 {
            let limiter = limiter.clone();
            let counter = Arc::clone(&counter);
            let thread_counter_inner = thread_counter.clone();
            handles.push(tokio::spawn(async move {
                thread_counter_inner.fetch_add(1, Ordering::SeqCst);
                limiter.acquire().await;
                counter.fetch_add(1, Ordering::SeqCst);
            }));
        }

        // Initially, the first 3 tasks should go through (since limiter=3)
        for _ in 0..3 {
            time::advance(Duration::from_millis(1)).await;
        }

        time::sleep(Duration::from_millis(10)).await; // Yield
        assert_eq!(counter.load(Ordering::SeqCst), 3);

        // Advance just short of the window, nothing else should proceed
        time::advance(Duration::from_secs(9)).await;
        time::sleep(Duration::from_millis(10)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 3);

        // Now advance to end of window, 3 more should go through
        time::advance(Duration::from_secs(1)).await;
        time::sleep(Duration::from_millis(10)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 6);

        // All tasks done, check task/thread count (should be 6 tasks launched)
        assert_eq!(thread_counter.load(Ordering::SeqCst), 6);

        for handle in handles {
            handle.await.unwrap();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_sliding_rate_limiter_ensures_order() {
        let limiter = SlidingRateLimiter::new(2, Duration::from_secs(5));
        let output = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let mut handles = vec![];

        for i in 0..5 {
            let limiter = limiter.clone();
            let output = output.clone();
            handles.push(tokio::spawn(async move {
                limiter.acquire().await;
                let mut lock = output.lock().await;
                lock.push(i);
            }));
        }

        // Yield to let all tasks start and register in the waiter queue
        // before we start advancing time
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        // Use advance instead of sleep to actually move paused time forward,
        // then yield to let newly unblocked tasks run
        time::advance(Duration::from_millis(1)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        // Scope the lock so it's dropped before any time advances or yields
        {
            let lock = output.lock().await;
            assert_eq!(lock.len(), 2, "Expected 2 tasks after first window start");
            assert_eq!(lock[0], 0);
            assert_eq!(lock[1], 1);
        } // lock dropped here

        // Advance just short of the window — no new acquires expected
        time::advance(Duration::from_secs(4)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        {
            let lock = output.lock().await;
            assert_eq!(
                lock.len(),
                2,
                "Expected still 2 tasks before window expires"
            );
        }

        // Advance past the first window (t=5s) — tasks 2 and 3 can now acquire
        time::advance(Duration::from_secs(1)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        {
            let lock = output.lock().await;
            assert_eq!(lock.len(), 4, "Expected 4 tasks after second window opens");
            assert_eq!(lock[2], 2);
            assert_eq!(lock[3], 3);
        }

        // Task 4 was acquired at t~=5s, so it expires at t=10s
        // We're at t=5s, so we need another full 5s for task 4's slot to open
        time::advance(Duration::from_secs(5)).await;
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        {
            let lock = output.lock().await;
            assert_eq!(lock.len(), 5, "Expected all 5 tasks complete");
            assert_eq!(lock[4], 4);

            // Ensure output order is monotonically increasing
            let ordered = lock.windows(2).all(|w| w[0] < w[1]);
            assert!(ordered, "Output is not in order: {:?}", *lock);
        }

        for handle in handles {
            handle.await.unwrap();
        }
    }
}
