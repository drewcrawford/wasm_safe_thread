//! Standard library backend - forwards to std::thread

use std::fmt;
use std::io;
use std::num::NonZeroUsize;
use std::thread;
use std::time::Duration;

/// A thread local storage key which owns its contents.
pub struct LocalKey<T: 'static> {
    inner: &'static std::thread::LocalKey<T>,
}

impl<T: 'static> LocalKey<T> {
    /// Creates a new `LocalKey` wrapping a std `LocalKey`.
    #[doc(hidden)]
    pub const fn new(inner: &'static std::thread::LocalKey<T>) -> Self {
        LocalKey { inner }
    }

    /// Acquires a reference to the value in this TLS key.
    pub fn with<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        self.inner.with(f)
    }

    /// Acquires a reference to the value in this TLS key.
    ///
    /// Returns `Err(AccessError)` if the key is being destroyed or was already destroyed.
    pub fn try_with<F, R>(&'static self, f: F) -> Result<R, AccessError>
    where
        F: FnOnce(&T) -> R,
    {
        self.inner.try_with(f).map_err(|_| AccessError)
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
pub struct JoinHandle<T>(thread::JoinHandle<T>);

impl<T> JoinHandle<T> {
    /// Waits for the thread to finish and returns its result.
    pub fn join(self) -> Result<T, Box<dyn std::any::Any + Send + 'static>> {
        self.0.join()
    }

    pub async fn join_async(self) -> Result<T, Box<String>> where T: Send + 'static {
        let (c,s) = r#continue::continuation();
        std::thread::Builder::new()
            .name("wasm_safe_thread::join_async".to_string())
            .spawn(move || {
                let output = self.join().map_err(|e| Box::new(format!("{:?}", e)) as Box<String>);
                c.send(output);
            });
        s.await
    }
    /// Gets the thread associated with this handle.
    pub fn thread(&self) -> &Thread {
        // We need to wrap the thread reference, but since Thread wraps std::thread::Thread
        // and we can't easily do that for a reference, we'll need a different approach.
        // For now, this is a limitation - we could store the Thread in JoinHandle.
        unimplemented!("thread() not yet implemented")
    }

    /// Checks if the thread has finished running.
    pub fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
}

/// A handle to a thread.
#[derive(Clone)]
pub struct Thread(thread::Thread);

impl Thread {
    /// Gets the thread's unique identifier.
    pub fn id(&self) -> ThreadId {
        ThreadId(self.0.id())
    }

    /// Gets the thread's name.
    pub fn name(&self) -> Option<&str> {
        self.0.name()
    }

    /// Atomically makes the handle's token available if it is not already.
    pub fn unpark(&self) {
        self.0.unpark()
    }
}

/// A unique identifier for a running thread.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ThreadId(thread::ThreadId);

/// A builder for configuring and spawning threads.
pub struct Builder {
    inner: thread::Builder,
    spawn_hooks: Vec<Box<dyn FnOnce() + Send + 'static>>,
}

impl Builder {
    /// Creates a new thread builder.
    pub fn new() -> Self {
        Builder {
            inner: thread::Builder::new(),
            spawn_hooks: Vec::new(),
        }
    }

    /// Sets the name of the thread.
    pub fn name(mut self, name: String) -> Self {
        self.inner = self.inner.name(name);
        self
    }

    /// Sets the stack size for the new thread.
    pub fn stack_size(mut self, size: usize) -> Self {
        self.inner = self.inner.stack_size(size);
        self
    }

    /// Registers a hook to run at the beginning of the spawned thread.
    ///
    /// Multiple hooks can be registered and they will run in the order they were added.
    pub fn spawn_hook<H>(mut self, hook: H) -> Self
    where
        H: FnOnce() + Send + 'static,
    {
        self.spawn_hooks.push(Box::new(hook));
        self
    }

    /// Spawns a new thread with this builder's configuration.
    pub fn spawn<F, T>(self, f: F) -> io::Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let Builder { inner, spawn_hooks } = self;
        inner
            .spawn(move || {
                for hook in spawn_hooks {
                    hook();
                }
                f()
            })
            .map(JoinHandle)
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawns a new thread, returning a JoinHandle for it.
pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    JoinHandle(thread::spawn(f))
}

/// Gets a handle to the thread that invokes it.
pub fn current() -> Thread {
    Thread(thread::current())
}

/// Puts the current thread to sleep for at least the specified duration.
pub fn sleep(dur: Duration) {
    thread::sleep(dur)
}

/// Cooperatively gives up a timeslice to the OS scheduler.
pub fn yield_now() {
    thread::yield_now()
}

/// Blocks unless or until the current thread's token is made available.
pub fn park() {
    thread::park()
}

/// Blocks unless or until the current thread's token is made available
/// or the specified duration has been reached.
pub fn park_timeout(dur: Duration) {
    thread::park_timeout(dur)
}

/// Returns an estimate of the default amount of parallelism a program should use.
pub fn available_parallelism() -> io::Result<NonZeroUsize> {
    thread::available_parallelism()
}
