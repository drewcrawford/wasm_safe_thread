// SPDX-License-Identifier: MIT OR Apache-2.0
//! A multi-producer, single-consumer FIFO queue communication primitive.
//!
//! This module provides message-based communication over channels, similar to `std::sync::mpsc`,
//! but designed to work reliably across Native and WebAssembly environments.
//!
//! # Features
//!
//! - **Cross-Platform**: Works on Native, WASM worker threads, and WASM main thread.
//! - **Async Support**: Provides `send_async` and `recv_async` for integration with async runtimes.
//! - **Platform-Aware**: Automatically chooses the best waiting strategy (blocking or spinning) based on the thread type.
//! - **Infinite Buffer**: The channel has an infinite buffer, so `send` never blocks (unless the lock is contended).
//!
//! # Usage
//!
//! Create a channel with [`channel()`], which returns a [`Sender`] and a [`Receiver`].
//!
//! ```
//! use wasm_safe_thread::mpsc::channel;
//!
//! let (tx, rx) = channel();
//!
//! tx.send_sync(42).unwrap();
//! assert_eq!(rx.recv_sync().unwrap(), 42);
//! ```

use crate::{Mutex, condvar::Condvar};
use std::cell::Cell;
use std::collections::VecDeque;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(target_arch = "wasm32")]
use web_time::Instant;

/// The shared state of the channel.
struct Shared<T> {
    queue: Mutex<VecDeque<T>>,
    condvar: Condvar,
    sender_count: AtomicUsize,
    receiver_active: AtomicBool,
}

/// The sending half of the channel.
///
/// Messages can be sent through this channel with `send_sync`, `send_block`, `send_spin`, or `send_async`.
/// The `Sender` can be cloned to create multiple producers that all send to the same `Receiver`.
pub struct Sender<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let old_count = self.shared.sender_count.fetch_sub(1, Ordering::SeqCst);
        if old_count == 1 {
            // Last sender dropped, notify receiver
            self.shared.condvar.notify_all();
        }
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.shared.sender_count.fetch_add(1, Ordering::SeqCst);
        Sender {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sender").finish()
    }
}

/// The receiving half of the channel.
pub struct Receiver<T> {
    shared: Arc<Shared<T>>,
    _marker: PhantomData<Cell<()>>,
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.shared.receiver_active.store(false, Ordering::SeqCst);
        // Notify senders so they can wake up and see the receiver is gone
        self.shared.condvar.notify_all();
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver").finish()
    }
}

/// An error returned from the `try_recv` method.
#[derive(PartialEq, Eq, Clone, Copy, Debug, Hash)]
#[non_exhaustive]
pub enum TryRecvError {
    /// The channel is empty.
    Empty,
    /// The channel has been disconnected.
    Disconnected,
}

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TryRecvError::Empty => "receiving on an empty channel".fmt(f),
            TryRecvError::Disconnected => "receiving on a closed channel".fmt(f),
        }
    }
}

impl std::error::Error for TryRecvError {}

/// An error returned from the `recv_timeout` methods.
#[derive(PartialEq, Eq, Clone, Copy, Debug, Hash)]
#[non_exhaustive]
pub enum RecvTimeoutError {
    /// The receive operation timed out.
    Timeout,
    /// The channel has been disconnected.
    Disconnected,
}

impl fmt::Display for RecvTimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecvTimeoutError::Timeout => "timed out waiting on channel".fmt(f),
            RecvTimeoutError::Disconnected => "channel is empty and disconnected".fmt(f),
        }
    }
}

impl std::error::Error for RecvTimeoutError {}

/// An error returned from the `recv` method.
#[derive(PartialEq, Eq, Clone, Copy, Debug, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum RecvError {
    /// The channel has been disconnected.
    Disconnected,
}

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "receiving on a closed channel".fmt(f)
    }
}

impl std::error::Error for RecvError {}

/// An error returned from the `send` methods.
#[derive(PartialEq, Eq, Clone, Copy)]
pub struct SendError<T>(pub T);

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SendError").finish_non_exhaustive()
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "sending on a closed channel".fmt(f)
    }
}

