//! Standard library backend - forwards to std::thread

use std::thread;
use std::time::Duration;
use std::io;
use std::num::NonZeroUsize;

/// A handle to a thread.
pub struct JoinHandle<T>(thread::JoinHandle<T>);

impl<T> JoinHandle<T> {
    /// Waits for the thread to finish and returns its result.
    pub fn join(self) -> Result<T, Box<dyn std::any::Any + Send + 'static>> {
        self.0.join()
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
}

impl Builder {
    /// Creates a new thread builder.
    pub fn new() -> Self {
        Builder {
            inner: thread::Builder::new(),
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

    /// Spawns a new thread with this builder's configuration.
    pub fn spawn<F, T>(self, f: F) -> io::Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.inner.spawn(f).map(JoinHandle)
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
