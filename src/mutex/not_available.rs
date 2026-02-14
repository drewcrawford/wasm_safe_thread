// SPDX-License-Identifier: MIT OR Apache-2.0
/// Error returned when a lock cannot be immediately acquired.
///
/// This error is returned by [`Mutex::try_lock`](crate::Mutex::try_lock) when the mutex is already
/// locked by another thread.
///
/// # Examples
///
/// ```
/// use wasm_safe_thread::{Mutex, NotAvailable};
///
/// let mutex = Mutex::new(42);
/// let _guard = mutex.lock_sync();
///
/// // Try to lock while already locked
/// match mutex.try_lock() {
///     Ok(_) => panic!("Should not succeed"),
///     Err(NotAvailable) => println!("Lock is held by another thread"),
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NotAvailable;

impl std::fmt::Display for NotAvailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "lock not available")
    }
}

impl std::error::Error for NotAvailable {}