impl<T> std::error::Error for SendError<T> {}

/// Creates a new asynchronous channel, returning the sender/receiver halves.
///
/// All data sent on the `Sender` will become available on the `Receiver` in
/// the same order as it was sent, and no `send` will block the calling thread
/// (this channel has an "infinite buffer", unlike `sync_channel`, which will
/// block after its buffer limit is reached). `recv` will block until a message
/// is available.
///
/// The `Sender` can be cloned to `send` to the same channel multiple times, but
/// only one `Receiver` is supported.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let shared = Arc::new(Shared {
        queue: Mutex::new(VecDeque::new()),
        condvar: Condvar::new(),
        sender_count: AtomicUsize::new(1),
        receiver_active: AtomicBool::new(true),
    });
    (
        Sender {
            shared: Arc::clone(&shared),
        },
        Receiver {
            shared,
            _marker: PhantomData,
        },
    )
}

impl<T> Sender<T> {
    /// Sends a value on this channel, spinning if the lock is contended.
    pub fn send_spin(&self, t: T) -> Result<(), SendError<T>> {
        if !self.shared.receiver_active.load(Ordering::SeqCst) {
            return Err(SendError(t));
        }
        let mut queue = self.shared.queue.lock_spin();
        if !self.shared.receiver_active.load(Ordering::SeqCst) {
            return Err(SendError(t));
        }
        queue.push_back(t);
        drop(queue);
        self.shared.condvar.notify_one();
        Ok(())
    }

    /// Sends a value on this channel, blocking if the lock is contended.
    ///
    /// # Platform Behavior
    ///
    /// - **Native**: Uses thread parking for efficient blocking
    /// - **WASM with `Atomics.wait`**: Blocks using `Atomics.wait`
    /// - **WASM without `Atomics.wait`**: **Will panic** - use [`send_sync`](Self::send_sync) instead
    ///
    /// This is a low-level primitive that unconditionally blocks. For adaptive
    /// behavior that works everywhere (including browser main threads where
    /// `Atomics.wait` is unavailable), use [`send_sync`](Self::send_sync).
    pub fn send_block(&self, t: T) -> Result<(), SendError<T>> {
        if !self.shared.receiver_active.load(Ordering::SeqCst) {
            return Err(SendError(t));
        }
        let mut queue = self.shared.queue.lock_block();
        if !self.shared.receiver_active.load(Ordering::SeqCst) {
            return Err(SendError(t));
        }
        queue.push_back(t);
        drop(queue);
        self.shared.condvar.notify_one();
        Ok(())
    }

    /// Sends a value on this channel, using the appropriate strategy for the platform.
    ///
    /// # Platform Behavior
    ///
    /// - **Native**: Uses efficient thread parking if the lock is contended
    /// - **WASM with `Atomics.wait`**: Uses `Atomics.wait` for proper blocking if contended
    /// - **WASM without `Atomics.wait`**: Falls back to spinning (e.g., browser main thread)
    pub fn send_sync(&self, t: T) -> Result<(), SendError<T>> {
        if !self.shared.receiver_active.load(Ordering::SeqCst) {
            return Err(SendError(t));
        }
        let mut queue = self.shared.queue.lock_sync();
        if !self.shared.receiver_active.load(Ordering::SeqCst) {
            return Err(SendError(t));
        }
        queue.push_back(t);
        drop(queue);
        self.shared.condvar.notify_one();
        Ok(())
    }

    /// Sends a value on this channel asynchronously.
    pub async fn send_async(&self, t: T) -> Result<(), SendError<T>> {
        if !self.shared.receiver_active.load(Ordering::SeqCst) {
            return Err(SendError(t));
        }
        let mut queue = self.shared.queue.lock_async().await;
        if !self.shared.receiver_active.load(Ordering::SeqCst) {
            return Err(SendError(t));
        }
        queue.push_back(t);
        drop(queue);
        self.shared.condvar.notify_one();
        Ok(())
    }
}

