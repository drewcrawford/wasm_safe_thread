// SPDX-License-Identifier: MIT OR Apache-2.0
//! A WebAssembly-safe read-write lock that papers over platform-specific locking constraints.
//!
//! # The Core Problem
//!
//! Like regular mutexes, **WebAssembly's main thread cannot use blocking locks**. However,
//! read-write locks face an additional challenge: they need to efficiently handle multiple
//! concurrent readers while ensuring exclusive access for writers, all while avoiding the
//! "cannot block on the main thread" panic.
//!
//! # The Solution
//!
//! This crate provides a read-write lock that automatically adapts its locking strategy
//! based on the runtime environment, just like our Mutex, but with the added benefit of
//! allowing multiple concurrent readers:
//!
//! - **Native**: Uses efficient thread parking for both readers and writers
//! - **WASM with `Atomics.wait`**: Uses `Atomics.wait` for proper blocking
//! - **WASM without `Atomics.wait`**: Falls back to spinning (e.g., browser main thread)
//!
//! This means you get the performance benefits of read-write locks (multiple concurrent readers)
//! while maintaining compatibility across all platforms without worrying about thread restrictions.
//!
//! # Features
//!
//! - **Multiple concurrent readers**: Better performance for read-heavy workloads
//! - **Exclusive writer access**: Ensures data consistency during writes
//! - **Transparent adaptation**: Automatically detects and uses the best locking mechanism
//! - **Main thread safe**: Won't panic on WASM main thread (uses spinning instead)
//! - **Worker thread optimized**: Uses proper blocking when available for efficiency
//! - **Multiple strategies**: Try-lock, spin-lock, blocking lock, and async lock for both read and write
//!
//! # Examples
//!
//! ## Basic Usage
//!
//! ```
//! use wasm_safe_thread::rwlock::RwLock;
//!
//! let rwlock = RwLock::new(42);
//!
//! // Multiple readers can access simultaneously
//! let guard1 = rwlock.lock_sync_read();
//! let guard2 = rwlock.lock_sync_read();
//! assert_eq!(*guard1, 42);
//! assert_eq!(*guard2, 42);
//! drop(guard1);
//! drop(guard2);
//!
//! // Writer gets exclusive access
//! let mut guard = rwlock.lock_sync_write();
//! *guard = 100;
//! drop(guard);
//!
//! // Read the updated value
//! let guard = rwlock.lock_sync_read();
//! assert_eq!(*guard, 100);
//! ```
//!
//! ## Try Lock
//!
//! ```
//! use wasm_safe_thread::rwlock::RwLock;
//! use wasm_safe_thread::NotAvailable;
//!
//! let rwlock = RwLock::new("data");
//!
//! // First read lock succeeds
//! let guard1 = rwlock.try_lock_read().unwrap();
//!
//! // Second read lock also succeeds (multiple readers allowed)
//! let guard2 = rwlock.try_lock_read().unwrap();
//! assert_eq!(*guard1, "data");
//! assert_eq!(*guard2, "data");
//!
//! // Write lock fails while readers are active
//! let result = rwlock.try_lock_write();
//! assert!(matches!(result, Err(NotAvailable)));
//! ```
//!
//! ## Async Usage
//!
//! ```
//! # test_executors::spin_on(async {
//! use wasm_safe_thread::rwlock::RwLock;
//!
//! let rwlock = RwLock::new(vec![1, 2, 3]);
//!
//! // Async read doesn't block the executor
//! let guard = rwlock.lock_async_read().await;
//! let sum: i32 = guard.iter().sum();
//! drop(guard);
//!
//! // Async write for modifications
//! let mut guard = rwlock.lock_async_write().await;
//! guard.push(4);
//! drop(guard);
//!
//! // Using the convenience method
//! let len = rwlock.with_async(|data| data.len()).await;
//! assert_eq!(len, 4);
//! # });
//! ```
//!
//! ## Thread-Safe Sharing with Multiple Readers
//!
//! ```
//! # // std::thread::spawn panics on wasm32
//! # if cfg!(target_arch = "wasm32") { return; }
//! use wasm_safe_thread::rwlock::RwLock;
//! use std::sync::Arc;
//! # use std::thread;
//!
//! let rwlock = Arc::new(RwLock::new(vec![1, 2, 3, 4, 5]));
//! let mut handles = vec![];
//!
//! // Spawn multiple reader threads
//! for i in 0..3 {
//!     let rwlock = Arc::clone(&rwlock);
//!     handles.push(thread::spawn(move || {
//!         let guard = rwlock.lock_sync_read();
//!         let sum: i32 = guard.iter().sum();
//!         println!("Reader {} calculated sum: {}", i, sum);
//!         sum
//!     }));
//! }
//!
//! // All readers can work concurrently
//! for handle in handles {
//!     assert_eq!(handle.join().unwrap(), 15);
//! }
//! ```

mod inner;
mod read;
mod write;

#[cfg(test)]
mod tests;

pub use inner::RwLock;

pub(crate) const UNLOCKED: u8 = 0;
pub(crate) const LOCKED_WRITE: u8 = 0b10000000;
