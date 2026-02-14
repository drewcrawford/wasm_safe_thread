// SPDX-License-Identifier: MIT OR Apache-2.0

use super::*;

/// A thread local storage key which owns its contents.
///
/// This wraps `std::thread::LocalKey` which provides thread-local storage
/// on WASM with atomics via the standard library's TLS implementation.
pub struct LocalKey<T: 'static> {
    inner: &'static std::thread::LocalKey<T>,
}

impl<T: 'static> LocalKey<T> {
    /// Creates a new `LocalKey`.
    #[doc(hidden)]
    pub const fn new(inner: &'static std::thread::LocalKey<T>) -> Self {
        LocalKey { inner }
    }

    /// Acquires a reference to the value in this TLS key.
    ///
    /// This will lazily initialize the value if this is the first time
    /// the current thread has called `with` on this key.
    ///
    /// # Panics
    ///
    /// This function will panic if the initialization function panics.
    pub fn with<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        self.inner.with(f)
    }

    /// Acquires a reference to the value in this TLS key.
    ///
    /// Returns `Err(AccessError)` if the key is being destroyed or
    /// was already destroyed.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccessError;

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "already destroyed or being destroyed")
    }
}

impl std::error::Error for AccessError {}

/// A handle to a thread.
pub struct JoinHandle<T> {
    receiver: crate::mpsc::Receiver<Result<T, String>>,
    thread: Thread,
    finished: Arc<AtomicBool>,
    /// Pointer to exit_state: [exit_code: u32, ref_count: u32] in wasm linear memory.
    /// This is managed via reference counting - worker and JoinHandle each hold a reference.
    /// Whoever decrements ref_count to 0 frees the memory.
    exit_state_ptr: u32,
}

impl<T> fmt::Debug for JoinHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinHandle")
            .field("thread", &self.thread)
            .finish_non_exhaustive()
    }
}

impl<T> JoinHandle<T> {
    /// Waits for the thread to finish and returns its result.
    ///
    /// Unfortunately, this is almost never what you want, and is likely to prevent your workers from spawning.
    /// Consider [`JoinHandle::join_async`] instead, or see the documentation on [`crate::spawn`] for details.
    pub fn join(self) -> Result<T, Box<String>> {
        if is_main_thread() {
            return Err(Box::new(
                "Can't join from the main thread on wasm32".to_string(),
            ));
        }
        self.receiver
            .recv_sync()
            .map_err(|e| Box::new(format!("{:?}", e)) as Box<String>)?
            .map_err(|e| Box::new(e) as Box<String>)
    }

    /// Waits asynchronously for the thread to finish and returns its result.
    ///
    /// This is the async version of [`JoinHandle::join`]. The error type differs
    /// from the synchronous version - panics are converted to `Box<String>` containing
    /// the debug representation of the panic payload.
    pub async fn join_async(self) -> Result<T, Box<String>> {
        // First get the return value from the channel
        // The channel sends Result<T, String> to support panic propagation
        let result = self
            .receiver
            .recv_async()
            .await
            .map_err(|e| Box::new(format!("{:?}", e)) as Box<String>)?
            .map_err(|e| Box::new(e) as Box<String>)?;

        // Then wait for the worker to actually exit
        let exit_code = wasm_bindgen_futures::JsFuture::from(wait_for_exit_async(
            &wasm_bindgen::memory(),
            self.exit_state_ptr,
        ))
        .await
        .map_err(|e| Box::new(format!("Worker exit error: {:?}", e)) as Box<String>)?;

        // Check exit code (1 = success, 2 = error)
        let code = exit_code.as_f64().unwrap_or(0.0) as u32;
        if code == 2 {
            return Err(Box::new("Worker exited with error".to_string()));
        }

        Ok(result)
        // Note: Drop impl will decrement ref_count and potentially free exit_state
    }

    /// Gets the thread associated with this handle.
    pub fn thread(&self) -> &Thread {
        &self.thread
    }

    /// Checks if the thread has finished running.
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }
}

impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        // Decrement ref_count and free if we're the last reference.
        // exit_state layout: [exit_code: u32, ref_count: u32]
        let ptr = self.exit_state_ptr as *mut [AtomicU32; 2];
        // SAFETY: ptr was created from Box::into_raw in spawn() and is valid
        // as long as ref_count > 0. We're atomically decrementing ref_count.
        let old_ref_count = unsafe { (*ptr)[1].fetch_sub(1, Ordering::AcqRel) };
        if old_ref_count == 1 {
            // We decremented from 1 to 0, we're the last reference - free the memory
            unsafe {
                drop(Box::from_raw(ptr));
            }
        }
    }
}

/// A handle to a thread.
#[derive(Clone)]
pub struct Thread {
    inner: Arc<ThreadInner>,
}