impl<T> Receiver<T> {
    /// Attempts to return a pending value on this receiver without blocking.
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        let mut queue = match self.shared.queue.try_lock() {
            Ok(guard) => guard,
            Err(_) => return Err(TryRecvError::Empty),
        };
        match queue.pop_front() {
            Some(t) => Ok(t),
            None => {
                if self.shared.sender_count.load(Ordering::SeqCst) == 0 {
                    Err(TryRecvError::Disconnected)
                } else {
                    Err(TryRecvError::Empty)
                }
            }
        }
    }

    /// Receives a value from the channel, spinning if empty.
    pub fn recv_spin(&self) -> Result<T, RecvError> {
        let mut queue = self.shared.queue.lock_spin();
        loop {
            if let Some(t) = queue.pop_front() {
                return Ok(t);
            }
            if self.shared.sender_count.load(Ordering::SeqCst) == 0 {
                return Err(RecvError::Disconnected);
            }
            queue = self.shared.condvar.wait_spin(queue);
        }
    }

    /// Receives a value from the channel, spinning if empty, with a timeout.
    pub fn recv_spin_timeout(&self, deadline: Instant) -> Result<T, RecvTimeoutError> {
        let mut queue = match self.shared.queue.lock_spin_timeout(deadline) {
            Some(guard) => guard,
            None => return Err(RecvTimeoutError::Timeout),
        };
        loop {
            if let Some(t) = queue.pop_front() {
                return Ok(t);
            }
            if self.shared.sender_count.load(Ordering::SeqCst) == 0 {
                return Err(RecvTimeoutError::Disconnected);
            }
            let result;
            (queue, result) = self.shared.condvar.wait_spin_timeout(queue, deadline);
            if result.timed_out() {
                return Err(RecvTimeoutError::Timeout);
            }
        }
    }

    /// Receives a value from the channel, blocking if empty.
    ///
    /// # Platform Behavior
    ///
    /// - **Native**: Uses thread parking for efficient blocking
    /// - **WASM with `Atomics.wait`**: Blocks using `Atomics.wait`
    /// - **WASM without `Atomics.wait`**: **Will panic** - use [`recv_sync`](Self::recv_sync) instead
    ///
    /// This is a low-level primitive that unconditionally blocks. For adaptive
    /// behavior that works everywhere (including browser main threads where
    /// `Atomics.wait` is unavailable), use [`recv_sync`](Self::recv_sync).
    pub fn recv_block(&self) -> Result<T, RecvError> {
        let mut queue = self.shared.queue.lock_block();
        loop {
            if let Some(t) = queue.pop_front() {
                return Ok(t);
            }
            if self.shared.sender_count.load(Ordering::SeqCst) == 0 {
                return Err(RecvError::Disconnected);
            }
            queue = self.shared.condvar.wait_block(queue);
        }
    }

    /// Receives a value from the channel, blocking if empty, with a timeout.
    ///
    /// # Platform Behavior
    ///
    /// - **Native**: Uses thread parking with timeout
    /// - **WASM with `Atomics.wait`**: Blocks using `Atomics.wait` with timeout
    /// - **WASM without `Atomics.wait`**: **Will panic** - use [`recv_sync_timeout`](Self::recv_sync_timeout) instead
    ///
    /// This is a low-level primitive that unconditionally blocks. For adaptive
    /// behavior that works everywhere (including browser main threads where
    /// `Atomics.wait` is unavailable), use [`recv_sync_timeout`](Self::recv_sync_timeout).
    pub fn recv_block_timeout(&self, deadline: Instant) -> Result<T, RecvTimeoutError> {
        let mut queue = match self.shared.queue.lock_block_timeout(deadline) {
            Some(guard) => guard,
            None => return Err(RecvTimeoutError::Timeout),
        };
        loop {
            if let Some(t) = queue.pop_front() {
                return Ok(t);
            }
            if self.shared.sender_count.load(Ordering::SeqCst) == 0 {
                return Err(RecvTimeoutError::Disconnected);
            }
            let result;
            (queue, result) = self.shared.condvar.wait_block_timeout(queue, deadline);
            if result.timed_out() {
                return Err(RecvTimeoutError::Timeout);
            }
        }
    }

    /// Receives a value from the channel, using the appropriate strategy for the platform.
    ///
    /// # Platform Behavior
    ///
    /// - **Native**: Uses efficient thread parking while waiting for data
    /// - **WASM with `Atomics.wait`**: Uses `Atomics.wait` for proper blocking while waiting
    /// - **WASM without `Atomics.wait`**: Falls back to spinning (e.g., browser main thread)
    pub fn recv_sync(&self) -> Result<T, RecvError> {
        let mut queue = self.shared.queue.lock_sync();
        loop {
            if let Some(t) = queue.pop_front() {
                return Ok(t);
            }
            if self.shared.sender_count.load(Ordering::SeqCst) == 0 {
                return Err(RecvError::Disconnected);
            }
            queue = self.shared.condvar.wait_sync(queue);
        }
    }

    /// Receives a value from the channel, using the appropriate strategy for the platform, with a timeout.
    ///
    /// # Platform Behavior
    ///
    /// - **Native**: Uses efficient thread parking with timeout
    /// - **WASM with `Atomics.wait`**: Uses `Atomics.wait` with timeout
    /// - **WASM without `Atomics.wait`**: Falls back to spinning (e.g., browser main thread)
    pub fn recv_sync_timeout(&self, deadline: Instant) -> Result<T, RecvTimeoutError> {
        let mut queue = match self.shared.queue.lock_sync_timeout(deadline) {
            Some(guard) => guard,
            None => return Err(RecvTimeoutError::Timeout),
        };
        loop {
            if let Some(t) = queue.pop_front() {
                return Ok(t);
            }
            if self.shared.sender_count.load(Ordering::SeqCst) == 0 {
                return Err(RecvTimeoutError::Disconnected);
            }
            let result;
            (queue, result) = self.shared.condvar.wait_sync_timeout(queue, deadline);
            if result.timed_out() {
                return Err(RecvTimeoutError::Timeout);
            }
        }
    }

    /// Receives a value from the channel asynchronously.
    pub async fn recv_async(&self) -> Result<T, RecvError> {
        let mut queue = self.shared.queue.lock_async().await;
        loop {
            if let Some(t) = queue.pop_front() {
                return Ok(t);
            }
            if self.shared.sender_count.load(Ordering::SeqCst) == 0 {
                return Err(RecvError::Disconnected);
            }
            queue = self.shared.condvar.wait_async(queue).await;
        }
    }

    /// Receives a value from the channel asynchronously, with a timeout.
    pub async fn recv_async_timeout(&self, deadline: Instant) -> Result<T, RecvTimeoutError> {
        let mut queue = match self.shared.queue.lock_async_timeout(deadline).await {
            Some(guard) => guard,
            None => return Err(RecvTimeoutError::Timeout),
        };
        loop {
            if let Some(t) = queue.pop_front() {
                return Ok(t);
            }
            if self.shared.sender_count.load(Ordering::SeqCst) == 0 {
                return Err(RecvTimeoutError::Disconnected);
            }
            let result;
            (queue, result) = self
                .shared
                .condvar
                .wait_async_timeout(queue, deadline)
                .await;
            if result.timed_out() {
                return Err(RecvTimeoutError::Timeout);
            }
        }
    }
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        self.rx.recv_sync().ok()
    }
}

/// An iterator over messages from the receiver.
///
/// This iterator is created by calling `into_iter` on a `Receiver`.
/// It will block using the platform-appropriate strategy when waiting for messages,
/// and will yield `None` when the channel is disconnected.
pub struct IntoIter<T> {
    rx: Receiver<T>,
}

impl<T> fmt::Debug for IntoIter<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IntoIter").finish_non_exhaustive()
    }
}

impl<T> IntoIterator for Receiver<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> IntoIter<T> {
        IntoIter { rx: self }
    }
}

unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {} // Sender is Clone and uses Arc<Shared>, Shared uses Mutex which is Sync if T is Send.
unsafe impl<T: Send> Send for Receiver<T> {}

#[cfg(test)]
mod tests;
