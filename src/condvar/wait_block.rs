// SPDX-License-Identifier: MIT OR Apache-2.0
use super::Instant;
use super::thread;
use super::{Condvar, WaitTimeoutResult};
use crate::Guard;

impl Condvar {
    /// Blocks the current thread while waiting for a notification.
    ///
    /// This method will atomically unlock the mutex specified by the guard and block
    /// the current thread. When a notification is received, the thread will wake up
    /// and re-acquire the lock before returning.
    ///
    /// # Platform Behavior
    ///
    /// - **Native**: Uses thread parking for efficient blocking
    /// - **WASM with `Atomics.wait`**: Blocks using `Atomics.wait`
    /// - **WASM without `Atomics.wait`**: **Will panic** - use [`wait_sync`](Self::wait_sync) instead
    ///
    /// This is a low-level primitive that unconditionally blocks. For adaptive
    /// behavior that works everywhere (including browser main threads where
    /// `Atomics.wait` is unavailable), use [`wait_sync`](Self::wait_sync).
    ///
    /// # Spurious Wakeups
    ///
    /// This method may return spuriously (without a notification). Always use it
    /// in a loop that checks the condition.
    ///
    /// # Examples
    ///
    /// ```
    /// # // std::thread::spawn panics on wasm32
    /// # if cfg!(target_arch = "wasm32") { return; }
    /// use wasm_safe_thread::{Mutex, condvar::Condvar};
    /// use std::sync::Arc;
    /// # use std::thread;
    ///
    /// let pair = Arc::new((Mutex::new(false), Condvar::new()));
    /// let pair_clone = Arc::clone(&pair);
    ///
    /// thread::spawn(move || {
    ///     let (mutex, condvar) = &*pair_clone;
    ///     let mut ready = mutex.lock_sync();
    ///     *ready = true;
    ///     drop(ready);
    ///     condvar.notify_one();
    /// });
    ///
    /// let (mutex, condvar) = &*pair;
    /// let mut ready = mutex.lock_sync();
    /// while !*ready {
    ///     ready = condvar.wait_block(ready);
    /// }
    /// assert!(*ready);
    /// ```
    pub fn wait_block<'a, T>(&self, guard: Guard<'a, T>) -> Guard<'a, T> {
        let mutex = guard.mutex;

        // Register this thread as waiting before releasing the lock
        self.waiting_sync_threads.with_mut(|threads| {
            threads.push(thread::current());
        });

        // Explicitly drop the guard to release the mutex
        drop(guard);

        // Park this thread while waiting to be notified
        thread::park();

        // Re-acquire the mutex before returning
        mutex.lock_sync()
    }

    /// Blocks the thread while the predicate returns `true`.
    ///
    /// This method will atomically unlock the mutex specified by the guard and block
    /// the current thread as long as the `condition` closure returns `true`.
    ///
    /// # Platform Behavior
    ///
    /// - **Native**: Uses thread parking for efficient blocking
    /// - **WASM with `Atomics.wait`**: Blocks using `Atomics.wait`
    /// - **WASM without `Atomics.wait`**: **Will panic** - use [`wait_sync_while`](Self::wait_sync_while) instead
    ///
    /// This is a low-level primitive that unconditionally blocks. For adaptive
    /// behavior that works everywhere (including browser main threads where
    /// `Atomics.wait` is unavailable), use [`wait_sync_while`](Self::wait_sync_while).
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
    /// use wasm_safe_thread::{Mutex, condvar::Condvar};
    /// use std::sync::Arc;
    /// # use std::thread;
    ///
    /// let pair = Arc::new((Mutex::new(false), Condvar::new()));
    /// let pair_clone = Arc::clone(&pair);
    ///
    /// thread::spawn(move || {
    ///     let (mutex, condvar) = &*pair_clone;
    ///     let mut ready = mutex.lock_sync();
    ///     *ready = true;
    ///     drop(ready);
    ///     condvar.notify_one();
    /// });
    ///
    /// let (mutex, condvar) = &*pair;
    /// let mut ready = mutex.lock_sync();
    /// // Wait until ready becomes true
    /// ready = condvar.wait_block_while(ready, |r| !*r);
    /// assert!(*ready);
    /// ```
    pub fn wait_block_while<'a, T, F>(
        &self,
        mut guard: Guard<'a, T>,
        mut condition: F,
    ) -> Guard<'a, T>
    where
        F: FnMut(&mut T) -> bool,
    {
        while condition(&mut guard) {
            guard = self.wait_block(guard);
        }
        guard
    }

    /// Blocks the current thread with a deadline for notification.
    ///
    /// This method will atomically unlock the mutex specified by the guard and block
    /// the current thread. When a notification is received or the timeout expires,
    /// the thread will wake up and re-acquire the lock before returning.
    ///
    /// # Platform Behavior
    ///
    /// - **Native**: Uses thread parking with timeout
    /// - **WASM with `Atomics.wait`**: Blocks using `Atomics.wait` with timeout
    /// - **WASM without `Atomics.wait`**: **Will panic** - use [`wait_sync_timeout`](Self::wait_sync_timeout) instead
    ///
    /// This is a low-level primitive that unconditionally blocks. For adaptive
    /// behavior that works everywhere (including browser main threads where
    /// `Atomics.wait` is unavailable), use [`wait_sync_timeout`](Self::wait_sync_timeout).
    ///
    /// # Spurious Wakeups
    ///
    /// This method may return spuriously (without a notification). Always use it
    /// in a loop that checks the condition.
    ///
    /// # Examples
    ///
    /// ```
    /// # // std::thread::spawn panics on wasm32
    /// # if cfg!(target_arch = "wasm32") { return; }
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
    /// thread::spawn(move || {
    ///     let (mutex, condvar) = &*pair_clone;
    ///     let mut ready = mutex.lock_sync();
    ///     *ready = true;
    ///     drop(ready);
    ///     condvar.notify_one();
    /// });
    ///
    /// let (mutex, condvar) = &*pair;
    /// let mut ready = mutex.lock_sync();
    /// let deadline = Instant::now() + Duration::from_secs(1);
    /// while !*ready {
    ///     let result;
    ///     (ready, result) = condvar.wait_block_timeout(ready, deadline);
    ///     if result.timed_out() {
    ///         break;
    ///     }
    /// }
    /// assert!(*ready);
    /// ```
    pub fn wait_block_timeout<'a, T>(
        &self,
        guard: Guard<'a, T>,
        deadline: Instant,
    ) -> (Guard<'a, T>, WaitTimeoutResult) {
        let mutex = guard.mutex;

        // Register this thread as waiting before releasing the lock
        self.waiting_sync_threads.with_mut(|threads| {
            threads.push(thread::current());
        });

        // Explicitly drop the guard to release the mutex
        drop(guard);

        loop {
            let now = Instant::now();
            if now >= deadline {
                // We timed out. We need to remove ourselves from the wait list.
                // It's possible we were notified just now, so we check one last time after locking.
                let notified = self.waiting_sync_threads.with_mut(|threads| {
                    // Find our thread and remove it
                    let current = thread::current();
                    if let Some(pos) = threads.iter().position(|x| x.id() == current.id()) {
                        threads.remove(pos);
                        false // We removed ourselves, so we were NOT notified by someone else popping us
                    } else {
                        true // We were not in the list, so we MUST have been notified/popped
                    }
                });

                if notified {
                    // We were notified, so we shouldn't return timeout.
                    // We just need to re-acquire the lock.
                    return (mutex.lock_sync(), WaitTimeoutResult(false));
                } else {
                    return (mutex.lock_sync(), WaitTimeoutResult(true));
                }
            }

            let timeout = deadline - now;
            // Park this thread while waiting for notification or timeout
            thread::park_timeout(timeout);

            // Check if we were notified
            let notified = self.waiting_sync_threads.with_mut(|threads| {
                let current = thread::current();
                if threads.iter().any(|x| x.id() == current.id()) {
                    // We are still in the list, so we were NOT notified (spurious wakeup or timeout)
                    false
                } else {
                    // We are not in the list, so we MUST have been notified/popped
                    true
                }
            });

            if notified {
                return (mutex.lock_sync(), WaitTimeoutResult(false));
            }
        }
    }

    /// Blocks the thread while the predicate remains `true`, with a deadline.
    ///
    /// This method will atomically unlock the mutex specified by the guard and block
    /// the current thread as long as the `condition` closure returns `true`.
    ///
    /// # Platform Behavior
    ///
    /// - **Native**: Uses thread parking with timeout
    /// - **WASM with `Atomics.wait`**: Blocks using `Atomics.wait` with timeout
    /// - **WASM without `Atomics.wait`**: **Will panic** - use [`wait_sync_timeout_while`](Self::wait_sync_timeout_while) instead
    ///
    /// This is a low-level primitive that unconditionally blocks. For adaptive
    /// behavior that works everywhere (including browser main threads where
    /// `Atomics.wait` is unavailable), use [`wait_sync_timeout_while`](Self::wait_sync_timeout_while).
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
    ///     let (mutex, condvar) = &*pair_clone;
    ///     let mut value = mutex.lock_sync();
    ///     *value = 10;
    ///     drop(value);
    ///     condvar.notify_one();
    /// });
    ///
    /// let (mutex, condvar) = &*pair;
    /// let guard = mutex.lock_sync();
    /// let deadline = Instant::now() + Duration::from_secs(1);
    /// let (guard, result) = condvar.wait_block_timeout_while(guard, deadline, |v| *v < 10);
    /// if !result.timed_out() {
    ///     assert_eq!(*guard, 10);
    /// }
    /// ```
    pub fn wait_block_timeout_while<'a, T, F>(
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
            (guard, result) = self.wait_block_timeout(guard, deadline);
            if result.timed_out() {
                return (guard, result);
            }
        }
        (guard, WaitTimeoutResult(false))
    }
}
