// SPDX-License-Identifier: MIT OR Apache-2.0
use super::Instant;
use super::{Condvar, WaitTimeoutResult};
use crate::Guard;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

impl Condvar {
    /// Waits by spinning for a notification from this condition variable.
    ///
    /// This method will atomically unlock the mutex specified by the guard and
    /// spin in a tight loop while waiting to be notified. When a notification is received, the
    /// mutex will be re-acquired before returning.
    ///
    /// While this ensures the wait completes, it consumes CPU cycles. Use this
    /// when you know notifications will arrive quickly, or when blocking is not
    /// possible (e.g., WASM main thread).
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
    ///     ready = condvar.wait_spin(ready);
    /// }
    /// assert!(*ready);
    /// ```
    pub fn wait_spin<'a, T>(&self, guard: Guard<'a, T>) -> Guard<'a, T> {
        let wake = Arc::new(AtomicBool::new(false));
        let mutex = guard.mutex;
        //insert into wait queue
        self.waiting_spin_threads.with_mut(|e| e.push(wake.clone()));

        // Release the mutex
        drop(guard);

        while !wake.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }

        // Re-acquire the mutex before returning
        mutex.lock_sync()
    }

    /// Waits by spinning while the predicate remains `true`.
    ///
    /// This method will atomically unlock the mutex specified by the guard and
    /// spin in a tight loop as long as the `condition` closure returns `true`.
    ///
    /// # Platform Behavior
    ///
    /// - **All Platforms**: Uses a busy loop (spinning) to wait. This consumes CPU
    ///   cycles but works in all environments, including WASM main thread.
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
    /// ready = condvar.wait_spin_while(ready, |r| !*r);
    /// assert!(*ready);
    /// ```
    pub fn wait_spin_while<'a, T, F>(
        &self,
        mut guard: Guard<'a, T>,
        mut condition: F,
    ) -> Guard<'a, T>
    where
        F: FnMut(&mut T) -> bool,
    {
        while condition(&mut guard) {
            guard = self.wait_spin(guard);
        }
        guard
    }

    /// Waits by spinning with a deadline for notification.
    ///
    /// This method will atomically unlock the mutex specified by the guard and
    /// spin in a tight loop while waiting for notification or the deadline. When a notification is received
    /// or the timeout expires, the mutex will be re-acquired before returning.
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
    ///     (ready, result) = condvar.wait_spin_timeout(ready, deadline);
    ///     if result.timed_out() {
    ///         break;
    ///     }
    /// }
    /// ```
    pub fn wait_spin_timeout<'a, T>(
        &self,
        guard: Guard<'a, T>,
        deadline: Instant,
    ) -> (Guard<'a, T>, WaitTimeoutResult) {
        let wake = Arc::new(AtomicBool::new(false));
        let mutex = guard.mutex;
        //insert into wait queue
        self.waiting_spin_threads.with_mut(|e| e.push(wake.clone()));

        // Release the mutex
        drop(guard);

        loop {
            if wake.load(Ordering::Acquire) {
                // Re-acquire the mutex before returning
                return (mutex.lock_sync(), WaitTimeoutResult(false));
            }
            if Instant::now() >= deadline {
                // We timed out. We need to remove ourselves from the wait list.
                // It's possible we were notified just now, so we check one last time after locking.
                let notified = self.waiting_spin_threads.with_mut(|threads| {
                    // Find our wake arc and remove it
                    if let Some(pos) = threads.iter().position(|x| Arc::ptr_eq(x, &wake)) {
                        threads.remove(pos);
                        false // We removed ourselves, so we were NOT notified by someone else popping us
                    } else {
                        true // We were not in the list, so we MUST have been notified/popped
                    }
                });

                if notified {
                    // We were notified, so we shouldn't return timeout.
                    // We still need to wait for the wake flag to be set to true by the notifier
                    // effectively behaving as a normal wait_spin completion.
                    while !wake.load(Ordering::Acquire) {
                        std::hint::spin_loop();
                    }
                    return (mutex.lock_sync(), WaitTimeoutResult(false));
                } else {
                    return (mutex.lock_sync(), WaitTimeoutResult(true));
                }
            }
            std::hint::spin_loop();
        }
    }

    /// Waits by spinning while the predicate remains `true` with a deadline.
    ///
    /// This method will atomically unlock the mutex specified by the guard and
    /// spin in a tight loop as long as the `condition` closure returns `true`.
    ///
    /// # Platform Behavior
    ///
    /// - **All Platforms**: Uses a busy loop (spinning) to wait. This consumes CPU
    ///   cycles but works in all environments, including WASM main thread.
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
    /// let mut guard = mutex.lock_sync();
    /// let deadline = Instant::now() + Duration::from_secs(1);
    /// let (guard, result) = condvar.wait_spin_timeout_while(guard, deadline, |v| *v < 10);
    /// if !result.timed_out() {
    ///     assert_eq!(*guard, 10);
    /// }
    /// ```
    pub fn wait_spin_timeout_while<'a, T, F>(
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
            (guard, result) = self.wait_spin_timeout(guard, deadline);
            if result.timed_out() {
                return (guard, result);
            }
        }
        (guard, WaitTimeoutResult(false))
    }
}
