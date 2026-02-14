// SPDX-License-Identifier: MIT OR Apache-2.0
use super::inner::RwLock;
use super::{LOCKED_WRITE, UNLOCKED};
use crate::NotAvailable;
use crate::guard::WriteGuard;
use std::sync::atomic::Ordering::{Acquire, Relaxed};

#[cfg(target_arch = "wasm32")]
use crate as thread;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

impl<T> RwLock<T> {
    /// Attempts to acquire a write lock without blocking.
    ///
    /// Returns immediately with either a guard to the protected data or
    /// a `NotAvailable` error if any readers or another writer currently
    /// hold locks.
    ///
    /// Write locks are exclusive - no other readers or writers can be active.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    /// use wasm_safe_thread::NotAvailable;
    ///
    /// let rwlock = RwLock::new(vec![1, 2, 3]);
    ///
    /// match rwlock.try_lock_write() {
    ///     Ok(mut guard) => {
    ///         guard.push(4);
    ///         println!("Updated: {:?}", *guard);
    ///     }
    ///     Err(NotAvailable) => {
    ///         println!("Could not acquire write lock");
    ///     }
    /// }
    /// ```
    ///
    /// ## Readers Block Writers
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    /// use wasm_safe_thread::NotAvailable;
    ///
    /// let rwlock = RwLock::new(0);
    /// let _reader = rwlock.lock_sync_read();
    ///
    /// // This will fail because a reader holds a lock
    /// assert!(matches!(rwlock.try_lock_write(), Err(NotAvailable)));
    /// ```
    pub fn try_lock_write(&self) -> Result<WriteGuard<'_, T>, NotAvailable> {
        match self
            .data_lock
            .compare_exchange(UNLOCKED, LOCKED_WRITE, Acquire, Relaxed)
        {
            Ok(_) => Ok(WriteGuard { mutex: self }),
            Err(_) => Err(NotAvailable),
        }
    }

    /// Acquires a write lock by spinning until it becomes available.
    ///
    /// This method will continuously check if the lock is available in a tight
    /// loop. The lock can only be acquired when no readers or writers are active.
    ///
    /// While spinning ensures the lock is eventually acquired, it consumes CPU
    /// cycles while waiting. Use this when blocking is not possible or when
    /// you know the lock will be available soon.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    ///
    /// let rwlock = RwLock::new(String::from("hello"));
    ///
    /// let mut guard = rwlock.lock_spin_write();
    /// guard.push_str(", world!");
    /// assert_eq!(&*guard, "hello, world!");
    /// ```
    pub fn lock_spin_write(&self) -> WriteGuard<'_, T> {
        // Spin until we can acquire the lock
        loop {
            let r = self.try_lock_write();
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

    /// Acquires a write lock by blocking the current thread until it becomes available.
    ///
    /// This method will put the thread to sleep if any readers or another writer
    /// hold locks, allowing other threads to run. When all locks are released,
    /// one waiting writer thread is woken up to acquire exclusive access.
    ///
    /// # Platform Behavior
    ///
    /// - **Native**: Uses thread parking for efficient blocking
    /// - **WASM with `Atomics.wait`**: Blocks using `Atomics.wait`
    /// - **WASM without `Atomics.wait`**: **Will panic** - use [`lock_sync_write`](Self::lock_sync_write) instead
    ///
    /// This is a low-level primitive that unconditionally blocks. For adaptive
    /// behavior that works everywhere (including browser main threads where
    /// `Atomics.wait` is unavailable), use [`lock_sync_write`](Self::lock_sync_write).
    ///
    /// # Examples
    ///
    /// ```
    /// # // std::thread::spawn panics on wasm32
    /// # if cfg!(target_arch = "wasm32") { return; }
    /// use wasm_safe_thread::rwlock::RwLock;
    /// use std::sync::Arc;
    /// # use std::thread;
    ///
    /// let rwlock = Arc::new(RwLock::new(0));
    /// let rwlock_clone = Arc::clone(&rwlock);
    ///
    /// thread::spawn(move || {
    ///     let mut guard = rwlock_clone.lock_block_write();
    ///     *guard = 42;
    /// });
    ///
    /// # thread::sleep(std::time::Duration::from_millis(10));
    /// let guard = rwlock.lock_block_read();
    /// assert_eq!(*guard, 42);
    /// ```
    pub fn lock_block_write(&self) -> WriteGuard<'_, T> {
        loop {
            let r =
                self.waiting_sync_write_threads
                    .with_mut(|threads| match self.try_lock_write() {
                        Ok(guard) => Ok(guard),
                        Err(_) => {
                            let handle = thread::current();
                            threads.push(handle);
                            Err(NotAvailable)
                        }
                    });
            match r {
                Ok(guard) => return guard,
                Err(NotAvailable) => thread::park(),
            }
        }
    }

    /// Asynchronously acquires a write lock.
    ///
    /// This method returns a future that resolves to a write guard when the lock
    /// becomes available. Unlike the blocking variants, this doesn't block
    /// the async executor, allowing other tasks to run while waiting.
    ///
    /// The write lock is exclusive - it waits until all readers and any other
    /// writer release their locks.
    ///
    /// # Examples
    ///
    /// ```
    /// # test_executors::spin_on(async {
    /// use wasm_safe_thread::rwlock::RwLock;
    /// use std::collections::HashMap;
    ///
    /// let rwlock = RwLock::new(HashMap::new());
    ///
    /// let mut guard = rwlock.lock_async_write().await;
    /// guard.insert("key", "value");
    /// drop(guard);
    ///
    /// let guard = rwlock.lock_async_read().await;
    /// assert_eq!(guard.get("key"), Some(&"value"));
    /// # });
    /// ```
    pub async fn lock_async_write(&self) -> WriteGuard<'_, T> {
        loop {
            let a = self.waiting_async_write_threads.with_mut(|senders| {
                match self.try_lock_write() {
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

    /// Automatically chooses the right write locking strategy for your platform.
    ///
    /// This is the recommended method for acquiring write locks as it papers over
    /// all platform differences:
    /// - **Native**: Uses efficient thread parking
    /// - **WASM with `Atomics.wait`**: Uses `Atomics.wait` for proper blocking
    /// - **WASM without `Atomics.wait`**: Falls back to spinning (e.g., browser main thread)
    ///
    /// The write lock provides exclusive access - no readers or other writers
    /// can access the data while this lock is held.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    ///
    /// let rwlock = RwLock::new(vec![1, 2, 3]);
    ///
    /// // Automatically uses the best strategy for the platform
    /// let mut guard = rwlock.lock_sync_write();
    /// guard.push(4);
    /// assert_eq!(guard.len(), 4);
    /// ```
    ///
    /// ## Cross-Platform Modifications
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    /// use std::collections::HashMap;
    ///
    /// fn update_config(rwlock: &RwLock<HashMap<String, i32>>) {
    ///     // Works on both native and WASM without changes
    ///     let mut guard = rwlock.lock_sync_write();
    ///     guard.insert("version".to_string(), 2);
    /// }
    ///
    /// let rwlock = RwLock::new(HashMap::new());
    /// update_config(&rwlock);
    ///
    /// let guard = rwlock.lock_sync_read();
    /// assert_eq!(guard.get("version"), Some(&2));
    /// ```
    pub fn lock_sync_write(&self) -> WriteGuard<'_, T> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.lock_block_write()
        }
        #[cfg(target_arch = "wasm32")]
        {
            if crate::wasm_support::atomics_wait_supported() {
                self.lock_block_write()
            } else {
                // Fallback to spin lock if Atomics.wait is not supported
                self.lock_spin_write()
            }
        }
    }

    /// Accesses the data inside the RwLock synchronously with a mutable closure.
    ///
    /// This method acquires a write lock, executes the provided closure with a mutable
    /// reference to the protected data, and immediately releases the lock. This ensures
    /// exclusive access during modifications.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::rwlock::RwLock;
    ///
    /// let rwlock = RwLock::new(vec![1, 2, 3]);
    ///
    /// // Modify and return a value
    /// let new_len = rwlock.with_mut_sync(|data| {
    ///     data.push(4);
    ///     data.len()
    /// });
    /// assert_eq!(new_len, 4);
    /// ```
    pub fn with_mut_sync<R, F: FnOnce(&mut T) -> R>(&self, f: F) -> R {
        let mut guard = self.lock_sync_write();
        f(&mut guard)
    }

    /// Accesses the data inside the RwLock asynchronously with a mutable closure.
    ///
    /// This method asynchronously acquires a write lock, executes the provided closure
    /// with a mutable reference to the protected data, and immediately releases the lock.
    /// This ensures exclusive access for modifications without blocking the async executor.
    ///
    /// # Examples
    ///
    /// ```
    /// # test_executors::spin_on(async {
    /// use wasm_safe_thread::rwlock::RwLock;
    /// use std::collections::HashMap;
    ///
    /// let rwlock = RwLock::new(HashMap::new());
    ///
    /// // Add multiple entries in one async operation
    /// rwlock.with_mut_async(|map| {
    ///     map.insert("async", 1);
    ///     map.insert("await", 2);
    /// }).await;
    ///
    /// let sum = rwlock.with_async(|map| {
    ///     map.values().sum::<i32>()
    /// }).await;
    /// assert_eq!(sum, 3);
    /// # });
    /// ```
    pub async fn with_mut_async<R, F: FnOnce(&mut T) -> R>(&self, f: F) -> R {
        let mut guard = self.lock_async_write().await;
        f(&mut guard)
    }
}
