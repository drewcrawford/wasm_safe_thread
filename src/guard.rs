// SPDX-License-Identifier: MIT OR Apache-2.0
//! Guard types for mutex and rwlock locks.
//!
//! This module provides guard types that wrap access to mutex-protected and rwlock-protected data.

use crate::Mutex;
use crate::rwlock::{LOCKED_WRITE, RwLock, UNLOCKED};

/// A guard that provides access to the data protected by a `Mutex`.
///
/// This guard is created by the locking methods on [`Mutex`]. When the guard
/// is dropped, the lock is automatically released, allowing other threads to
/// acquire it.
///
/// The guard implements `Deref` and `DerefMut`, allowing you to access the
/// protected data directly.
///
/// # RAII Pattern
///
/// This guard follows the RAII (Resource Acquisition Is Initialization) pattern.
/// The lock is acquired when the guard is created and released when the guard is dropped.
/// This ensures that locks are always released, even if the code panics or returns early.
///
/// You can manually drop the guard using `drop(guard)` to release the lock early.
///
/// # Examples
///
/// ```
/// use wasm_safe_thread::Mutex;
///
/// let mutex = Mutex::new(String::from("hello"));
///
/// // The guard provides access to the protected data
/// let mut guard = mutex.lock_sync();
/// guard.push_str(", world!");
/// assert_eq!(&*guard, "hello, world!");
///
/// // Lock is released when guard is dropped
/// drop(guard);
/// ```
pub struct Guard<'a, T> {
    pub(crate) mutex: &'a Mutex<T>,
    pub(crate) data: &'a mut T,
}

impl<T> std::ops::Deref for Guard<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.data
    }
}

impl<T> std::ops::DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.data
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        // Release the lock
        self.mutex
            .data_lock
            .store(false, std::sync::atomic::Ordering::Release);
        // Notify any waiting threads
        self.mutex.did_unlock();
    }
}

// ================================================================================================
// Boilerplate trait implementations
// ================================================================================================

impl<T: std::fmt::Debug> std::fmt::Debug for Guard<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guard")
            .field("data", &**self)
            .finish_non_exhaustive()
    }
}

impl<T: std::fmt::Display> std::fmt::Display for Guard<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&**self, f)
    }
}

impl<T> AsRef<T> for Guard<'_, T> {
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T> AsMut<T> for Guard<'_, T> {
    fn as_mut(&mut self) -> &mut T {
        self
    }
}

// ================================================================================================
// RwLock Guards
// ================================================================================================

/// A guard that provides read-only access to the data protected by an [`RwLock`].
///
/// This guard is created by the read locking methods on [`RwLock`]. When the guard
/// is dropped, the read lock is automatically released, allowing writers to acquire
/// the lock if no other readers are active.
///
/// Multiple `ReadGuard`s can exist simultaneously for the same `RwLock`, enabling
/// concurrent read access.
///
/// # Examples
///
/// ```
/// use wasm_safe_thread::rwlock::RwLock;
///
/// let rwlock = RwLock::new(vec![1, 2, 3]);
///
/// {
///     let guard1 = rwlock.lock_sync_read();
///     let guard2 = rwlock.lock_sync_read();
///
///     // Both guards can read simultaneously
///     assert_eq!(guard1.len(), 3);
///     assert_eq!(guard2[0], 1);
/// } // Both guards dropped, read locks released
/// ```
#[derive(Debug)]
pub struct ReadGuard<'a, T> {
    pub(crate) mutex: &'a RwLock<T>,
}

/// A guard that provides exclusive read-write access to the data protected by an [`RwLock`].
///
/// This guard is created by the write locking methods on [`RwLock`]. When the guard
/// is dropped, the write lock is automatically released, allowing other readers or
/// writers to acquire the lock.
///
/// Only one `WriteGuard` can exist at a time for a given `RwLock`, ensuring exclusive
/// access for modifications.
///
/// # Examples
///
/// ```
/// use wasm_safe_thread::rwlock::RwLock;
///
/// let rwlock = RwLock::new(String::from("hello"));
///
/// {
///     let mut guard = rwlock.lock_sync_write();
///     guard.push_str(", world!");
///     assert_eq!(&*guard, "hello, world!");
/// } // Guard dropped, write lock released
/// ```
#[derive(Debug)]
pub struct WriteGuard<'a, T> {
    pub(crate) mutex: &'a RwLock<T>,
}

impl<'a, T> AsRef<T> for ReadGuard<'a, T> {
    fn as_ref(&self) -> &T {
        self
    }
}

impl<'a, T> AsRef<T> for WriteGuard<'a, T> {
    fn as_ref(&self) -> &T {
        self
    }
}

impl<'a, T> AsMut<T> for WriteGuard<'a, T> {
    fn as_mut(&mut self) -> &mut T {
        &mut *self
    }
}

impl<'a, T: std::fmt::Display> std::fmt::Display for ReadGuard<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&**self, f)
    }
}

impl<'a, T: std::fmt::Display> std::fmt::Display for WriteGuard<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&**self, f)
    }
}

impl<'a, T> std::ops::Deref for WriteGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.mutex.inner.get() }
    }
}

impl<'a, T> std::ops::DerefMut for WriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.inner.get() }
    }
}

impl<'a, T> std::ops::Deref for ReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.inner.get() }
    }
}

impl<'a, T> Drop for ReadGuard<'a, T> {
    fn drop(&mut self) {
        let r = self
            .mutex
            .data_lock
            .fetch_sub(1, std::sync::atomic::Ordering::Release);
        assert!(r > 0);
        self.mutex.did_unlock_read();
    }
}

impl<'a, T> Drop for WriteGuard<'a, T> {
    fn drop(&mut self) {
        let old = self
            .mutex
            .data_lock
            .swap(UNLOCKED, std::sync::atomic::Ordering::Release);
        assert!(old == LOCKED_WRITE);
        self.mutex.did_unlock_write();
    }
}
