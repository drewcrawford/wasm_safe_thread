// SPDX-License-Identifier: MIT OR Apache-2.0
use super::Instant;
use super::thread;
use super::{ASYNC_WAITER_ID_COUNTER, AsyncWaiter, Condvar, WaitTimeoutResult};
use crate::Guard;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::task::{Context, Poll};

impl Condvar {
    /// Asynchronously waits for a notification from this condition variable.
    ///
    /// This method will atomically unlock the mutex specified by the guard and
    /// await a notification. When a notification is received, the mutex will be
    /// re-acquired before the future resolves.
    ///
    /// This method is non-blocking and works everywhere, including WASM main thread.
    ///
    /// # Examples
    ///
    /// ```
    /// # // std::thread::spawn panics on wasm32
    /// # if cfg!(target_arch = "wasm32") { return; }
    /// # test_executors::spin_on(async {
    /// use wasm_safe_thread::{Mutex, condvar::Condvar};
    /// use std::sync::Arc;
    /// # use std::thread;
    ///
    /// let pair = Arc::new((Mutex::new(false), Condvar::new()));
    /// let pair_clone = Arc::clone(&pair);
    ///
    /// // Spawn a thread that will notify us
    /// thread::spawn(move || {
    ///     # #[cfg(not(target_arch = "wasm32"))]
    ///     std::thread::sleep(std::time::Duration::from_millis(10));
    ///     test_executors::spin_on(async {
    ///         let (mutex, condvar) = &*pair_clone;
    ///         let mut ready = mutex.lock_async().await;
    ///         *ready = true;
    ///         drop(ready);
    ///         condvar.notify_one();
    ///     });
    /// });
    ///
    /// let (mutex, condvar) = &*pair;
    /// let mut ready = mutex.lock_async().await;
    /// while !*ready {
    ///     ready = condvar.wait_async(ready).await;
    /// }
    /// assert!(*ready);
    /// # });
    /// ```
    pub async fn wait_async<'a, T>(&self, guard: Guard<'a, T>) -> Guard<'a, T> {
        let mutex = guard.mutex;

        // Create a channel to receive the notification
        let receiver = self.waiting_async_threads.with_mut(|waiters| {
            let (sender, receiver) = r#continue::continuation();
            let id = ASYNC_WAITER_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
            waiters.push(AsyncWaiter { id, sender });
            receiver
        });

        // Release the mutex
        drop(guard);

        // Wait for notification
        receiver.await;

        // Re-acquire the mutex
        mutex.lock_async().await
    }

    /// Asynchronously waits while the predicate remains `true`.
    ///
    /// This method will atomically unlock the mutex specified by the guard and
    /// await a notification as long as the `condition` closure returns `true`.
    ///
    /// # Platform Behavior
    ///
    /// - **All Platforms**: Uses async/await to yield execution. This is non-blocking
    ///   and works in all environments, including WASM main thread.
    ///
    /// # Predicate
    ///
    /// The `condition` closure is called:
    /// 1. Before waiting (if it returns `false`, the method returns immediately)
    /// 2. After each notification (to check if we should keep waiting)
    /// 3. After spurious wakeups (to ensure we don't return prematurely)
    ///
    /// # Spurious Wakeups
    ///
    /// This method automatically handles spurious wakeups by re-checking the condition.
    /// You do not need to loop around this call.
    ///
    /// # Examples
    ///
    /// ```
    /// # // std::thread::spawn panics on wasm32
    /// # if cfg!(target_arch = "wasm32") { return; }
    /// # test_executors::spin_on(async {
    /// use wasm_safe_thread::{Mutex, condvar::Condvar};
    /// use std::sync::Arc;
    /// # use std::thread;
    ///
    /// let pair = Arc::new((Mutex::new(false), Condvar::new()));
    /// let pair_clone = Arc::clone(&pair);
    ///
    /// thread::spawn(move || {
    ///     # #[cfg(not(target_arch = "wasm32"))]
    ///     std::thread::sleep(std::time::Duration::from_millis(10));
    ///     test_executors::spin_on(async {
    ///         let (mutex, condvar) = &*pair_clone;
    ///         let mut ready = mutex.lock_async().await;
    ///         *ready = true;
    ///         drop(ready);
    ///         condvar.notify_one();
    ///     });
    /// });
    ///
    /// let (mutex, condvar) = &*pair;
    /// let mut ready = mutex.lock_async().await;
    /// // Wait until ready becomes true
    /// ready = condvar.wait_async_while(ready, |r| !*r).await;
    /// assert!(*ready);
    /// # });
    /// ```
    pub async fn wait_async_while<'a, T, F>(
        &self,
        mut guard: Guard<'a, T>,
        mut condition: F,
    ) -> Guard<'a, T>
    where
        F: FnMut(&mut T) -> bool,
    {
        while condition(&mut guard) {
            guard = self.wait_async(guard).await;
        }
        guard
    }

    /// Asynchronously waits for a notification from this condition variable with a deadline.
    ///
    /// This method will atomically unlock the mutex specified by the guard and
    /// await a notification or timeout. When a notification is received or the timeout expires,
    /// the mutex will be re-acquired before the future resolves.
    ///
    /// This method is non-blocking and works everywhere, including WASM main thread.
    ///
    /// # Examples
    ///
    /// ```
    /// # // std::thread::spawn panics on wasm32
    /// # if cfg!(target_arch = "wasm32") { return; }
    /// # test_executors::spin_on(async {
    /// use wasm_safe_thread::{Mutex, condvar::Condvar};
    /// use std::sync::Arc;
    /// # #[cfg(target_arch = "wasm32")]
    /// use web_time::{Duration, Instant};
    /// # #[cfg(not(target_arch = "wasm32"))]
    /// # use std::time::{Duration, Instant};
    /// # use std::thread;
    ///
    /// let pair = Arc::new((Mutex::new(false), Condvar::new()));
    /// let pair_clone = Arc::clone(&pair);
    ///
    /// // Spawn a thread that will notify us
    /// thread::spawn(move || {
    ///     # #[cfg(not(target_arch = "wasm32"))]
    ///     std::thread::sleep(std::time::Duration::from_millis(10));
    ///     test_executors::spin_on(async {
    ///         let (mutex, condvar) = &*pair_clone;
    ///         let mut ready = mutex.lock_async().await;
    ///         *ready = true;
    ///         drop(ready);
    ///         condvar.notify_one();
    ///     });
    /// });
    ///
    /// let (mutex, condvar) = &*pair;
    /// let mut ready = mutex.lock_async().await;
    /// let deadline = Instant::now() + Duration::from_secs(1);
    /// while !*ready {
    ///     let result;
    ///     (ready, result) = condvar.wait_async_timeout(ready, deadline).await;
    ///     if result.timed_out() {
    ///         break;
    ///     }
    /// }
    /// assert!(*ready);
    /// # });
    /// ```
    pub async fn wait_async_timeout<'a, T>(
        &self,
        guard: Guard<'a, T>,
        deadline: Instant,
    ) -> (Guard<'a, T>, WaitTimeoutResult) {
        let mutex = guard.mutex;

        // Create a unique ID for this waiter
        let waiter_id = ASYNC_WAITER_ID_COUNTER.fetch_add(1, Ordering::Relaxed);

        // Create two channels - one for normal notification, one for timeout
        let (notify_sender, notify_receiver) = r#continue::continuation();
        let (timeout_sender, timeout_receiver) = r#continue::continuation();

        // Add to waiting list
        self.waiting_async_threads.with_mut(|waiters| {
            waiters.push(AsyncWaiter {
                id: waiter_id,
                sender: notify_sender,
            });
        });

        // Spawn a thread to handle the timeout
        thread::spawn(move || {
            let now = Instant::now();
            if deadline > now {
                let duration = deadline - now;
                thread::sleep(duration);
            }
            // Send timeout signal
            timeout_sender.send(());
        });

        // Release the mutex
        drop(guard);

        // Race between notification and timeout
        // We'll poll both futures and see which completes first
        struct Race<F1, F2> {
            notify: Option<F1>,
            timeout: Option<F2>,
        }

        impl<F1: Future + Unpin, F2: Future + Unpin> Future for Race<F1, F2> {
            type Output = bool; // true if timed out

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                // Poll notification future
                if let Some(ref mut notify) = self.notify {
                    if Pin::new(notify).poll(cx).is_ready() {
                        self.notify = None;
                        return Poll::Ready(false); // Got notification
                    }
                }

                // Poll timeout future
                if let Some(ref mut timeout) = self.timeout {
                    if Pin::new(timeout).poll(cx).is_ready() {
                        self.timeout = None;
                        return Poll::Ready(true); // Timed out
                    }
                }

                Poll::Pending
            }
        }

        let timed_out = Race {
            notify: Some(notify_receiver),
            timeout: Some(timeout_receiver),
        }
        .await;

        // If we timed out, remove ourselves from the list
        if timed_out {
            self.waiting_async_threads.with_mut(|waiters| {
                if let Some(pos) = waiters.iter().position(|w| w.id == waiter_id) {
                    let waiter = waiters.remove(pos);
                    // Send the notification to complete the receiver
                    waiter.sender.send(());
                }
            });
        }

        // Re-acquire the mutex
        let guard = mutex.lock_async().await;

        // Return the result
        (guard, WaitTimeoutResult(timed_out))
    }

    /// Asynchronously waits while the predicate remains `true`, bounded by the deadline.
    ///
    /// This method will atomically unlock the mutex specified by the guard and
    /// await a notification as long as the `condition` closure returns `true`.
    ///
    /// # Platform Behavior
    ///
    /// - **All Platforms**: Uses async/await to yield execution. This is non-blocking
    ///   and works in all environments, including WASM main thread.
    ///
    /// # Predicate
    ///
    /// The `condition` closure is called:
    /// 1. Before waiting (if it returns `false`, the method returns immediately)
    /// 2. After each notification (to check if we should keep waiting)
    /// 3. After spurious wakeups (to ensure we don't return prematurely)
    ///
    /// # Spurious Wakeups
    ///
    /// This method automatically handles spurious wakeups by re-checking the condition.
    /// You do not need to loop around this call.
    ///
    /// # Examples
    ///
    /// ```
    /// # // std::thread::spawn panics on wasm32
    /// # if cfg!(target_arch = "wasm32") { return; }
    /// # test_executors::spin_on(async {
    /// use wasm_safe_thread::{Mutex, condvar::Condvar};
    /// use std::sync::Arc;
    /// # #[cfg(target_arch = "wasm32")]
    /// use web_time::{Duration, Instant};
    /// # #[cfg(not(target_arch = "wasm32"))]
    /// # use std::time::{Duration, Instant};
    /// # use std::thread;
    ///
    /// let pair = Arc::new((Mutex::new(0), Condvar::new()));
    /// let pair_clone = Arc::clone(&pair);
    ///
    /// thread::spawn(move || {
    ///     # #[cfg(not(target_arch = "wasm32"))]
    ///     std::thread::sleep(std::time::Duration::from_millis(10));
    ///     test_executors::spin_on(async {
    ///         let (mutex, condvar) = &*pair_clone;
    ///         let mut value = mutex.lock_async().await;
    ///         *value = 10;
    ///         drop(value);
    ///         condvar.notify_one();
    ///     });
    /// });
    ///
    /// let (mutex, condvar) = &*pair;
    /// let guard = mutex.lock_async().await;
    /// let deadline = Instant::now() + Duration::from_secs(1);
    /// let (guard, result) = condvar.wait_async_timeout_while(guard, deadline, |v| *v < 10).await;
    /// if !result.timed_out() {
    ///     assert_eq!(*guard, 10);
    /// }
    /// # });
    /// ```
    pub async fn wait_async_timeout_while<'a, T, F>(
        &self,
        mut guard: Guard<'a, T>,
        deadline: Instant,
        mut condition: F,
    ) -> (Guard<'a, T>, WaitTimeoutResult)
    where
        F: FnMut(&mut T) -> bool,
    {
        while condition(&mut guard) {
            let result;
            (guard, result) = self.wait_async_timeout(guard, deadline).await;
            if result.timed_out() {
                return (guard, result);
            }
        }
        (guard, WaitTimeoutResult(false))
    }
}
