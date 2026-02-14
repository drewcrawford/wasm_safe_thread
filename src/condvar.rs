// SPDX-License-Identifier: MIT OR Apache-2.0
//! A WebAssembly-safe condition variable implementation.
//!
//! This module provides a condition variable that works across native and WebAssembly targets,
//! automatically adapting its waiting strategy based on the runtime environment.

use crate::spinlock::Spinlock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(target_arch = "wasm32")]
use web_time::Instant;

#[cfg(target_arch = "wasm32")]
use crate as thread;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

// Re-export for internal use within condvar submodules
#[cfg(target_arch = "wasm32")]
pub(crate) use crate::wasm_support::atomics_wait_supported;

#[derive(Debug)]
struct AsyncWaiter {
    id: u64,
    sender: r#continue::Sender<()>,
}

static ASYNC_WAITER_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A condition variable that works across native and WebAssembly targets.
///
/// A condition variable allows threads to wait for a particular condition to become true,
/// and to notify other threads when the condition changes. This implementation automatically
/// adapts to the platform:
///
/// - **Native**: Uses efficient thread parking
/// - **WASM with `Atomics.wait`**: Uses `Atomics.wait` for proper blocking
/// - **WASM without `Atomics.wait`**: Falls back to spinning (e.g., browser main thread)
///
/// # Examples
///
/// ## Basic Usage
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
/// assert!(*ready);
/// ```
///
/// ## Producer-Consumer Pattern
///
/// ```
/// # // std::thread::spawn panics on wasm32
/// # if cfg!(target_arch = "wasm32") { return; }
/// use wasm_safe_thread::{Mutex, condvar::Condvar};
/// use std::sync::Arc;
/// use std::collections::VecDeque;
/// # use std::thread;
///
/// let shared = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
/// let producer = Arc::clone(&shared);
///
/// // Producer thread
/// thread::spawn(move || {
///     let (mutex, condvar) = &*producer;
///     for i in 0..5 {
///         let mut queue = mutex.lock_sync();
///         queue.push_back(i);
///         drop(queue);
///         condvar.notify_one();
///     }
/// });
///
/// // Consumer
/// let (mutex, condvar) = &*shared;
/// let mut collected = Vec::new();
/// for _ in 0..5 {
///     let mut queue = mutex.lock_sync();
///     while queue.is_empty() {
///         queue = condvar.wait_sync(queue);
///     }
///     if let Some(value) = queue.pop_front() {
///         collected.push(value);
///     }
/// }
/// assert_eq!(collected, vec![0, 1, 2, 3, 4]);
/// ```
#[derive(Debug)]
pub struct Condvar {
    waiting_sync_threads: Spinlock<Vec<thread::Thread>>,
    waiting_async_threads: Spinlock<Vec<AsyncWaiter>>,
    waiting_spin_threads: Spinlock<Vec<Arc<AtomicBool>>>,
}

impl Condvar {
    /// Creates a new condition variable.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::condvar::Condvar;
    ///
    /// let condvar = Condvar::new();
    /// ```
    pub const fn new() -> Self {
        Condvar {
            waiting_sync_threads: Spinlock::new(vec![]),
            waiting_async_threads: Spinlock::new(vec![]),
            waiting_spin_threads: Spinlock::new(vec![]),
        }
    }
}

/// A type indicating whether a timed wait on a condition variable returned
/// due to a time out or not.
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash, Default)]
pub struct WaitTimeoutResult(bool);

impl WaitTimeoutResult {
    /// Returns `true` if the wait was known to have timed out.
    pub fn timed_out(&self) -> bool {
        self.0
    }
}

impl Default for Condvar {
    /// Creates a new condition variable with default settings.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::condvar::Condvar;
    ///
    /// let condvar: Condvar = Default::default();
    /// ```
    fn default() -> Self {
        Condvar::new()
    }
}

unsafe impl Send for Condvar {}
unsafe impl Sync for Condvar {}

mod notify;
mod wait_async;
mod wait_block;
mod wait_spin;
mod wait_sync;

#[cfg(test)]
mod tests;
