//! WebAssembly backend - placeholder implementation

use std::time::Duration;
use std::io;
use std::num::NonZeroUsize;
use std::fmt;
use std::marker::PhantomData;

/// A thread local storage key which owns its contents.
pub struct LocalKey<T: 'static> {
    _marker: PhantomData<T>,
}

impl<T: 'static> LocalKey<T> {
    /// Creates a new `LocalKey`.
    #[doc(hidden)]
    pub const fn new(_init: fn() -> T) -> Self {
        LocalKey { _marker: PhantomData }
    }

    /// Acquires a reference to the value in this TLS key.
    pub fn with<F, R>(&'static self, _f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        todo!("wasm LocalKey::with")
    }

    /// Acquires a reference to the value in this TLS key.
    ///
    /// Returns `Err(AccessError)` if the key is being destroyed or was already destroyed.
    pub fn try_with<F, R>(&'static self, _f: F) -> Result<R, AccessError>
    where
        F: FnOnce(&T) -> R,
    {
        todo!("wasm LocalKey::try_with")
    }
}

impl<T: 'static> fmt::Debug for LocalKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalKey").finish_non_exhaustive()
    }
}

/// An error returned by [`LocalKey::try_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessError;

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "already destroyed or being destroyed")
    }
}

impl std::error::Error for AccessError {}

/// A handle to a thread.
pub struct JoinHandle<T> {
    _marker: std::marker::PhantomData<T>,
}

impl<T> JoinHandle<T> {
    /// Waits for the thread to finish and returns its result.
    pub fn join(self) -> Result<T, Box<dyn std::any::Any + Send + 'static>> {
        todo!("wasm JoinHandle::join")
    }

    /// Gets the thread associated with this handle.
    pub fn thread(&self) -> &Thread {
        todo!("wasm JoinHandle::thread")
    }

    /// Checks if the thread has finished running.
    pub fn is_finished(&self) -> bool {
        todo!("wasm JoinHandle::is_finished")
    }
}

/// A handle to a thread.
#[derive(Clone)]
pub struct Thread {
    _private: (),
}

impl Thread {
    /// Gets the thread's unique identifier.
    pub fn id(&self) -> ThreadId {
        todo!("wasm Thread::id")
    }

    /// Gets the thread's name.
    pub fn name(&self) -> Option<&str> {
        todo!("wasm Thread::name")
    }

    /// Atomically makes the handle's token available if it is not already.
    pub fn unpark(&self) {
        todo!("wasm Thread::unpark")
    }
}

/// A unique identifier for a running thread.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ThreadId(u64);

/// A builder for configuring and spawning threads.
pub struct Builder {
    _name: Option<String>,
    _stack_size: Option<usize>,
}

impl Builder {
    /// Creates a new thread builder.
    pub fn new() -> Self {
        Builder {
            _name: None,
            _stack_size: None,
        }
    }

    /// Sets the name of the thread.
    pub fn name(mut self, name: String) -> Self {
        self._name = Some(name);
        self
    }

    /// Sets the stack size for the new thread.
    pub fn stack_size(mut self, size: usize) -> Self {
        self._stack_size = Some(size);
        self
    }

    /// Spawns a new thread with this builder's configuration.
    pub fn spawn<F, T>(self, _f: F) -> io::Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        todo!("wasm Builder::spawn")
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawns a new thread, returning a JoinHandle for it.
pub fn spawn<F, T>(_f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    todo!("wasm spawn")
}

/// Gets a handle to the thread that invokes it.
pub fn current() -> Thread {
    todo!("wasm current")
}

/// Puts the current thread to sleep for at least the specified duration.
pub fn sleep(_dur: Duration) {
    todo!("wasm sleep")
}

/// Cooperatively gives up a timeslice to the OS scheduler.
pub fn yield_now() {
    todo!("wasm yield_now")
}

/// Blocks unless or until the current thread's token is made available.
pub fn park() {
    todo!("wasm park")
}

/// Blocks unless or until the current thread's token is made available
/// or the specified duration has been reached.
pub fn park_timeout(_dur: Duration) {
    todo!("wasm park_timeout")
}

/// Returns an estimate of the default amount of parallelism a program should use.
pub fn available_parallelism() -> io::Result<NonZeroUsize> {
    todo!("wasm available_parallelism")
}
