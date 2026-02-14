// SPDX-License-Identifier: MIT OR Apache-2.0
use super::Condvar;
use std::sync::atomic::Ordering;

impl Condvar {
    /// Wakes up one blocked thread on this condition variable.
    ///
    /// If there are multiple threads waiting, one will be woken up (unspecified which one).
    /// If no threads are waiting, this is a no-op.
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
    ///     ready = condvar.wait_sync(ready);
    /// }
    /// ```
    pub fn notify_one(&self) {
        //Try to wake one spinlock first

        let thread = self.waiting_spin_threads.with_mut(|threads| threads.pop());
        if let Some(thread) = thread {
            eprintln!("Popped a waiting_spin_thread");
            thread.store(true, Ordering::Release);
            return;
        }
        // Try to wake one sync thread first
        let thread = self.waiting_sync_threads.with_mut(|threads| threads.pop());
        if let Some(thread) = thread {
            thread.unpark();
            return;
        }

        // If no sync threads, wake one async task
        let waiter = self.waiting_async_threads.with_mut(|waiters| waiters.pop());
        if let Some(waiter) = waiter {
            waiter.sender.send(());
        }
    }

    /// Wakes up all blocked threads on this condition variable.
    ///
    /// All waiting threads will be woken up. If no threads are waiting, this is a no-op.
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
    /// let pair = Arc::new((Mutex::new(0), Condvar::new()));
    /// let mut handles = vec![];
    ///
    /// for _ in 0..3 {
    ///     let pair = Arc::clone(&pair);
    ///     handles.push(thread::spawn(move || {
    ///         let (mutex, condvar) = &*pair;
    ///         let mut count = mutex.lock_sync();
    ///         while *count < 10 {
    ///             count = condvar.wait_sync(count);
    ///         }
    ///     }));
    /// }
    ///
    /// let (mutex, condvar) = &*pair;
    /// let mut count = mutex.lock_sync();
    /// *count = 10;
    /// drop(count);
    /// condvar.notify_all();
    ///
    /// for handle in handles {
    ///     handle.join().unwrap();
    /// }
    /// ```
    pub fn notify_all(&self) {
        //wake all spin threads
        let threads = self.waiting_spin_threads.with_mut(std::mem::take);
        for thread in threads {
            thread.store(true, Ordering::Release);
        }

        // Wake all sync threads
        let threads = self.waiting_sync_threads.with_mut(std::mem::take);
        for thread in threads {
            thread.unpark();
        }

        // Wake all async tasks
        let waiters = self.waiting_async_threads.with_mut(std::mem::take);
        for waiter in waiters {
            waiter.sender.send(());
        }
    }
}