impl fmt::Debug for Thread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Thread")
            .field("id", &self.inner.id)
            .field("name", &self.inner.name)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Thread {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for Thread {}

impl std::hash::Hash for Thread {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

struct ThreadInner {
    name: Option<String>,
    id: ThreadId,
    /// Parking state: 0 = no token, 1 = unpark token present.
    /// This is a Box'd atomic so we get a stable address in wasm linear memory.
    parking_state: Box<AtomicU32>,
}

impl Thread {
    /// Gets the thread's unique identifier.
    pub fn id(&self) -> ThreadId {
        self.inner.id
    }

    /// Gets the thread's name.
    pub fn name(&self) -> Option<&str> {
        self.inner.name.as_deref()
    }

    /// Atomically makes the handle's token available if it is not already.
    pub fn unpark(&self) {
        let ptr = self.inner.parking_state.as_ref() as *const AtomicU32 as u32;
        park_notify_at_addr(&wasm_bindgen::memory(), ptr);
    }
}

/// A unique identifier for a running thread.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ThreadId(u64);

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ThreadId({})", self.0)
    }
}

/// A builder for configuring and spawning threads.
#[derive(Debug)]
pub struct Builder {
    _name: Option<String>,
    _stack_size: Option<usize>,
    _shim_name: Option<String>,
}

impl Builder {
    /// Creates a new thread builder.
    pub fn new() -> Self {
        Builder {
            _name: None,
            _stack_size: None,
            _shim_name: None,
        }
    }

    /// Set the wasm-bindgen shim name for worker spawning.
    /// This should match the binary/example name (e.g., "single_spawn", "wasm-bindgen-test").
    pub fn shim_name(mut self, name: String) -> Self {
        self._shim_name = Some(name);
        self
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

    /// Spawns a new thread with the configured settings.
    ///
    /// # Platform Notes (WASM)
    ///
    /// **The spawned thread will not begin executing until you yield to the JavaScript
    /// event loop.** On WASM targets, worker threads are spawned asynchronously via
    /// the JS event loop. When this function returns, the worker has been *created*
    /// but has not yet *started*.
    ///
    /// You must yield to the event loop to allow the worker to begin:
    ///
    /// ```
    /// let handle = wasm_safe_thread::Builder::new().spawn(|| { /* ... */ }).unwrap();
    /// # async fn ex() {
    /// wasm_safe_thread::yield_to_event_loop_async().await;  // Worker starts here, not above!
    /// # }
    /// ```
    ///
    /// Failure to yield can cause:
    /// - `is_finished()` to always return `false`
    /// - Deadlocks when waiting on atomics the worker should have set
    /// - Tests that hang indefinitely
    ///
    /// For more details on this problem and why it is hard, see implementation bugs:
    /// * <https://issues.chromium.org/issues/40633395>
    /// * <https://bugzilla.mozilla.org/show_bug.cgi?id=1888109>
    /// * <https://bugs.webkit.org/show_bug.cgi?id=271756>
    pub fn spawn<F, T>(self, f: F) -> io::Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        // Create Thread before spawning so we can pass it to the worker
        // Thread::name() only returns Some if explicitly set via Builder::name()
        let id = ThreadId(THREAD_COUNTER.fetch_add(1, Ordering::Relaxed));
        let thread = Thread {
            inner: Arc::new(ThreadInner {
                name: self._name.clone(),
                id,
                parking_state: Box::new(AtomicU32::new(0)),
            }),
        };

        // Worker name: use explicit name or generate a default for JS devtools
        let worker_name = self
            ._name
            .unwrap_or_else(|| format!("wasm_safe_thread {}", id.0));
        let thread_for_worker = thread.clone();

        let finished = Arc::new(AtomicBool::new(false));
        let finished_for_worker = finished.clone();

        // Exit state: [exit_code: u32, ref_count: u32]
        // - exit_code: 0 = running, 1 = exited successfully, 2 = exited with error
        // - ref_count: starts at 2 (worker + JoinHandle), decremented on worker exit and JoinHandle drop
        // Box'd so we get a stable address in wasm linear memory
        let exit_state = Box::new([AtomicU32::new(0), AtomicU32::new(2)]);
        let exit_state_ptr = Box::into_raw(exit_state) as u32;

        let (send, recv) = crate::mpsc::channel();
        let closure = move || {
            // Set up TLS for current() before running user code
            CURRENT_THREAD.with(|cell| {
                *cell.borrow_mut() = Some(thread_for_worker);
            });

            // Set up panic handling: store a sender closure that the panic hook can use
            // to send the error through the channel before aborting
            PANIC_SENDER.with(|cell| {
                let send_clone = send.clone();
                let finished_clone = finished_for_worker.clone();
                *cell.borrow_mut() = Some(Box::new(move |msg: String| {
                    finished_clone.store(true, Ordering::Release);
                    let _ = send_clone.send_sync(Err(msg));
                }));
            });

            // Set a panic hook that sends the error through PANIC_SENDER
            // We chain with the previous hook so that panics on other threads
            // (like the main thread) still work correctly
            let prev_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let msg = info.to_string();
                let sent = PANIC_SENDER.with(|cell| {
                    if let Some(sender) = cell.borrow_mut().take() {
                        flush_captured_prints_to_console_current_thread_impl();
                        sender(msg);
                        true
                    } else {
                        false
                    }
                });
                // If we didn't have a PANIC_SENDER (e.g., panic on main thread),
                // call the previous hook
                if !sent {
                    prev_hook(info);
                }
            }));

            crate::hooks::run_spawn_hooks();

            let result = f();

            // Clear panic sender since we completed successfully
            PANIC_SENDER.with(|cell| {
                cell.borrow_mut().take();
            });

            flush_captured_prints_to_console_current_thread_impl();

            // Mark as finished before sending result (Release pairs with Acquire in is_finished)
            finished_for_worker.store(true, Ordering::Release);
            // Ignore send errors - receiver may have been dropped if JoinHandle wasn't joined
            let _ = send.send_sync(Ok(result));
        };

        // Double-box to get a thin pointer (Box<dyn FnOnce()> is a fat pointer)
        let boxed: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(closure));
        let ptr = Box::into_raw(boxed) as *mut () as usize;
        let work: JsValue = (ptr as f64).into();

        // If shim_name not explicitly set, pass empty string to trigger auto-detection in JS
        let shim_name = self._shim_name.as_deref().unwrap_or("");

        // Spawn the worker. The WorkerHandle is not needed after spawn -
        // the worker runs independently and signals completion via exit_state.
        let _worker_handle =
            spawn_with_shared_module(work, &worker_name, shim_name, exit_state_ptr);

        Ok(JoinHandle {
            receiver: recv,
            thread,
            finished,
            exit_state_ptr,
        })
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawns a new thread, returning a [`JoinHandle`] for it.
///
/// # Warning (WASM)
///
/// **The thread will not start until you yield to the JS event loop.**
/// See [`Builder::spawn`] for details.
pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    Builder::new().spawn(f).expect("failed to spawn thread")
}

