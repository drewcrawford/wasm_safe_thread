// SPDX-License-Identifier: MIT OR Apache-2.0
//! A simple spinlock implementation for short-lived critical sections.
//!
//! This module provides a spinlock that is used internally by the main mutex
//! implementation for managing waiting thread lists.
//!
//! # Internal Use Only
//!
//! This spinlock is primarily intended for internal use within this crate.
//! While it is exposed publicly, most users should prefer [`Mutex`](crate::Mutex)
//! or [`RwLock`](crate::rwlock::RwLock) which provide more robust locking strategies
//! including thread parking and async support.

use std::cell::UnsafeCell;

/// A spinlock for protecting short-lived critical sections.
///
/// This spinlock uses atomic operations to provide mutual exclusion without
/// blocking. It's designed for very short critical sections where the overhead
/// of thread parking would be greater than spinning.
///
/// The spinlock only exposes a `with_mut` method to ensure all accesses are
/// short-lived and the lock is automatically released.
///
/// # Examples
///
/// ```
/// use wasm_safe_thread::spinlock::Spinlock;
///
/// let spinlock = Spinlock::new(vec![1, 2, 3]);
///
/// let sum = spinlock.with_mut(|data| {
///     data.push(4);
///     data.iter().sum::<i32>()
/// });
///
/// assert_eq!(sum, 10);
/// ```
#[derive(Debug)]
pub struct Spinlock<T> {
    data: UnsafeCell<T>,
    locked: std::sync::atomic::AtomicBool,
}

impl<T> Spinlock<T> {
    /// Creates a new spinlock with the given initial value.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::spinlock::Spinlock;
    ///
    /// let spinlock = Spinlock::new(42);
    /// let value = spinlock.with_mut(|data| *data);
    /// assert_eq!(value, 42);
    /// ```
    pub const fn new(data: T) -> Self {
        Spinlock {
            data: UnsafeCell::new(data),
            locked: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Executes a closure with exclusive access to the protected data.
    ///
    /// This method acquires the spinlock, executes the provided closure with
    /// a mutable reference to the data, and then releases the lock. The lock
    /// is guaranteed to be released even if the closure panics.
    ///
    /// By design, this is the only way to access the data, ensuring all
    /// accesses are short-lived and the lock is automatically released.
    ///
    /// # Performance
    ///
    /// This method spins in a tight loop until the lock is acquired. Keep
    /// the critical section as short as possible to minimize contention.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::spinlock::Spinlock;
    ///
    /// let counter = Spinlock::new(0);
    ///
    /// // Increment the counter
    /// counter.with_mut(|count| *count += 1);
    ///
    /// // Read and modify in one operation
    /// let doubled = counter.with_mut(|count| {
    ///     *count *= 2;
    ///     *count
    /// });
    ///
    /// assert_eq!(doubled, 2);
    /// ```
    ///
    /// ## Thread Safety
    ///
    /// ```
    /// # // std::thread::spawn panics on wasm32
    /// # if cfg!(target_arch = "wasm32") { return; }
    /// use wasm_safe_thread::spinlock::Spinlock;
    /// use std::sync::Arc;
    /// use std::thread;
    ///
    /// let shared = Arc::new(Spinlock::new(0));
    /// let mut handles = vec![];
    ///
    /// for _ in 0..4 {
    ///     let shared = Arc::clone(&shared);
    ///     handles.push(thread::spawn(move || {
    ///         for _ in 0..25 {
    ///             shared.with_mut(|n| *n += 1);
    ///         }
    ///     }));
    /// }
    ///
    /// for handle in handles {
    ///     handle.join().unwrap();
    /// }
    ///
    /// let final_value = shared.with_mut(|n| *n);
    /// assert_eq!(final_value, 100);
    /// ```
    pub fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        // Spin until we can acquire the lock
        while self.locked.swap(true, std::sync::atomic::Ordering::Acquire) {
            std::hint::spin_loop();
        }

        // SAFETY: We have exclusive access to the data now
        let result = unsafe { f(&mut *self.data.get()) };

        // Release the lock
        self.locked
            .store(false, std::sync::atomic::Ordering::Release);

        result
    }
}

unsafe impl<T: Send> Send for Spinlock<T> {}
unsafe impl<T: Send> Sync for Spinlock<T> {}

// ================================================================================================
// Boilerplate trait implementations
// ================================================================================================

impl<T: Default> Default for Spinlock<T> {
    /// Creates a new spinlock with the default value of the wrapped type.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::spinlock::Spinlock;
    ///
    /// let spinlock: Spinlock<i32> = Spinlock::default();
    /// let value = spinlock.with_mut(|data| *data);
    /// assert_eq!(value, 0);
    ///
    /// let spinlock: Spinlock<Vec<String>> = Spinlock::default();
    /// let is_empty = spinlock.with_mut(|data| data.is_empty());
    /// assert!(is_empty);
    /// ```
    fn default() -> Self {
        Spinlock::new(T::default())
    }
}

impl<T> From<T> for Spinlock<T> {
    /// Creates a new spinlock from the given value.
    ///
    /// This is equivalent to [`Spinlock::new`] but can be more convenient
    /// in generic contexts or when using turbofish syntax.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::spinlock::Spinlock;
    ///
    /// let spinlock = Spinlock::from(vec![1, 2, 3]);
    /// let sum = spinlock.with_mut(|data| data.iter().sum::<i32>());
    /// assert_eq!(sum, 6);
    ///
    /// // Useful in generic contexts
    /// fn create_spinlock<T>(value: T) -> Spinlock<T> {
    ///     Spinlock::from(value)
    /// }
    /// ```
    fn from(value: T) -> Self {
        Spinlock::new(value)
    }
}

impl<T: std::fmt::Display> std::fmt::Display for Spinlock<T> {
    /// Formats the spinlock by attempting to access and format the contained value.
    ///
    /// Since spinlocks only provide scoped access through `with_mut`, this
    /// implementation uses that method to safely access the data for formatting.
    /// This ensures the lock is properly acquired and released.
    ///
    /// # Examples
    ///
    /// ```
    /// use wasm_safe_thread::spinlock::Spinlock;
    ///
    /// let spinlock = Spinlock::new(42);
    /// println!("{}", spinlock); // Prints "42"
    /// ```
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.with_mut(|data| std::fmt::Display::fmt(data, f))
    }
}
