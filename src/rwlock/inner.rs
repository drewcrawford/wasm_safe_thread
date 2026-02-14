// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::spinlock::Spinlock;
use std::cell::UnsafeCell;
use std::fmt::Display;
use std::sync::atomic::AtomicU8;

#[cfg(target_arch = "wasm32")]
use crate as thread;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

use super::UNLOCKED;

/// A reader-writer lock that works across native and WebAssembly targets.
///
/// This lock allows multiple readers to access the data simultaneously, but only
/// one writer at a time. Writers get exclusive access - no readers or other writers
/// can access the data while a write lock is held.
///
/// Like [`Mutex`](crate::Mutex), this lock provides multiple locking strategies:
/// - **`try_lock_read/write`**: Non-blocking attempt to acquire the lock
/// - **`lock_spin_read/write`**: Spin-wait until the lock is acquired
/// - **`lock_block_read/write`**: Unconditionally blocks (will panic without `Atomics.wait`)
/// - **`lock_sync_read/write`**: Automatically chooses blocking or spinning based on platform
/// - **`lock_async_read/write`**: Always non-blocking, works everywhere
///
/// ## Platform Behavior
///
/// The `_sync` methods transparently handle platform differences:
/// - **Native**: Full blocking with thread parking
/// - **WASM with `Atomics.wait`**: Blocks using `Atomics.wait`
/// - **WASM without `Atomics.wait`**: Falls back to spinning (e.g., browser main thread)
///
/// ## When to Use RwLock vs Mutex
///
/// Use `RwLock` when:
/// - Your workload is read-heavy (many reads, few writes)
/// - Multiple threads need to read data simultaneously
/// - You want to maximize concurrent access for readers
///
/// Use [`Mutex`](crate::Mutex) when:
/// - Reads and writes are equally common
/// - Operations are very quick
/// - You want simpler semantics
///
/// # Examples
///
/// ## Basic Reader-Writer Pattern
///
/// ```
/// use wasm_safe_thread::rwlock::RwLock;
///
/// let rwlock = RwLock::new(0i32);
///
/// // Multiple readers can access simultaneously
/// {
///     let reader1 = rwlock.lock_sync_read();
///     let reader2 = rwlock.lock_sync_read();
///     assert_eq!(*reader1, *reader2);
/// } // Both read locks released here
///
/// // Writer gets exclusive access
/// {
///     let mut writer = rwlock.lock_sync_write();
///     *writer += 1;
/// } // Write lock released here
///
/// // Read the updated value
/// let value = rwlock.with_sync(|data| *data);
/// assert_eq!(value, 1);
/// ```
///
/// ## Concurrent Readers Example
///
/// ```
/// # // std::thread::spawn panics on wasm32
/// # if cfg!(target_arch = "wasm32") { return; }
/// use wasm_safe_thread::rwlock::RwLock;
/// use std::sync::Arc;
/// # use std::thread;
///
/// let shared_data = Arc::new(RwLock::new(vec![1, 2, 3, 4, 5]));
///
/// // Multiple threads can read simultaneously
/// let data_clone1 = Arc::clone(&shared_data);
/// let handle1 = thread::spawn(move || {
///     data_clone1.with_sync(|vec| {
///         vec.iter().sum::<i32>()
///     })
/// });
///
/// let data_clone2 = Arc::clone(&shared_data);
/// let handle2 = thread::spawn(move || {
///     data_clone2.with_sync(|vec| {
///         vec.len()
///     })
/// });
///
/// // Both readers execute concurrently
/// let sum = handle1.join().unwrap();
/// let len = handle2.join().unwrap();
/// assert_eq!(sum, 15);
/// assert_eq!(len, 5);
/// ```
#[derive(Debug, Default)]
pub struct RwLock<T> {
    pub(crate) inner: UnsafeCell<T>,
    pub(crate) data_lock: AtomicU8,
    pub(crate) waiting_sync_read_threads: Spinlock<Vec<thread::Thread>>,
    pub(crate) waiting_sync_write_threads: Spinlock<Vec<thread::Thread>>,
    pub(crate) waiting_async_read_threads: Spinlock<Vec<r#continue::Sender<()>>>,
    pub(crate) waiting_async_write_threads: Spinlock<Vec<r#continue::Sender<()>>>,
}

impl<T: Display> Display for RwLock<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.try_lock_read() {
            Ok(guard) => std::fmt::Display::fmt(&*guard, f),
            Err(_) => write!(f, "Mutex {{ <locked> }}"),
        }
    }
}

impl<T> From<T> for RwLock<T> {
    fn from(value: T) -> Self {
        RwLock::new(value)
    }
}

unsafe impl<T: Send> Send for RwLock<T> {}
unsafe impl<T: Send> Sync for RwLock<T> {}

impl<T> RwLock<T> {
    /// Creates a new read-write lock with the given initial value.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    ///
    /// let rwlock = RwLock::new(42);
    /// assert_eq!(*rwlock.lock_sync_read(), 42);
    /// ```
    pub const fn new(value: T) -> RwLock<T> {
        RwLock {
            inner: UnsafeCell::new(value),
            data_lock: AtomicU8::new(UNLOCKED),
            waiting_sync_read_threads: Spinlock::new(vec![]),
            waiting_async_read_threads: Spinlock::new(vec![]),
            waiting_sync_write_threads: Spinlock::new(vec![]),
            waiting_async_write_threads: Spinlock::new(vec![]),
        }
    }

    pub(crate) fn did_unlock_write(&self) {
        //pop the waiting READ threads
        let threads = self.waiting_sync_read_threads.with_mut(std::mem::take);
        for thread in threads {
            // Wake up the thread
            thread.unpark();
        }
        let threads = self.waiting_async_read_threads.with_mut(std::mem::take);
        for thread in threads {
            thread.send(())
        }

        //AND the write threads
        let threads = self.waiting_sync_write_threads.with_mut(std::mem::take);
        for thread in threads {
            thread.unpark();
        }
        let threads = self.waiting_async_write_threads.with_mut(std::mem::take);
        for thread in threads {
            thread.send(())
        }
    }

    pub(crate) fn did_unlock_read(&self) {
        //unlock only WRITE threads
        let threads = self.waiting_sync_write_threads.with_mut(std::mem::take);
        for thread in threads {
            thread.unpark();
        }
        let threads = self.waiting_async_write_threads.with_mut(std::mem::take);
        for thread in threads {
            thread.send(())
        }
    }
}