/// Gets a handle to the thread that invokes it.
pub fn current() -> Thread {
    CURRENT_THREAD.with(|cell| {
        let mut borrowed = cell.borrow_mut();
        if let Some(ref thread) = *borrowed {
            thread.clone()
        } else {
            // Lazily create a Thread for threads we didn't spawn
            let name = if is_main_thread() {
                Some("main".to_string())
            } else {
                // Thread not spawned by us (e.g., a Web Worker created externally)
                None
            };
            let id = ThreadId(THREAD_COUNTER.fetch_add(1, Ordering::Relaxed));
            let thread = Thread {
                inner: Arc::new(ThreadInner {
                    name,
                    id,
                    parking_state: Box::new(AtomicU32::new(0)),
                }),
            };
            *borrowed = Some(thread.clone());
            thread
        }
    })
}

/// Puts the current thread to sleep for at least the specified duration.
pub fn sleep(dur: Duration) {
    sleep_sync_ms(dur.as_millis() as f64);
}

/// Cooperatively gives up a timeslice to the OS scheduler.
pub fn yield_now() {
    atomics_wait_timeout_ms_try(0.001);
}

/// Yields to the browser event loop, allowing pending tasks (like worker startup) to execute.
/// This is necessary when spawning workers from within workers, as the child worker
/// won't start executing until the parent yields to the event loop.
pub async fn yield_to_event_loop_async() {
    let _: JsValue = wasm_bindgen_futures::JsFuture::from(yield_to_event_loop())
        .await
        .unwrap();
}

/// Blocks unless or until the current thread's token is made available.
pub fn park() {
    let thread = current();
    let ptr = thread.inner.parking_state.as_ref() as *const AtomicU32 as u32;
    let result = park_wait_at_addr(&wasm_bindgen::memory(), ptr);
    let result_str = result.as_string().unwrap_or_default();
    if result_str == "unsupported" {
        panic!("Atomics.wait is not available in this context (likely main thread)");
    }
}

/// Blocks unless or until the current thread's token is made available
/// or the specified duration has been reached.
pub fn park_timeout(dur: Duration) {
    let thread = current();
    let ptr = thread.inner.parking_state.as_ref() as *const AtomicU32 as u32;
    let timeout_ms = dur.as_millis() as f64;
    let result = park_wait_timeout_at_addr(&wasm_bindgen::memory(), ptr, timeout_ms);
    let result_str = result.as_string().unwrap_or_default();
    if result_str == "unsupported" {
        panic!("Atomics.wait is not available in this context (likely main thread)");
    }
}

/// Returns an estimate of the default amount of parallelism a program should use.
pub fn available_parallelism() -> io::Result<NonZeroUsize> {
    let count = get_available_parallelism();
    NonZeroUsize::new(count as usize).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "could not determine available parallelism",
        )
    })
}
