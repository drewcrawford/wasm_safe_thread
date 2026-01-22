//! Standard library backend - forwards to std::thread

use std::fmt;
use std::io;
use std::num::NonZeroUsize;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread;
use std::time::Duration;
use wasm_safe_mutex::mpsc;

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
pub struct JoinHandle<T> {
    std_handle: thread::JoinHandle<()>,
    receiver: mpsc::Receiver<Result<T, Box<dyn std::any::Any + Send + 'static>>>,
    thread: Thread,
}

impl<T> JoinHandle<T> {
    /// Waits for the thread to finish and returns its result.
    pub fn join(self) -> Result<T, Box<dyn std::any::Any + Send + 'static>> {
        // The thread always sends a result before exiting, so this should never fail
        self.receiver
            .recv_sync()
            .expect("thread terminated without sending result")
    }

    pub async fn join_async(self) -> Result<T, Box<String>>
    where
        T: Send + 'static,
    {
        self.receiver
            .recv_async()
            .await
            .expect("thread terminated without sending result")
            .map_err(|e| Box::new(format!("{:?}", e)) as Box<String>)
    }

    /// Gets the thread associated with this handle.
    pub fn thread(&self) -> &Thread {
        &self.thread
    }

    /// Checks if the thread has finished running.
    pub fn is_finished(&self) -> bool {
        self.std_handle.is_finished()
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
        let (sender, receiver) = mpsc::channel();
        let std_handle = self.inner.spawn(move || {
            crate::hooks::run_spawn_hooks();
            let result = catch_unwind(AssertUnwindSafe(f));
            let _ = sender.send_sync(result);
        })?;
        let thread = Thread(std_handle.thread().clone());
        Ok(JoinHandle {
            std_handle,
            receiver,
            thread,
        })
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
    let (sender, receiver) = mpsc::channel();
    let std_handle = thread::spawn(move || {
        crate::hooks::run_spawn_hooks();
        let result = catch_unwind(AssertUnwindSafe(f));
        let _ = sender.send_sync(result);
    });
    let thread = Thread(std_handle.thread().clone());
    JoinHandle {
        std_handle,
        receiver,
        thread,
    }
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

/// A future that yields once to the async executor, then completes.
struct YieldOnce {
    yielded: bool,
}

impl std::future::Future for YieldOnce {
    type Output = ();

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        if self.yielded {
            std::task::Poll::Ready(())
        } else {
            self.yielded = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    }
}

/// Yields to the async executor, allowing other tasks to run.
///
/// On wasm32, this yields to the browser event loop. On native platforms,
/// this yields to the async executor's scheduler by returning Pending once
/// and immediately re-scheduling the task.
pub async fn yield_to_event_loop_async() {
    YieldOnce { yielded: false }.await
}

/// No-op on native - async task tracking is only needed on WASM.
///
/// On WASM, this increments a pending task counter that the worker waits for
/// before exiting. On native, threads don't have this issue.
#[inline]
pub fn task_begin() {}

/// No-op on native - async task tracking is only needed on WASM.
///
/// On WASM, this decrements the pending task counter.
#[inline]
pub fn task_finished() {}
