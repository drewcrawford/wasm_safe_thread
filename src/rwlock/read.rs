// SPDX-License-Identifier: MIT OR Apache-2.0
use super::LOCKED_WRITE;
use super::inner::RwLock;
use crate::NotAvailable;
use crate::guard::ReadGuard;
use std::sync::atomic::Ordering::{Acquire, Relaxed};

#[cfg(target_arch = "wasm32")]
use crate as thread;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

impl<T> RwLock<T> {
    /// Attempts to acquire a read lock without blocking.
    ///
    /// Returns immediately with either a guard to the protected data or
    /// a `NotAvailable` error if a writer currently holds the lock.
    ///
    /// Multiple readers can hold read locks simultaneously, so this will
    /// only fail if there's an active writer.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    /// use wasm_safe_thread::NotAvailable;
    ///
    /// let rwlock = RwLock::new("data");
    ///
    /// // Multiple readers can acquire locks
    /// let guard1 = rwlock.try_lock_read().unwrap();
    /// let guard2 = rwlock.try_lock_read().unwrap();
    /// assert_eq!(*guard1, "data");
    /// assert_eq!(*guard2, "data");
    /// ```
    ///
    /// ## Writer Blocks Readers
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    /// use wasm_safe_thread::NotAvailable;
    ///
    /// let rwlock = RwLock::new(0);
    /// let _writer = rwlock.lock_sync_write();
    ///
    /// // This will fail because a writer holds the lock
    /// assert!(matches!(rwlock.try_lock_read(), Err(NotAvailable)));
    /// ```
    pub fn try_lock_read(&self) -> Result<ReadGuard<'_, T>, NotAvailable> {
        let r = self.data_lock.fetch_update(Acquire, Relaxed, |f| {
            if f & LOCKED_WRITE != 0 {
                None
            } else if f == LOCKED_WRITE - 1 {
                panic!("Too many readers")
            } else {
                Some(f + 1)
            }
        });
        match r {
            Ok(_) => Ok(ReadGuard { mutex: self }),
            Err(_) => Err(NotAvailable),
        }
    }

    /// Acquires a read lock by spinning until it becomes available.
    ///
    /// This method will continuously check if the lock is available in a tight
    /// loop. While this ensures the lock is eventually acquired, it consumes
    /// CPU cycles while waiting. Use this when you know the lock will be held
    /// only briefly, or when blocking is not possible (e.g., WASM main thread).
    ///
    /// Multiple readers can hold locks simultaneously - this only spins if
    /// a writer currently holds the lock.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    ///
    /// let rwlock = RwLock::new(vec![1, 2, 3]);
    ///
    /// let guard1 = rwlock.lock_spin_read();
    /// let guard2 = rwlock.lock_spin_read();
    ///
    /// // Both readers can access the data
    /// assert_eq!(guard1.len(), guard2.len());
    /// ```
    pub fn lock_spin_read(&self) -> ReadGuard<'_, T> {
        // Spin until we can acquire the lock
        loop {
            let r = self.try_lock_read();
            match r {
                Ok(r) => {
                    return r;
                }
                Err(_) => {
                    std::hint::spin_loop();
                }
            }
        }
    }

    /// Acquires a read lock by blocking the current thread until it becomes available.
    ///
    /// This method will put the thread to sleep if a writer holds the lock,
    /// allowing other threads to run. When the writer releases the lock, waiting
    /// reader threads are woken up to acquire read access.
    ///
    /// Multiple readers can acquire locks simultaneously once no writer is active.
    ///
    /// # Platform Behavior
    ///
    /// - **Native**: Uses thread parking for efficient blocking
    /// - **WASM with `Atomics.wait`**: Blocks using `Atomics.wait`
    /// - **WASM without `Atomics.wait`**: **Will panic** - use [`lock_sync_read`](Self::lock_sync_read) instead
    ///
    /// This is a low-level primitive that unconditionally blocks. For adaptive
    /// behavior that works everywhere (including browser main threads where
    /// `Atomics.wait` is unavailable), use [`lock_sync_read`](Self::lock_sync_read).
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    ///
    /// let rwlock = RwLock::new(HashMap::from([("key", "value")]));
    ///
    /// // This will block if a writer holds the lock
    /// let guard = rwlock.lock_block_read();
    /// assert_eq!(guard.get("key"), Some(&"value"));
    /// # use std::collections::HashMap;
    /// ```
    pub fn lock_block_read(&self) -> ReadGuard<'_, T> {
        //insert our thread into the waiting list
        loop {
            let r = self.waiting_sync_read_threads.with_mut(|threads| {
                match self.try_lock_read() {
                    Ok(guard) => {
                        // Return the guard
                        Ok(guard)
                    }
                    Err(_) => {
                        let handle = thread::current();
                        threads.push(handle);
                        Err(NotAvailable)
                    }
                }
            });
            match r {
                Ok(guard) => return guard,
                Err(NotAvailable) => thread::park(),
            }
        }
    }

    /// Asynchronously acquires a read lock.
    ///
    /// This method returns a future that resolves to a read guard when the lock
    /// becomes available. Unlike the blocking variants, this doesn't block
    /// the async executor, allowing other tasks to run while waiting.
    ///
    /// Multiple readers can hold locks simultaneously - this only waits if
    /// a writer currently holds the lock.
    ///
    /// # Examples
    ///
    /// ```
    /// # test_executors::spin_on(async {
    /// use wasm_safe_thread::rwlock::RwLock;
    ///
    /// let rwlock = RwLock::new(vec!["async", "data"]);
    ///
    /// let guard1 = rwlock.lock_async_read().await;
    /// let guard2 = rwlock.lock_async_read().await;
    ///
    /// // Both readers can access simultaneously
    /// assert_eq!(guard1.len(), 2);
    /// assert_eq!(guard2[0], "async");
    /// # });
    /// ```
    pub async fn lock_async_read(&self) -> ReadGuard<'_, T> {
        loop {
            let a = self.waiting_async_read_threads.with_mut(|senders| {
                match self.try_lock_read() {
                    Ok(guard) => Ok(guard),
                    Err(NotAvailable) => {
                        // Create a new channel to signal when the lock is available
                        let (sender, receiver) = r#continue::continuation();
                        senders.push(sender);
                        Err(receiver)
                    }
                }
            });
            match a {
                Ok(guard) => return guard,
                Err(receiver) => {
                    // Wait for the signal that the lock is available
                    receiver.await;
                }
            }
        }
    }

    /// Automatically chooses the right read locking strategy for your platform.
    ///
    /// This is the recommended method for acquiring read locks as it papers over
    /// all platform differences:
    /// - **Native**: Uses efficient thread parking
    /// - **WASM with `Atomics.wait`**: Uses `Atomics.wait` for proper blocking
    /// - **WASM without `Atomics.wait`**: Falls back to spinning (e.g., browser main thread)
    ///
    /// You don't need to worry about "cannot block on main thread" errors -
    /// this method handles that automatically by detecting the environment
    /// and choosing the appropriate strategy.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    ///
    /// let rwlock = RwLock::new(vec!["apple", "banana"]);
    ///
    /// // Automatically uses the best strategy for the platform
    /// let guard1 = rwlock.lock_sync_read();
    /// let guard2 = rwlock.lock_sync_read();
    ///
    /// // Multiple readers work on all platforms
    /// assert_eq!(guard1.len(), 2);
    /// assert_eq!(guard2[1], "banana");
    /// ```
    ///
    /// ## Cross-Platform Code
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    ///
    /// fn read_data(rwlock: &RwLock<String>) -> usize {
    ///     // Works efficiently on both native and WASM
    ///     let guard = rwlock.lock_sync_read();
    ///     guard.len()
    /// }
    ///
    /// let rwlock = RwLock::new(String::from("cross-platform"));
    /// assert_eq!(read_data(&rwlock), 14);
    /// ```
    pub fn lock_sync_read(&self) -> ReadGuard<'_, T> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.lock_block_read()
        }
        #[cfg(target_arch = "wasm32")]
        {
            if crate::wasm_support::atomics_wait_supported() {
                self.lock_block_read()
            } else {
                // Fallback to spin lock if Atomics.wait is not supported
                self.lock_spin_read()
            }
        }
    }

    /// Accesses the data inside the RwLock synchronously with a read-only closure.
    ///
    /// This method acquires a read lock, executes the provided closure with a reference
    /// to the protected data, and immediately releases the lock. This ensures the
    /// critical section is as short as possible.
    ///
    /// Multiple threads can execute read closures simultaneously, making this efficient
    /// for read-heavy workloads.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    ///
    /// let rwlock = RwLock::new(vec![1, 2, 3, 4, 5]);
    ///
    /// // Calculate sum without holding the lock longer than needed
    /// let sum = rwlock.with_sync(|data| data.iter().sum::<i32>());
    /// assert_eq!(sum, 15);
    ///
    /// // Multiple operations in one critical section
    /// let (first, last) = rwlock.with_sync(|data| {
    ///     (data.first().copied(), data.last().copied())
    /// });
    /// assert_eq!(first, Some(1));
    /// assert_eq!(last, Some(5));
    /// ```
    pub fn with_sync<R, F: FnOnce(&T) -> R>(&self, f: F) -> R {
        let guard = self.lock_sync_read();
        f(&guard)
    }

    /// Accesses the data inside the RwLock asynchronously with a read-only closure.
    ///
    /// This method asynchronously acquires a read lock, executes the provided closure
    /// with a reference to the protected data, and immediately releases the lock.
    /// This ensures the critical section is as short as possible while not blocking
    /// the async executor.
    ///
    /// Multiple async tasks can read simultaneously, making this efficient for
    /// concurrent async read operations.
    ///
    /// # Examples
    ///
    /// ```
    /// # test_executors::spin_on(async {
    /// use wasm_safe_thread::rwlock::RwLock;
    ///
    /// let rwlock = RwLock::new(String::from("async world"));
    ///
    /// let length = rwlock.with_async(|s| s.len()).await;
    /// assert_eq!(length, 11);
    ///
    /// let uppercase = rwlock.with_async(|s| s.to_uppercase()).await;
    /// assert_eq!(uppercase, "ASYNC WORLD");
    /// # });
    /// ```
    pub async fn with_async<R, F: FnOnce(&T) -> R>(&self, f: F) -> R {
        let guard = self.lock_async_read().await;
        f(&guard)
    }
}
